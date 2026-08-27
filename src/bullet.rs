use bevy::prelude::*;
use std::collections::{HashMap, VecDeque};

use crate::audio::SfxEvent;
use crate::map::{
    DestroyedExplodables, Explodable, ExplodableObstacleIdx, ExplodableSmolder, MapObstacles,
    ObstacleShape,
};
use crate::net::{is_authoritative, NetContext, NetEntities, NetId};
use crate::pixelart::{Canvas, Rgba};
use crate::player::{Player, PlayerDamagedEvent, PLAYER_RADIUS};
use crate::weapon::ThrowableKind;
use crate::zombie::{
    DamageNumberEvent, Zombie, ZombieKilledEvent, ZombieKind, ZOMBIE_HIT_FLASH_DURATION,
};
use crate::{gameplay_active, GameState, Score};

/// Number of past ticks of zombie position state we keep for lag
/// compensation.  6 ticks at 60 Hz = 100 ms.  Picked because that's a
/// reasonable upper bound on LAN RTT — any client lagging worse than
/// that gets the latest position (less fair, but degrades gracefully).
pub const REWIND_TICKS: usize = 6;

/// Rolling history of zombie positions, indexed `[ticks_back][NetId] → pos`.
/// `front` = newest tick, `back` = oldest.  Recorded by `record_zombie_history`
/// after `zombie_movement` each `FixedUpdate` tick.  Used by
/// `bullet_collision` to look up where a target was when a remote shooter
/// pulled the trigger.
#[derive(Resource, Default)]
pub struct RewindBuffer {
    frames: VecDeque<HashMap<u32, Vec2>>,
    /// Maps evicted from the back of `frames` are stashed here (cleared, but
    /// keeping their allocated capacity) and handed back out via `take_frame`,
    /// so the steady state allocates zero new `HashMap`s per tick.
    spare: Vec<HashMap<u32, Vec2>>,
}

impl RewindBuffer {
    /// Hand out a cleared, capacity-preserving map to fill this tick's poses.
    /// Recycles an evicted frame if one is available, else allocates once.
    pub fn take_frame(&mut self) -> HashMap<u32, Vec2> {
        match self.spare.pop() {
            Some(mut m) => {
                m.clear();
                m
            }
            None => HashMap::new(),
        }
    }
    /// Snapshot the current zombie poses; trim to `REWIND_TICKS + 1` frames,
    /// recycling the oldest map back into the spare pool.
    pub fn record(&mut self, snapshot: HashMap<u32, Vec2>) {
        self.frames.push_front(snapshot);
        while self.frames.len() > REWIND_TICKS + 1 {
            if let Some(old) = self.frames.pop_back() {
                self.spare.push(old);
            }
        }
    }
    /// Position from `ticks_back` ticks ago (saturating at the oldest entry).
    /// `None` = the zombie didn't exist back then.
    pub fn position(&self, net_id: u32, ticks_back: usize) -> Option<Vec2> {
        let idx = ticks_back.min(self.frames.len().saturating_sub(1));
        self.frames.get(idx)?.get(&net_id).copied()
    }
    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

/// Side length of one zombie spatial-grid cell, in world px.  Mirrors
/// `map_obstacles::ObstacleGrid`; 128 px keeps each cell to a handful of
/// zombies while a hit query touches only ~1–4 cells.
const ZOMBIE_GRID_CELL: f32 = 128.0;

/// Conservative broad-phase query radius for the bullet→zombie hit test.  The
/// grid bins each zombie by its **current** centre, but a lag-compensated hit
/// can be tested against a *rewound* position up to `REWIND_TICKS` old.  This
/// radius must upper-bound how far a real hit centre can sit from a candidate's
/// bin so the grid never drops a zombie the exact distance test would accept.
/// Budget: BULLET_RADIUS (3) + max zombie radius (Giant, 20) + max rewind
/// displacement (235 px/s × 6/60 ≈ 24) ≈ 47, rounded up to 96 for margin.
/// Being generous only widens the candidate set slightly; it can never cause a
/// missed hit — the exact per-zombie distance test still decides every hit.
const ZOMBIE_HIT_QUERY_RADIUS: f32 = 96.0;

/// Spatial index over live zombies, rebuilt once per tick by
/// `record_zombie_history` (same pass that snapshots the rewind buffer, so no
/// extra full iteration).  Lets `bullet_collision` test each bullet against
/// only nearby zombies instead of scanning all of them — turning the hot
/// O(bullets × zombies) loop into O(bullets × local).  Each zombie is binned
/// in exactly one cell (its centre), so candidates are yielded without dupes.
#[derive(Resource, Default)]
pub struct ZombieGrid {
    cells: Vec<Vec<Entity>>,
    cols: usize,
    rows: usize,
}

impl ZombieGrid {
    fn dims() -> (usize, usize) {
        let cols = ((crate::map::MAP_WIDTH / ZOMBIE_GRID_CELL).ceil() as usize) + 1;
        let height = crate::map::MAP_HEIGHT + crate::map_obstacles::UNDERGROUND_EXTENT_Y;
        let rows = ((height / ZOMBIE_GRID_CELL).ceil() as usize) + 1;
        (cols, rows)
    }

    #[inline]
    fn world_to_cell(p: Vec2) -> (i32, i32) {
        // Same coordinate transform as ObstacleGrid so underground (negative-Y)
        // zombies land at non-negative cell indices.
        let cx = ((p.x + crate::map::MAP_WIDTH * 0.5) / ZOMBIE_GRID_CELL).floor() as i32;
        let cy = ((p.y - crate::map_obstacles::world_min_y()) / ZOMBIE_GRID_CELL).floor() as i32;
        (cx, cy)
    }

    /// Clear and resize the grid for a fresh tick, preserving cell capacity.
    fn begin(&mut self) {
        let (cols, rows) = Self::dims();
        self.cols = cols;
        self.rows = rows;
        let total = cols * rows;
        if self.cells.len() != total {
            self.cells.clear();
            self.cells.resize_with(total, Vec::new);
        } else {
            for cell in &mut self.cells {
                cell.clear();
            }
        }
    }

    #[inline]
    fn insert(&mut self, e: Entity, p: Vec2) {
        let (cx, cy) = Self::world_to_cell(p);
        if cx < 0 || cy < 0 || cx >= self.cols as i32 || cy >= self.rows as i32 {
            return;
        }
        self.cells[cy as usize * self.cols + cx as usize].push(e);
    }

    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            cell.clear();
        }
    }

    /// Push every zombie entity whose bin lies within `radius` of `center` into
    /// `out`.  Conservative superset — the caller must still run the exact
    /// distance test.  Each entity appears at most once (single-cell binning).
    fn collect_candidates(&self, center: Vec2, radius: f32, out: &mut Vec<Entity>) {
        if self.cells.is_empty() {
            return;
        }
        let lo = Vec2::new(center.x - radius, center.y - radius);
        let hi = Vec2::new(center.x + radius, center.y + radius);
        let (c0, r0) = Self::world_to_cell(lo);
        let (c1, r1) = Self::world_to_cell(hi);
        // Fully out of bounds → no candidates (avoids the degenerate
        // clamp-to-zero that would scan cell 0).
        if c1 < 0 || r1 < 0 || c0 >= self.cols as i32 || r0 >= self.rows as i32 {
            return;
        }
        let cs = c0.max(0) as usize;
        let ce = (c1.min(self.cols as i32 - 1)).max(0) as usize;
        let rs = r0.max(0) as usize;
        let re = (r1.min(self.rows as i32 - 1)).max(0) as usize;
        for r in rs..=re {
            let row_off = r * self.cols;
            for c in cs..=ce {
                out.extend_from_slice(&self.cells[row_off + c]);
            }
        }
    }
}

const BULLET_SPRITE_SIZE: Vec2 = Vec2::new(14.0, 6.0);
const ROCKET_SPRITE_SIZE: Vec2 = Vec2::new(26.0, 12.0);

pub const BULLET_RADIUS: f32 = 3.0;
pub const BULLET_LIFETIME: f32 = 1.4;
pub const ROCKET_LIFETIME: f32 = 2.2;
pub const FLAME_LIFETIME: f32 = 0.42;
pub const EXPLOSION_LIFETIME: f32 = 0.38;

pub const ROCKET_EXPLOSION_RADIUS: f32 = 95.0;
pub const ROCKET_EXPLOSION_ZOMBIE_DAMAGE: i32 = 12;
pub const ROCKET_EXPLOSION_PLAYER_DAMAGE: i32 = 30;

pub const EXPLODER_EXPLOSION_RADIUS: f32 = 72.0;
pub const EXPLODER_EXPLOSION_ZOMBIE_DAMAGE: i32 = 5;
pub const EXPLODER_EXPLOSION_PLAYER_DAMAGE: i32 = 40;

#[derive(Component)]
pub struct Bullet {
    pub velocity: Vec2,
    pub lifetime: f32,
    pub damage: i32,
    pub is_rocket: bool,
    /// If true, the bullet steers toward `target` while it has line-of-sight.
    pub homing: bool,
    pub target: Option<Entity>,
    /// True for flamethrower puffs.  Renders as a flame sprite that grows
    /// and fades over its short lifetime.  Damage still applies.
    pub is_flame: bool,
    /// Original lifetime, kept for the flame fade-out alpha curve.
    pub initial_lifetime: f32,
    /// Player id of whoever fired the round.  Used by the host's
    /// lag-compensated hit test: bullets from a non-local shooter are
    /// rewound against the rolling zombie-position history so the shot lands
    /// where the lagged client *saw* the target.
    pub shooter_id: u8,
}

#[derive(Component)]
pub struct Explosion {
    pub lifetime: f32,
    pub radius: f32,
}

#[derive(Event)]
pub struct ShootEvent {
    pub origin: Vec2,
    pub direction: Vec2,
    pub damage: i32,
    pub speed: f32,
    pub is_rocket: bool,
    pub homing: bool,
    pub is_flame: bool,
    /// Player id of the shooter.  Carried into the spawned `Bullet`
    /// component so `bullet_collision` can lag-compensate.
    pub shooter_id: u8,
}

#[derive(Event, Clone, Copy)]
pub struct ExplodeEvent {
    pub pos: Vec2,
    pub radius: f32,
    pub zombie_damage: i32,
    pub player_damage: i32,
}

#[derive(Event)]
pub struct ThrowEvent {
    pub origin: Vec2,
    pub direction: Vec2,
    pub kind: ThrowableKind,
}

#[derive(Component)]
pub struct ThrownProjectile {
    pub velocity: Vec2,
    pub fuse: f32,
    pub kind: ThrowableKind,
}

#[derive(Component)]
pub struct SmokeCloud {
    pub lifetime: f32,
    pub radius: f32,
}

/// Timed movement debuff applied by smoke clouds.  Holds the zombie's
/// original (per-spawn jittered) speed so `expire_smoke_slow` can restore
/// it exactly when the debuff runs out; `remaining` is refreshed every tick
/// the zombie stays inside a cloud, so full speed comes back shortly after
/// it walks out or the cloud fades.
#[derive(Component)]
pub struct SmokeSlowed {
    pub base_speed: f32,
    pub remaining: f32,
}

#[derive(Component)]
pub struct FirePool {
    pub lifetime: f32,
    pub radius: f32,
    pub tick_timer: f32,
}

pub const GRENADE_EXPLOSION_RADIUS: f32 = 85.0;
pub const GRENADE_ZOMBIE_DAMAGE: i32 = 15;
pub const GRENADE_PLAYER_DAMAGE: i32 = 25;
pub const SMOKE_RADIUS: f32 = 90.0;
pub const SMOKE_DURATION: f32 = 5.0;
pub const SMOKE_SLOW_FACTOR: f32 = 0.3;
/// How long the smoke slow lingers after the last tick spent inside a
/// cloud.  Comfortably longer than one 60 Hz tick so the debuff never
/// flickers off while the zombie is still standing in the smoke.
pub const SMOKE_SLOW_LINGER: f32 = 0.25;
pub const FIRE_RADIUS: f32 = 65.0;
pub const FIRE_DURATION: f32 = 4.0;
pub const FIRE_DAMAGE: i32 = 3;
pub const FIRE_TICK: f32 = 0.4;
pub const THROW_RANGE: f32 = 300.0;

#[derive(Component)]
pub struct ThrowIndicator;

/// Cosmetic muzzle flash spawned at the gun tip on every shot.  Fades out
/// over `max_lifetime` seconds.
#[derive(Component)]
pub struct MuzzleFlash {
    pub lifetime: f32,
    pub max_lifetime: f32,
}

/// Tiny impact spark thrown off when a bullet hits a wall — a small
/// translating, fading sprite with no gameplay effect.
#[derive(Component)]
pub struct ImpactSpark {
    pub velocity: Vec2,
    pub lifetime: f32,
    pub max_lifetime: f32,
}

/// Expanding ring drawn over the explosion sprite for extra punch.  Grows
/// from 0 to `max_radius` while alpha fades.
#[derive(Component)]
pub struct ShockwaveRing {
    pub lifetime: f32,
    pub max_lifetime: f32,
    pub max_radius: f32,
}

/// Smoke puff drifting upward — used for the smoke columns that linger over
/// destroyed wrecks and barrels.  Grows + fades over its lifetime.
#[derive(Component)]
pub struct SmokePuff {
    pub lifetime: f32,
    pub max_lifetime: f32,
    pub velocity: Vec2,
    pub start_size: f32,
    pub end_size: f32,
}

/// Short bullet tracer streak.  Fades in ~0.18 s — purely cosmetic, drawn
/// over the bullet path so the firing line reads even at high RPM.
#[derive(Component)]
pub struct BulletTracer {
    pub lifetime: f32,
    pub max_lifetime: f32,
}

/// Spent brass casing ejected on every shot.  Flies sideways with spin,
/// decelerates, then sits on the ground as a faint decal for ~5 s.
#[derive(Component)]
pub struct BulletShell {
    pub velocity: Vec2,
    pub spin: f32,
    pub lifetime: f32,
    pub max_lifetime: f32,
}

/// Ambient ember / drifting ash particle that rises slowly through the
/// world to sell the post-apocalyptic mood.  Spawned periodically around
/// the camera; despawned on lifetime expiry.
#[derive(Component)]
pub struct AmbientEmber {
    pub velocity: Vec2,
    pub lifetime: f32,
    pub max_lifetime: f32,
    pub flicker: f32,
}

#[derive(Resource)]
pub struct BulletAssets {
    pub bullet: Handle<Image>,
    pub rocket: Handle<Image>,
    pub explosion: Handle<Image>,
    pub thrown: Handle<Image>,
    pub muzzle_flash: Handle<Image>,
    pub spark: Handle<Image>,
    pub shockwave: Handle<Image>,
    pub flame: Handle<Image>,
    pub smoke: Handle<Image>,
    pub tracer: Handle<Image>,
    pub shell: Handle<Image>,
    pub ember: Handle<Image>,
}

/// Pre-baked Mesh + ColorMaterial handles for thrown-projectile FX (smoke
/// cloud + fire pool).  Without these, every grenade/molotov throw allocates
/// a fresh `Circle` mesh and a fresh `ColorMaterial` — both leak into the
/// asset GC roots until the cloud despawns, then have to be re-allocated for
/// the next throw.  Building once at startup turns spawn into a cheap handle
/// clone.
#[derive(Resource)]
pub struct ThrowableFxAssets {
    pub smoke_mesh: bevy::sprite::Mesh2dHandle,
    pub smoke_material: Handle<ColorMaterial>,
    pub fire_mesh: bevy::sprite::Mesh2dHandle,
    pub fire_material: Handle<ColorMaterial>,
}

pub struct BulletPlugin;

impl Plugin for BulletPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<ShootEvent>()
            .add_event::<ExplodeEvent>()
            .add_event::<ThrowEvent>()
            .init_resource::<RewindBuffer>()
            .init_resource::<ZombieGrid>()
            .add_systems(Startup, setup_bullet_assets)
            .add_systems(OnExit(GameState::Playing), despawn_all_bullets)
            .add_systems(OnExit(GameState::Playing), reset_rewind_buffer)
            .add_systems(
                Update,
                (
                    update_throw_indicator,
                    update_muzzle_flashes,
                    update_impact_sparks,
                    update_shockwaves,
                    update_smoke_puffs,
                    update_bullet_tracers,
                    update_bullet_shells,
                    spawn_ambient_embers,
                    update_ambient_embers,
                    animate_explodable_smolder,
                )
                    // Cosmetic FX — freeze on a singleplayer pause (no spawn/anim
                    // churn while paused); stays live in multiplayer.
                    .run_if(gameplay_active),
            )
            .add_systems(
                FixedUpdate,
                (
                    record_zombie_history,
                    shoot_listener,
                    throw_listener,
                    bullet_movement,
                    thrown_projectile_update,
                    bullet_collision,
                    explode_listener,
                    explosion_lifetime,
                    smoke_cloud_update,
                    expire_smoke_slow,
                    fire_pool_update,
                )
                    .chain()
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

fn setup_bullet_assets(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let indicator_img = images.add(build_indicator_image());
    commands.insert_resource(BulletAssets {
        bullet: images.add(build_bullet_image()),
        rocket: images.add(build_rocket_image()),
        explosion: images.add(build_explosion_image()),
        thrown: images.add(build_thrown_image()),
        muzzle_flash: images.add(build_muzzle_flash_image()),
        spark: images.add(build_spark_image()),
        shockwave: images.add(build_shockwave_image()),
        flame: images.add(build_flame_puff_image()),
        smoke: images.add(build_smoke_puff_image()),
        tracer: images.add(build_tracer_image()),
        shell: images.add(build_shell_image()),
        ember: images.add(build_ember_image()),
    });
    // Bake the smoke/fire mesh + material once and stash the handles —
    // every throw later just clones the handles instead of allocating.
    commands.insert_resource(ThrowableFxAssets {
        smoke_mesh: bevy::sprite::Mesh2dHandle(meshes.add(Circle::new(SMOKE_RADIUS))),
        smoke_material: materials.add(Color::srgba(0.7, 0.72, 0.75, 0.35)),
        fire_mesh: bevy::sprite::Mesh2dHandle(meshes.add(Circle::new(FIRE_RADIUS))),
        fire_material: materials.add(Color::srgba(0.9, 0.35, 0.05, 0.4)),
    });
    commands.spawn((
        SpriteBundle {
            texture: indicator_img,
            sprite: Sprite {
                custom_size: Some(Vec2::splat(40.0)),
                ..default()
            },
            transform: Transform::from_xyz(0.0, 0.0, 7.0),
            visibility: Visibility::Hidden,
            ..default()
        },
        ThrowIndicator,
    ));
}

fn build_thrown_image() -> Image {
    let body: Rgba = [80, 90, 60, 255];
    let hi: Rgba = [120, 130, 90, 255];
    let outline: Rgba = [20, 24, 16, 255];
    let mut c = Canvas::new(7, 7);
    c.fill_circle(3, 3, 3, outline);
    c.fill_circle(3, 3, 2, body);
    c.put(2, 2, hi);
    c.into_image()
}

fn build_indicator_image() -> Image {
    let ring: Rgba = [255, 180, 40, 200];
    let fill: Rgba = [255, 200, 60, 40];
    let mut c = Canvas::new(32, 32);
    c.fill_circle(16, 16, 15, ring);
    c.fill_circle(16, 16, 13, fill);
    c.fill_circle(16, 16, 2, ring);
    c.into_image()
}

fn build_bullet_image() -> Image {
    let trail: Rgba = [220, 80, 20, 180];
    let trail_dim: Rgba = [140, 50, 10, 110];
    let glow: Rgba = [255, 180, 40, 255];
    let core: Rgba = [255, 245, 160, 255];
    let tip: Rgba = [255, 255, 220, 255];

    let mut c = Canvas::new(11, 5);

    c.put(0, 2, trail_dim);
    c.put(1, 2, trail_dim);
    c.put(2, 2, trail);
    c.put(3, 2, trail);
    c.put(1, 1, trail_dim);
    c.put(2, 1, trail_dim);
    c.put(1, 3, trail_dim);
    c.put(2, 3, trail_dim);

    c.put(4, 2, glow);
    c.put(5, 2, glow);
    c.put(3, 1, trail);
    c.put(3, 3, trail);
    c.put(4, 1, glow);
    c.put(4, 3, glow);

    c.put(6, 2, core);
    c.put(7, 2, core);
    c.put(8, 2, core);
    c.put(5, 1, glow);
    c.put(5, 3, glow);
    c.put(6, 1, glow);
    c.put(6, 3, glow);

    c.put(9, 2, tip);
    c.put(10, 2, tip);

    c.into_image()
}

fn build_rocket_image() -> Image {
    let outline: Rgba = [10, 10, 12, 255];
    let body: Rgba = [180, 180, 186, 255];
    let body_light: Rgba = [220, 222, 230, 255];
    let body_dark: Rgba = [100, 100, 108, 255];
    let stripe: Rgba = [220, 40, 30, 255];
    let warhead: Rgba = [60, 60, 64, 255];
    let fin: Rgba = [140, 30, 25, 255];
    let flame_core: Rgba = [255, 240, 160, 255];
    let flame_mid: Rgba = [255, 170, 40, 255];
    let flame_edge: Rgba = [220, 60, 20, 220];

    let mut c = Canvas::new(20, 9);

    c.put(0, 4, flame_edge);
    c.put(1, 4, flame_edge);
    c.put(0, 3, flame_edge);
    c.put(0, 5, flame_edge);
    c.put(2, 4, flame_mid);
    c.put(2, 3, flame_mid);
    c.put(2, 5, flame_mid);
    c.put(3, 4, flame_mid);
    c.put(3, 3, flame_core);
    c.put(3, 5, flame_core);
    c.put(4, 4, flame_core);

    c.fill_rect(5, 3, 10, 3, outline);
    c.fill_rect(6, 3, 9, 1, body_light);
    c.fill_rect(6, 4, 9, 1, body);
    c.fill_rect(6, 5, 9, 1, body_dark);

    c.put(9, 4, stripe);
    c.put(10, 4, stripe);
    c.put(11, 4, stripe);

    c.fill_rect(15, 3, 3, 3, outline);
    c.put(15, 4, warhead);
    c.put(16, 4, warhead);
    c.put(17, 4, outline);

    c.put(5, 2, outline);
    c.put(6, 2, fin);
    c.put(5, 6, outline);
    c.put(6, 6, fin);
    c.put(7, 1, outline);
    c.put(7, 7, outline);

    c.into_image()
}

fn build_explosion_image() -> Image {
    let ring: Rgba = [255, 70, 20, 240];
    let outer: Rgba = [255, 130, 30, 255];
    let mid: Rgba = [255, 190, 60, 255];
    let hot: Rgba = [255, 230, 140, 255];
    let core: Rgba = [255, 255, 235, 255];
    let spark: Rgba = [255, 240, 180, 255];

    let mut c = Canvas::new(33, 33);

    c.fill_circle(16, 16, 16, ring);
    c.fill_circle(16, 16, 14, outer);
    c.fill_circle(16, 16, 11, mid);
    c.fill_circle(16, 16, 8, hot);
    c.fill_circle(16, 16, 4, core);

    c.put(2, 16, spark);
    c.put(30, 16, spark);
    c.put(16, 2, spark);
    c.put(16, 30, spark);
    c.put(5, 5, spark);
    c.put(27, 27, spark);
    c.put(5, 27, spark);
    c.put(27, 5, spark);

    c.into_image()
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_bullet_entity(
    commands: &mut Commands,
    assets: &BulletAssets,
    origin: Vec2,
    direction: Vec2,
    speed: f32,
    damage: i32,
    net_id: u32,
    is_rocket: bool,
    homing: bool,
    target: Option<Entity>,
    is_flame: bool,
    shooter_id: u8,
) -> Entity {
    let angle = direction.y.atan2(direction.x);
    let (texture, size, lifetime) = if is_flame {
        (
            assets.flame.clone(),
            Vec2::new(28.0, 22.0),
            FLAME_LIFETIME,
        )
    } else if is_rocket {
        (assets.rocket.clone(), ROCKET_SPRITE_SIZE, ROCKET_LIFETIME)
    } else {
        (assets.bullet.clone(), BULLET_SPRITE_SIZE, BULLET_LIFETIME)
    };
    let color = if is_flame {
        // Slight per-puff color jitter so the cone reads as living flame
        // rather than copies of the same sprite.  Pure-orange biased.
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let r = 1.0;
        let g = rng.gen_range(0.55..0.85);
        let b = rng.gen_range(0.10..0.25);
        Color::srgba(r, g, b, 1.0)
    } else {
        Color::WHITE
    };
    commands
        .spawn((
            SpriteBundle {
                texture,
                sprite: Sprite {
                    custom_size: Some(size),
                    color,
                    ..default()
                },
                transform: Transform::from_xyz(origin.x, origin.y, 8.0)
                    .with_rotation(Quat::from_rotation_z(angle)),
                ..default()
            },
            Bullet {
                velocity: direction * speed,
                lifetime,
                damage,
                is_rocket,
                homing,
                target,
                is_flame,
                initial_lifetime: lifetime,
                shooter_id,
            },
            NetId(net_id),
        ))
        .id()
}

pub fn spawn_explosion_entity(
    commands: &mut Commands,
    assets: &BulletAssets,
    pos: Vec2,
    radius: f32,
    net_id: u32,
) -> Entity {
    commands
        .spawn((
            SpriteBundle {
                texture: assets.explosion.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(radius * 1.2)),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 9.5),
                ..default()
            },
            Explosion {
                lifetime: EXPLOSION_LIFETIME,
                radius,
            },
            NetId(net_id),
        ))
        .id()
}

fn shoot_listener(
    mut commands: Commands,
    mut events: EventReader<ShootEvent>,
    assets: Res<BulletAssets>,
    mut ctx: ResMut<NetContext>,
    zombies: Query<(Entity, &Transform), With<Zombie>>,
    // Shooters that already got their once-per-trigger-pull cosmetics this
    // tick.  Multi-pellet weapons emit one ShootEvent per pellet in a single
    // tick; without the dedupe a SawedOff pull stacks 8 identical muzzle
    // flashes at one origin (pure overdraw) and ejects 8 brass shells.
    mut flashed: Local<Vec<u8>>,
) {
    flashed.clear();
    for ev in events.read() {
        let net_id = ctx.alloc_bullet_id();
        // For homing rockets, lock the closest zombie at fire time so the
        // missile commits to a single target.  We pick the nearest in the
        // forward 120° cone, falling back to the absolute nearest if no
        // zombie sits in front.
        let target = if ev.homing {
            find_homing_target(&zombies, ev.origin, ev.direction)
        } else {
            None
        };
        spawn_bullet_entity(
            &mut commands,
            &assets,
            ev.origin,
            ev.direction,
            ev.speed,
            ev.damage,
            net_id,
            ev.is_rocket,
            ev.homing,
            target,
            ev.is_flame,
            ev.shooter_id,
        );
        // First event this shooter fired this tick == first pellet of the
        // trigger pull.  Tracers stay per-pellet — they fan out with spread
        // and are what makes the pellet cone readable — but the shell and
        // muzzle flash spawn once per pull.
        let first_pellet = !flashed.contains(&ev.shooter_id);
        if first_pellet {
            flashed.push(ev.shooter_id);
        }

        // Cheap glowing tracer streak from origin in the direction of fire.
        // Skipped for rockets and flames (they have their own bigger
        // visuals).
        if !ev.is_rocket && !ev.is_flame {
            spawn_bullet_tracer(&mut commands, &assets, ev.origin, ev.direction);
            if first_pellet {
                spawn_bullet_shell(&mut commands, &assets, ev.origin, ev.direction);
            }
        }

        // Cosmetic muzzle flash at the gun tip.  Skip for flamethrower
        // since the flame puffs themselves are the visual.  Bigger for
        // rockets so the launch reads correctly even from a distance.
        if ev.is_flame || !first_pellet {
            continue;
        }
        let angle = ev.direction.y.atan2(ev.direction.x);
        let flash_size = if ev.is_rocket {
            Vec2::new(40.0, 30.0)
        } else {
            Vec2::new(28.0, 20.0)
        };
        commands.spawn((
            SpriteBundle {
                texture: assets.muzzle_flash.clone(),
                sprite: Sprite {
                    custom_size: Some(flash_size),
                    color: Color::srgba(1.0, 0.95, 0.55, 1.0),
                    ..default()
                },
                transform: Transform::from_xyz(ev.origin.x, ev.origin.y, 9.4)
                    .with_rotation(Quat::from_rotation_z(angle)),
                ..default()
            },
            MuzzleFlash {
                lifetime: 0.07,
                max_lifetime: 0.07,
            },
        ));
    }
}

/// Pick the nearest live zombie roughly in front of the launcher.  Cosine
/// over 0.5 ≈ within 60° on either side of `aim`.  If nothing sits in the
/// cone, fall back to the nearest zombie anywhere — better to track
/// something than fly straight past a wall of enemies.
fn find_homing_target(
    zombies: &Query<(Entity, &Transform), With<Zombie>>,
    origin: Vec2,
    aim: Vec2,
) -> Option<Entity> {
    let mut best_in_cone: Option<(Entity, f32)> = None;
    let mut best_any: Option<(Entity, f32)> = None;
    let aim = if aim.length_squared() > 0.0 { aim.normalize() } else { Vec2::X };
    const MAX_RANGE_SQ: f32 = 1400.0 * 1400.0;
    for (ent, t) in zombies.iter() {
        let p = t.translation.truncate();
        let delta = p - origin;
        let dist_sq = delta.length_squared();
        if !(1.0..=MAX_RANGE_SQ).contains(&dist_sq) {
            continue;
        }
        if best_any.map(|(_, d)| dist_sq < d).unwrap_or(true) {
            best_any = Some((ent, dist_sq));
        }
        let dir = delta / dist_sq.sqrt();
        if dir.dot(aim) >= 0.5
            && best_in_cone.map(|(_, d)| dist_sq < d).unwrap_or(true)
        {
            best_in_cone = Some((ent, dist_sq));
        }
    }
    best_in_cone.or(best_any).map(|(e, _)| e)
}

#[allow(clippy::too_many_arguments)]
fn bullet_movement(
    mut commands: Commands,
    time: Res<Time>,
    obstacles: Res<MapObstacles>,
    assets: Res<BulletAssets>,
    targets: Query<&Transform, (With<Zombie>, Without<Bullet>)>,
    explodables: Query<(&Transform, &Explodable), Without<Bullet>>,
    mut q: Query<(Entity, &mut Transform, &mut Bullet, &mut Sprite)>,
    mut explode: EventWriter<ExplodeEvent>,
) {
    let dt = time.delta_seconds();
    const HOMING_TURN_RATE: f32 = 6.0; // radians/sec
    for (entity, mut transform, mut bullet, mut sprite) in &mut q {
        // Homing steering — lerp velocity direction toward the target each
        // tick.  Speed is preserved; only the heading changes.  If the
        // target died (entity gone), fall through to straight-line flight.
        if bullet.homing {
            if let Some(target_ent) = bullet.target {
                if let Ok(target_t) = targets.get(target_ent) {
                    let pos = transform.translation.truncate();
                    let to_target = target_t.translation.truncate() - pos;
                    if to_target.length_squared() > 1.0 {
                        let speed = bullet.velocity.length().max(1.0);
                        let want = to_target.normalize();
                        let cur = bullet.velocity / speed;
                        // Quat slerp would work, but for 2D it's cheaper to
                        // rotate `cur` toward `want` by a clamped angle.
                        let cur_a = cur.y.atan2(cur.x);
                        let want_a = want.y.atan2(want.x);
                        let mut delta = want_a - cur_a;
                        while delta > std::f32::consts::PI {
                            delta -= std::f32::consts::TAU;
                        }
                        while delta < -std::f32::consts::PI {
                            delta += std::f32::consts::TAU;
                        }
                        let max_step = HOMING_TURN_RATE * dt;
                        let step = delta.clamp(-max_step, max_step);
                        let new_a = cur_a + step;
                        let new_dir = Vec2::new(new_a.cos(), new_a.sin());
                        bullet.velocity = new_dir * speed;
                        transform.rotation = Quat::from_rotation_z(new_a);
                    }
                } else {
                    // Target gone — drop the lock and fly straight.
                    bullet.target = None;
                }
            }
        }
        transform.translation += (bullet.velocity * dt).extend(0.0);
        bullet.lifetime -= dt;

        // Flame puffs grow + fade as they travel — flamethrower visual.
        // Also slow them down so they "drift" near the end of their
        // lifetime instead of zipping in a straight line.
        if bullet.is_flame {
            let pct = (bullet.lifetime / bullet.initial_lifetime).clamp(0.0, 1.0);
            let progress = 1.0 - pct;
            let scale = 0.6 + progress * 1.6; // 0.6 → 2.2x size
            sprite.custom_size = Some(Vec2::new(28.0 * scale, 22.0 * scale));
            // Quadratic alpha falloff so the tail of the puff disappears
            // smoothly instead of clipping out.
            sprite.color.set_alpha(pct.powf(1.4));
            // Drag — flames slow down as they spread.
            bullet.velocity *= 1.0 - 1.6 * dt;
        }

        let pos = transform.translation.truncate();
        let hit_obstacle = obstacles.hits(pos, BULLET_RADIUS);
        // Explodables are registered in `MapObstacles` so movement still
        // routes around them — but a bullet hitting one shouldn't be eaten
        // silently by the obstacle check.  If the bullet is overlapping a
        // live explodable's rect, skip the despawn here and let
        // `bullet_collision` apply damage on this same tick.
        let on_explodable = hit_obstacle
            && explodables.iter().any(|(t, expl)| {
                if expl.hp <= 0 {
                    return false;
                }
                let ep = t.translation.truncate();
                let half = expl.kind.collision_half();
                (pos.x - ep.x).abs() < half.x + BULLET_RADIUS + 2.0
                    && (pos.y - ep.y).abs() < half.y + BULLET_RADIUS + 2.0
            });
        if on_explodable {
            continue;
        }
        if bullet.lifetime <= 0.0 || hit_obstacle {
            if bullet.is_rocket {
                explode.send(ExplodeEvent {
                    pos,
                    radius: ROCKET_EXPLOSION_RADIUS,
                    zombie_damage: ROCKET_EXPLOSION_ZOMBIE_DAMAGE,
                    player_damage: ROCKET_EXPLOSION_PLAYER_DAMAGE,
                });
            } else if hit_obstacle && !bullet.is_flame {
                // Spawn 3 small impact sparks bouncing off the wall in
                // random directions — non-rocket bullets only, so rocket
                // explosions don't compete with the sparks visually.
                // Flame puffs already have their own dissipation visual,
                // so they skip the sparks.
                spawn_impact_sparks(&mut commands, &assets, pos, bullet.velocity);
            }
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn spawn_impact_sparks(
    commands: &mut Commands,
    assets: &BulletAssets,
    pos: Vec2,
    incoming_velocity: Vec2,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    // Reflect-ish: sparks fly back generally opposite the bullet, with a
    // wide cone of randomisation so the burst feels fizzy, not directional.
    let base = if incoming_velocity.length_squared() > 0.0001 {
        -incoming_velocity.normalize()
    } else {
        Vec2::Y
    };
    for _ in 0..4 {
        let angle: f32 = rng.gen_range(-1.4..1.4);
        let (sin, cos) = angle.sin_cos();
        let dir = Vec2::new(base.x * cos - base.y * sin, base.x * sin + base.y * cos);
        let speed = rng.gen_range(120.0..240.0);
        let life = rng.gen_range(0.18..0.32);
        commands.spawn((
            SpriteBundle {
                texture: assets.spark.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(rng.gen_range(4.0..7.0))),
                    color: Color::srgba(1.0, 0.95, 0.55, 1.0),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 8.5),
                ..default()
            },
            ImpactSpark {
                velocity: dir * speed,
                lifetime: life,
                max_lifetime: life,
            },
        ));
    }
}

/// Snapshots the live zombie poses into `RewindBuffer` **and** rebuilds the
/// `ZombieGrid` spatial index in the same pass.  Runs at the top of the
/// FixedUpdate chain so that `bullet_collision` later in the same tick sees
/// "current" history and an up-to-date grid.  One iteration over all zombies
/// feeds both; the rewind map is recycled so the steady state allocates
/// nothing.
fn record_zombie_history(
    mut rewind: ResMut<RewindBuffer>,
    mut grid: ResMut<ZombieGrid>,
    zombies: Query<(Entity, &Transform, &NetId), With<Zombie>>,
) {
    let mut frame = rewind.take_frame();
    grid.begin();
    for (e, t, id) in &zombies {
        let p = t.translation.truncate();
        frame.insert(id.0, p);
        grid.insert(e, p);
    }
    rewind.record(frame);
}

fn reset_rewind_buffer(mut rewind: ResMut<RewindBuffer>, mut grid: ResMut<ZombieGrid>) {
    rewind.clear();
    grid.clear();
}

/// Minimum FixedUpdate ticks between two `SfxEvent::Hit` blips.  A shotgun
/// blast consumes up to 8 pellets in a single tick and sustained flame fire
/// lands ~80 hits/s; playing every one stacks identical tones into a volume
/// spike and allocates an audio entity + `Pitch` asset per event.  Two ticks
/// (~33 ms) keeps hits sounding continuous while capping the churn.
const HIT_SFX_INTERVAL_TICKS: u8 = 2;

#[allow(clippy::too_many_arguments)]
fn bullet_collision(
    mut commands: Commands,
    assets: Res<BulletAssets>,
    rewind: Res<RewindBuffer>,
    grid: Res<ZombieGrid>,
    ctx: Res<NetContext>,
    bullets: Query<(Entity, &Transform, &Bullet)>,
    mut zombies: Query<(Entity, &Transform, &mut Zombie, &NetId)>,
    // Reused scratch for grid candidates — zero steady-state allocation.
    mut z_candidates: Local<Vec<Entity>>,
    mut explodables: Query<(Entity, &Transform, &mut Explodable, &ExplodableObstacleIdx)>,
    mut obstacles: ResMut<MapObstacles>,
    mut destroyed: ResMut<DestroyedExplodables>,
    players: Query<&Player>,
    // Bundled into one tuple param to stay within Bevy's 16-param system limit.
    (mut killed, mut explode, mut dmg_numbers, mut sfx, mut hit_sfx_cooldown): (
        EventWriter<ZombieKilledEvent>,
        EventWriter<ExplodeEvent>,
        EventWriter<DamageNumberEvent>,
        EventWriter<SfxEvent>,
        // Ticks left before the next Hit blip — see HIT_SFX_INTERVAL_TICKS.
        Local<u8>,
    ),
    mut score: ResMut<Score>,
) {
    if *hit_sfx_cooldown > 0 {
        *hit_sfx_cooldown -= 1;
    }
    let mult = max_money_mult(players.iter());
    let host_local_id = ctx.my_id;
    for (b_entity, b_transform, bullet) in &bullets {
        let bp = b_transform.translation.truncate();
        let mut consumed = false;

        // Lag compensation: bullets fired by remote players hit-test against
        // the rewound zombie position (~100 ms back), so the shot lands where
        // the lagged client's screen showed the target.  Bullets from the
        // host's local player or with no shooter id (single-player) skip the
        // rewind — they already see real-time positions.
        let rewind_for_this_bullet =
            bullet.shooter_id != 0 && bullet.shooter_id != host_local_id;

        // Zombies first — they're the primary target.  Broad-phase via the
        // spatial grid: only zombies binned near the bullet are candidates.
        // The query radius is a conservative superset (covers lag-rewind
        // displacement + max body radius), so the exact distance test below
        // still decides every hit byte-for-byte as before.
        z_candidates.clear();
        grid.collect_candidates(bp, ZOMBIE_HIT_QUERY_RADIUS, &mut z_candidates);
        for &z_entity in z_candidates.iter() {
            let Ok((_, z_transform, mut zombie, z_net)) = zombies.get_mut(z_entity) else {
                continue;
            };
            if zombie.hp <= 0 {
                continue;
            }
            let zp = if rewind_for_this_bullet {
                rewind
                    .position(z_net.0, REWIND_TICKS)
                    .unwrap_or_else(|| z_transform.translation.truncate())
            } else {
                z_transform.translation.truncate()
            };
            let r = BULLET_RADIUS + zombie.kind.radius();
            if bp.distance_squared(zp) < r * r {
                if bullet.is_rocket {
                    explode.send(ExplodeEvent {
                        pos: bp,
                        radius: ROCKET_EXPLOSION_RADIUS,
                        zombie_damage: ROCKET_EXPLOSION_ZOMBIE_DAMAGE,
                        player_damage: ROCKET_EXPLOSION_PLAYER_DAMAGE,
                    });
                    commands.entity(b_entity).despawn_recursive();
                    consumed = true;
                    break;
                }
                zombie.hp -= bullet.damage;
                zombie.hit_flash = ZOMBIE_HIT_FLASH_DURATION;
                dmg_numbers.send(DamageNumberEvent {
                    pos: zp,
                    amount: bullet.damage,
                });
                commands.entity(b_entity).despawn_recursive();
                if *hit_sfx_cooldown == 0 {
                    sfx.send(SfxEvent::Hit);
                    *hit_sfx_cooldown = HIT_SFX_INTERVAL_TICKS;
                }
                if zombie.hp <= 0 {
                    let z_kind = zombie.kind;
                    let was_exploder = matches!(z_kind, ZombieKind::Exploder);
                    commands.entity(z_entity).despawn_recursive();
                    killed.send(ZombieKilledEvent {
                        kind: z_kind,
                        by_explosion: false,
                        pos: zp,
                    });
                    score.0 += z_kind.kill_reward() * mult as u32;
                    sfx.send(SfxEvent::ZombieDeath);
                    if was_exploder {
                        explode.send(ExplodeEvent {
                            pos: zp,
                            radius: EXPLODER_EXPLOSION_RADIUS,
                            zombie_damage: EXPLODER_EXPLOSION_ZOMBIE_DAMAGE,
                            player_damage: EXPLODER_EXPLOSION_PLAYER_DAMAGE,
                        });
                    }
                }
                consumed = true;
                break;
            }
        }
        if consumed {
            continue;
        }

        // Then explodable props (cars, fuel barrels).  Bullets always
        // consume on hit; rockets that hit a wreck go straight to a chain
        // explosion (skip the prop's own HP roll).
        for (e_entity, e_transform, mut expl, obs_idx) in &mut explodables {
            if expl.hp <= 0 {
                continue;
            }
            let ep = e_transform.translation.truncate();
            // Use the prop's collision half + bullet radius for hit-test
            // (so big wrecks are easy to plink, small barrels still feel
            // accurate).
            let half = expl.kind.collision_half();
            let dx = (bp.x - ep.x).abs();
            let dy = (bp.y - ep.y).abs();
            if dx < half.x + BULLET_RADIUS && dy < half.y + BULLET_RADIUS {
                if bullet.is_rocket {
                    explode.send(ExplodeEvent {
                        pos: bp,
                        radius: ROCKET_EXPLOSION_RADIUS,
                        zombie_damage: ROCKET_EXPLOSION_ZOMBIE_DAMAGE,
                        player_damage: ROCKET_EXPLOSION_PLAYER_DAMAGE,
                    });
                    commands.entity(b_entity).despawn_recursive();
                    consumed = true;
                    break;
                }
                expl.hp -= bullet.damage;
                // Visual feedback for plink hits — without this the bullet
                // disappears silently when it doesn't destroy the prop and
                // it feels like a missed hit.  Flame puffs have their own
                // dissipation visual so they opt out.
                if !bullet.is_flame {
                    spawn_impact_sparks(&mut commands, &assets, bp, bullet.velocity);
                }
                commands.entity(b_entity).despawn_recursive();
                if *hit_sfx_cooldown == 0 {
                    sfx.send(SfxEvent::Hit);
                    *hit_sfx_cooldown = HIT_SFX_INTERVAL_TICKS;
                }
                if expl.hp <= 0 {
                    if let Some(o) = obstacles.list.get_mut(obs_idx.0) {
                        o.shape = ObstacleShape::Circle(0.0);
                    }
                    destroyed.indices.insert(obs_idx.0 as u32);
                    explode.send(ExplodeEvent {
                        pos: ep,
                        radius: expl.radius,
                        zombie_damage: expl.zombie_damage,
                        player_damage: expl.player_damage,
                    });
                    spawn_smoke_column(&mut commands, &assets, ep);
                    commands.entity(e_entity).despawn_recursive();
                }
                consumed = true;
                break;
            }
        }
        let _ = consumed;
    }
}

#[allow(clippy::too_many_arguments)]
fn explode_listener(
    mut commands: Commands,
    assets: Res<BulletAssets>,
    mut events: EventReader<ExplodeEvent>,
    mut zombies: Query<(Entity, &Transform, &mut Zombie)>,
    mut explodables: Query<(Entity, &Transform, &mut Explodable, &ExplodableObstacleIdx)>,
    mut obstacles: ResMut<MapObstacles>,
    mut destroyed: ResMut<DestroyedExplodables>,
    players: Query<(&Transform, &Player)>,
    mut damage_evw: EventWriter<PlayerDamagedEvent>,
    mut killed_evw: EventWriter<ZombieKilledEvent>,
    mut dmg_numbers: EventWriter<DamageNumberEvent>,
    mut sfx: EventWriter<SfxEvent>,
    mut score: ResMut<Score>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
) {
    let mult = max_money_mult(players.iter().map(|(_, p)| p));
    let mut queue: Vec<ExplodeEvent> = events.read().copied().collect();
    while let Some(ev) = queue.pop() {
        for (z_ent, z_t, mut zombie) in &mut zombies {
            if zombie.hp <= 0 {
                continue;
            }
            let zp = z_t.translation.truncate();
            let r = ev.radius + zombie.kind.radius();
            if zp.distance_squared(ev.pos) < r * r {
                zombie.hp -= ev.zombie_damage;
                zombie.hit_flash = ZOMBIE_HIT_FLASH_DURATION;
                dmg_numbers.send(DamageNumberEvent {
                    pos: zp,
                    amount: ev.zombie_damage,
                });
                if zombie.hp <= 0 {
                    let z_kind = zombie.kind;
                    let was_exploder = matches!(z_kind, ZombieKind::Exploder);
                    commands.entity(z_ent).despawn_recursive();
                    killed_evw.send(ZombieKilledEvent {
                        kind: z_kind,
                        by_explosion: true,
                        pos: zp,
                    });
                    score.0 += z_kind.kill_reward() * mult as u32;
                    if was_exploder {
                        queue.push(ExplodeEvent {
                            pos: zp,
                            radius: EXPLODER_EXPLOSION_RADIUS,
                            zombie_damage: EXPLODER_EXPLOSION_ZOMBIE_DAMAGE,
                            player_damage: EXPLODER_EXPLOSION_PLAYER_DAMAGE,
                        });
                    }
                }
            }
        }
        // Chain explosions through nearby explodables (cars, barrels).
        // Apply ev's zombie_damage to their HP — that lets ordinary grenades
        // weaken a wreck without instantly detonating it, while a rocket's
        // big damage still chains as expected.
        for (e_ent, e_t, mut expl, obs_idx) in &mut explodables {
            if expl.hp <= 0 {
                continue;
            }
            let ep = e_t.translation.truncate();
            let r = ev.radius + 16.0;
            if ep.distance_squared(ev.pos) < r * r {
                expl.hp -= ev.zombie_damage.max(8);
                if expl.hp <= 0 {
                    if let Some(o) = obstacles.list.get_mut(obs_idx.0) {
                        o.shape = ObstacleShape::Circle(0.0);
                    }
                    destroyed.indices.insert(obs_idx.0 as u32);
                    queue.push(ExplodeEvent {
                        pos: ep,
                        radius: expl.radius,
                        zombie_damage: expl.zombie_damage,
                        player_damage: expl.player_damage,
                    });
                    spawn_smoke_column(&mut commands, &assets, ep);
                    commands.entity(e_ent).despawn_recursive();
                }
            }
        }
        for (p_t, player) in &players {
            if player.hp <= 0 {
                continue;
            }
            let pp = p_t.translation.truncate();
            let r = ev.radius + PLAYER_RADIUS;
            if pp.distance_squared(ev.pos) < r * r {
                damage_evw.send(PlayerDamagedEvent {
                    target_id: player.id,
                    amount: ev.player_damage,
                });
            }
        }
        let net_id = ctx.alloc_explosion_id();
        let ent = spawn_explosion_entity(&mut commands, &assets, ev.pos, ev.radius, net_id);
        net_entities.explosions.insert(net_id, ent);
        // Cosmetic shockwave ring — purely client-side flair on top of the
        // explosion sprite.  Not synced; both host and clients spawn one
        // independently when they receive the explosion.
        let shock_life = 0.4;
        commands.spawn((
            SpriteBundle {
                texture: assets.shockwave.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(ev.radius * 0.4)),
                    color: Color::srgba(1.0, 0.85, 0.5, 0.9),
                    ..default()
                },
                transform: Transform::from_xyz(ev.pos.x, ev.pos.y, 9.6),
                ..default()
            },
            ShockwaveRing {
                lifetime: shock_life,
                max_lifetime: shock_life,
                max_radius: ev.radius * 2.6,
            },
        ));
        sfx.send(SfxEvent::Explosion);
    }
}

fn throw_listener(
    mut commands: Commands,
    mut events: EventReader<ThrowEvent>,
    assets: Res<BulletAssets>,
) {
    for ev in events.read() {
        let speed = ev.kind.throw_speed();
        let fuse = THROW_RANGE / speed;
        let angle = ev.direction.y.atan2(ev.direction.x);
        commands.spawn((
            SpriteBundle {
                texture: assets.thrown.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(10.0)),
                    ..default()
                },
                transform: Transform::from_xyz(ev.origin.x, ev.origin.y, 8.0)
                    .with_rotation(Quat::from_rotation_z(angle)),
                ..default()
            },
            ThrownProjectile {
                velocity: ev.direction * speed,
                fuse,
                kind: ev.kind,
            },
        ));
    }
}

fn thrown_projectile_update(
    mut commands: Commands,
    time: Res<Time>,
    obstacles: Res<MapObstacles>,
    fx_assets: Res<ThrowableFxAssets>,
    mut q: Query<(Entity, &mut Transform, &mut ThrownProjectile)>,
    mut explode: EventWriter<ExplodeEvent>,
) {
    let dt = time.delta_seconds();
    for (entity, mut transform, mut proj) in &mut q {
        transform.translation += (proj.velocity * dt).extend(0.0);
        proj.fuse -= dt;
        let pos = transform.translation.truncate();
        let hit = obstacles.hits(pos, 4.0);
        let expired = proj.fuse <= 0.0;
        // Molotov explodes on obstacle hit or range
        let detonate = expired || (hit && proj.kind == ThrowableKind::Molotov);
        if !detonate {
            continue;
        }
        commands.entity(entity).despawn_recursive();
        match proj.kind {
            ThrowableKind::Grenade => {
                explode.send(ExplodeEvent {
                    pos,
                    radius: GRENADE_EXPLOSION_RADIUS,
                    zombie_damage: GRENADE_ZOMBIE_DAMAGE,
                    player_damage: GRENADE_PLAYER_DAMAGE,
                });
            }
            ThrowableKind::Smoke => {
                spawn_smoke_cloud(&mut commands, &fx_assets, pos);
            }
            ThrowableKind::Molotov => {
                spawn_fire_pool(&mut commands, &fx_assets, pos);
            }
        }
    }
}

fn spawn_smoke_cloud(commands: &mut Commands, fx: &ThrowableFxAssets, pos: Vec2) {
    use bevy::sprite::MaterialMesh2dBundle;
    commands.spawn((
        MaterialMesh2dBundle {
            mesh: fx.smoke_mesh.clone(),
            material: fx.smoke_material.clone(),
            transform: Transform::from_xyz(pos.x, pos.y, 9.0),
            ..default()
        },
        SmokeCloud {
            lifetime: SMOKE_DURATION,
            radius: SMOKE_RADIUS,
        },
    ));
}

fn spawn_fire_pool(commands: &mut Commands, fx: &ThrowableFxAssets, pos: Vec2) {
    use bevy::sprite::MaterialMesh2dBundle;
    commands.spawn((
        MaterialMesh2dBundle {
            mesh: fx.fire_mesh.clone(),
            material: fx.fire_material.clone(),
            transform: Transform::from_xyz(pos.x, pos.y, 9.0),
            ..default()
        },
        FirePool {
            lifetime: FIRE_DURATION,
            radius: FIRE_RADIUS,
            tick_timer: 0.0,
        },
    ));
}

fn smoke_cloud_update(
    mut commands: Commands,
    time: Res<Time>,
    mut clouds: Query<(Entity, &mut SmokeCloud, &Transform)>,
    mut zombies: Query<
        (Entity, &Transform, &mut Zombie, Option<&mut SmokeSlowed>),
        Without<SmokeCloud>,
    >,
) {
    let dt = time.delta_seconds();
    for (entity, mut cloud, _) in &mut clouds {
        cloud.lifetime -= dt;
        if cloud.lifetime <= 0.0 {
            commands.entity(entity).despawn_recursive();
        }
    }
    if clouds.is_empty() {
        return;
    }
    // Slow zombies inside a cloud.  The slow is a refreshed timed debuff —
    // `expire_smoke_slow` hands the stored original speed back once the
    // zombie has been outside every cloud for `SMOKE_SLOW_LINGER` seconds —
    // rather than a permanent overwrite of the stat.
    for (z_entity, zt, mut zombie, slow) in &mut zombies {
        let zp = zt.translation.truncate();
        let inside = clouds.iter().any(|(_, cloud, cloud_t)| {
            let r = cloud.radius + zombie.kind.radius();
            cloud.lifetime > 0.0
                && zp.distance_squared(cloud_t.translation.truncate()) < r * r
        });
        if !inside {
            continue;
        }
        if let Some(mut slow) = slow {
            slow.remaining = SMOKE_SLOW_LINGER;
        } else {
            commands.entity(z_entity).insert(SmokeSlowed {
                base_speed: zombie.speed,
                remaining: SMOKE_SLOW_LINGER,
            });
            zombie.speed *= SMOKE_SLOW_FACTOR;
        }
    }
}

/// Counts down the smoke-cloud slow and restores the zombie's original
/// speed (per-spawn jitter included) when it runs out.  Runs right after
/// `smoke_cloud_update` in the tick, so a zombie still inside a cloud has
/// its timer refreshed before it is ever decremented here.
fn expire_smoke_slow(
    mut commands: Commands,
    time: Res<Time>,
    mut slowed: Query<(Entity, &mut Zombie, &mut SmokeSlowed)>,
) {
    let dt = time.delta_seconds();
    for (entity, mut zombie, mut slow) in &mut slowed {
        slow.remaining -= dt;
        if slow.remaining <= 0.0 {
            zombie.speed = slow.base_speed;
            commands.entity(entity).remove::<SmokeSlowed>();
        }
    }
}

fn fire_pool_update(
    mut commands: Commands,
    time: Res<Time>,
    mut fires: Query<(Entity, &mut FirePool, &Transform)>,
    mut zombies: Query<(Entity, &Transform, &mut Zombie), Without<FirePool>>,
    players: Query<&Player>,
    mut killed: EventWriter<ZombieKilledEvent>,
    mut score: ResMut<Score>,
) {
    let dt = time.delta_seconds();
    let mult = max_money_mult(players.iter());
    for (entity, mut fire, fire_t) in &mut fires {
        fire.lifetime -= dt;
        if fire.lifetime <= 0.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        fire.tick_timer -= dt;
        if fire.tick_timer > 0.0 {
            continue;
        }
        fire.tick_timer = FIRE_TICK;
        let fp = fire_t.translation.truncate();
        for (z_ent, zt, mut zombie) in &mut zombies {
            if zombie.hp <= 0 {
                continue;
            }
            let zp = zt.translation.truncate();
            let r = fire.radius + zombie.kind.radius();
            if zp.distance_squared(fp) < r * r {
                zombie.hp -= FIRE_DAMAGE;
                zombie.hit_flash = ZOMBIE_HIT_FLASH_DURATION;
                if zombie.hp <= 0 {
                    let z_kind = zombie.kind;
                    commands.entity(z_ent).despawn_recursive();
                    killed.send(ZombieKilledEvent {
                        kind: z_kind,
                        by_explosion: true,
                        pos: zp,
                    });
                    score.0 += z_kind.kill_reward() * mult as u32;
                }
            }
        }
    }
}

fn explosion_lifetime(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Explosion, &mut Sprite, &NetId)>,
    mut net_entities: ResMut<NetEntities>,
) {
    let dt = time.delta_seconds();
    for (e, mut exp, mut sprite, net_id) in &mut q {
        exp.lifetime -= dt;
        if exp.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            net_entities.explosions.remove(&net_id.0);
            continue;
        }
        let t = (exp.lifetime / EXPLOSION_LIFETIME).clamp(0.0, 1.0);
        let phase = 1.0 - t;
        let scale = 1.1 + phase;
        sprite.custom_size = Some(Vec2::splat(exp.radius * scale));
    }
}

#[allow(clippy::type_complexity)]
fn update_throw_indicator(
    players: Query<(&Transform, &Player), Without<ThrowIndicator>>,
    ctx: Res<NetContext>,
    mut indicators: Query<
        (&mut Transform, &mut Visibility),
        (With<ThrowIndicator>, Without<Player>),
    >,
) {
    let Ok((mut ind_t, mut vis)) = indicators.get_single_mut() else {
        return;
    };
    let my_player = players.iter().find(|(_, p)| p.id == ctx.my_id);
    match my_player {
        Some((t, p)) if p.active_slot == 2 && p.throwable_count > 0 && p.hp > 0 => {
            *vis = Visibility::Visible;
            let pos = t.translation.truncate() + p.aim * THROW_RANGE;
            ind_t.translation.x = pos.x;
            ind_t.translation.y = pos.y;
        }
        _ => {
            *vis = Visibility::Hidden;
        }
    }
}

#[allow(clippy::type_complexity)]
fn despawn_all_bullets(
    mut commands: Commands,
    q: Query<
        Entity,
        Or<(
            With<Bullet>,
            With<Explosion>,
            With<ThrownProjectile>,
            With<SmokeCloud>,
            With<FirePool>,
            With<MuzzleFlash>,
            With<ImpactSpark>,
            With<ShockwaveRing>,
            With<SmokePuff>,
            With<BulletTracer>,
            With<BulletShell>,
            With<AmbientEmber>,
        )>,
    >,
    mut net_entities: ResMut<NetEntities>,
) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
    net_entities.explosions.clear();
}

/// Highest active money multiplier across all alive players (min 1× when
/// nobody has a buff).  Used for kill payouts so all players benefit from
/// the strongest active buff in the squad.
fn max_money_mult<'a>(mut players: impl Iterator<Item = &'a Player>) -> u8 {
    players.by_ref().map(|p| p.money_mult).max().unwrap_or(1).max(1)
}

// ── FX update systems ─────────────────────────────────────────────────

fn update_muzzle_flashes(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut MuzzleFlash, &mut Sprite, &mut Transform)>,
) {
    let dt = time.delta_seconds();
    for (e, mut flash, mut sprite, mut transform) in &mut q {
        flash.lifetime -= dt;
        if flash.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            continue;
        }
        let pct = (flash.lifetime / flash.max_lifetime).clamp(0.0, 1.0);
        // Quick brighten-then-fade so the flash punches but doesn't linger.
        let alpha = (pct * 1.2).min(1.0);
        sprite.color.set_alpha(alpha);
        // Slight scale punch outward.
        let scale = 1.0 + (1.0 - pct) * 0.4;
        if let Some(size) = sprite.custom_size {
            transform.scale = Vec3::new(scale, scale, 1.0);
            // Keep size constant; rely on transform.scale for the punch.
            let _ = size;
        }
    }
}

fn update_impact_sparks(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut ImpactSpark, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_seconds();
    for (e, mut spark, mut transform, mut sprite) in &mut q {
        spark.lifetime -= dt;
        if spark.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            continue;
        }
        // Decelerate and drop with a hint of gravity for liveliness.
        spark.velocity *= 0.94;
        spark.velocity.y -= 220.0 * dt;
        transform.translation += (spark.velocity * dt).extend(0.0);
        let pct = (spark.lifetime / spark.max_lifetime).clamp(0.0, 1.0);
        sprite.color.set_alpha(pct);
    }
}

fn update_shockwaves(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut ShockwaveRing, &mut Sprite)>,
) {
    let dt = time.delta_seconds();
    for (e, mut ring, mut sprite) in &mut q {
        ring.lifetime -= dt;
        if ring.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            continue;
        }
        let pct = (ring.lifetime / ring.max_lifetime).clamp(0.0, 1.0);
        let progress = 1.0 - pct;
        let radius = ring.max_radius * (0.18 + progress * 0.85);
        sprite.custom_size = Some(Vec2::splat(radius));
        // Fade strongly toward the end.
        sprite.color.set_alpha(pct * pct);
    }
}

fn update_smoke_puffs(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut SmokePuff, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_seconds();
    for (e, mut puff, mut transform, mut sprite) in &mut q {
        puff.lifetime -= dt;
        if puff.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            continue;
        }
        // Drift up, decelerate slightly with air drag.
        puff.velocity *= 1.0 - 0.4 * dt;
        transform.translation += (puff.velocity * dt).extend(0.0);
        let pct = (puff.lifetime / puff.max_lifetime).clamp(0.0, 1.0);
        let progress = 1.0 - pct;
        let size = puff.start_size + (puff.end_size - puff.start_size) * progress;
        sprite.custom_size = Some(Vec2::splat(size));
        // Hold opacity briefly, then fade.
        let alpha = if pct > 0.65 {
            0.55
        } else {
            (pct / 0.65) * 0.55
        };
        sprite.color.set_alpha(alpha);
    }
}

fn update_bullet_tracers(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut BulletTracer, &mut Sprite)>,
) {
    let dt = time.delta_seconds();
    for (e, mut tr, mut sprite) in &mut q {
        tr.lifetime -= dt;
        if tr.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            continue;
        }
        let pct = (tr.lifetime / tr.max_lifetime).clamp(0.0, 1.0);
        sprite.color.set_alpha(pct);
    }
}

/// Spawns a few ambient ember / ash particles per second within a square
/// around the camera position.  Cheap atmospheric flair — capped count so
/// it never overwhelms.
fn spawn_ambient_embers(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<BulletAssets>,
    cameras: Query<&Transform, With<Camera>>,
    existing: Query<(), With<AmbientEmber>>,
    mut timer: Local<f32>,
) {
    *timer += time.delta_seconds();
    if *timer < 0.07 {
        return;
    }
    *timer = 0.0;
    if existing.iter().count() > 80 {
        return; // soft cap so the world doesn't fill with embers
    }
    let Ok(cam) = cameras.get_single() else {
        return;
    };
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let cam_pos = cam.translation.truncate();
    // Spawn anywhere within ~700 px around the camera so embers are
    // visible at any zoom level.
    for _ in 0..rng.gen_range(1..=3) {
        let x = cam_pos.x + rng.gen_range(-700.0..700.0);
        let y = cam_pos.y + rng.gen_range(-450.0..450.0);
        let life = rng.gen_range(2.5..5.0);
        let vy = rng.gen_range(20.0..45.0);
        let vx = rng.gen_range(-12.0..12.0);
        // Subtle colour variation between embers (warm) and ash (grey).
        let warm = rng.gen_bool(0.55);
        let color = if warm {
            Color::srgba(1.0, 0.55, 0.18, 0.85)
        } else {
            Color::srgba(0.62, 0.58, 0.50, 0.6)
        };
        commands.spawn((
            SpriteBundle {
                texture: assets.ember.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(rng.gen_range(2.0..4.0))),
                    color,
                    ..default()
                },
                transform: Transform::from_xyz(x, y, 7.5),
                ..default()
            },
            AmbientEmber {
                velocity: Vec2::new(vx, vy),
                lifetime: life,
                max_lifetime: life,
                flicker: rng.gen_range(0.0..std::f32::consts::TAU),
            },
        ));
    }
}

fn update_ambient_embers(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut AmbientEmber, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_seconds();
    let t = time.elapsed_seconds();
    for (e, mut em, mut transform, mut sprite) in &mut q {
        em.lifetime -= dt;
        if em.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            continue;
        }
        // Slight side-to-side wobble so embers don't rise in straight lines.
        let wob = (t * 3.0 + em.flicker).sin() * 6.0;
        transform.translation += (em.velocity * dt).extend(0.0);
        transform.translation.x += wob * dt;
        let pct = (em.lifetime / em.max_lifetime).clamp(0.0, 1.0);
        // Gentle alpha ramp-in for the first 20% then smooth fade out.
        let alpha = if pct > 0.8 {
            ((1.0 - pct) / 0.2).clamp(0.0, 1.0)
        } else {
            pct
        };
        sprite.color.set_alpha(alpha * 0.85);
    }
}

/// Tiny dust puff at the player's feet during running — re-uses the smoke
/// puff sprite + lifetime model with a smaller scale and earthier tint.
pub fn spawn_walking_dust(
    commands: &mut Commands,
    assets: &BulletAssets,
    pos: Vec2,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let life = rng.gen_range(0.32..0.55);
    let start = rng.gen_range(10.0..16.0);
    let end = rng.gen_range(20.0..28.0);
    let vy = rng.gen_range(8.0..18.0);
    let vx = rng.gen_range(-12.0..12.0);
    commands.spawn((
        SpriteBundle {
            texture: assets.smoke.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::splat(start)),
                color: Color::srgba(0.62, 0.55, 0.42, 0.0),
                ..default()
            },
            // Slightly under the player but above the ground tiles.
            transform: Transform::from_xyz(pos.x, pos.y - 6.0, -10.0),
            ..default()
        },
        SmokePuff {
            lifetime: life,
            max_lifetime: life,
            velocity: Vec2::new(vx, vy),
            start_size: start,
            end_size: end,
        },
    ));
}

/// Spawns a column of 5 smoke puffs above an explodable that just blew up.
/// Each puff drifts upward with a small horizontal jitter so the column
/// reads as natural rising smoke rather than identical clones.
pub fn spawn_smoke_column(
    commands: &mut Commands,
    assets: &BulletAssets,
    pos: Vec2,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    for i in 0..6 {
        let dx = rng.gen_range(-12.0..12.0);
        let dy = i as f32 * 8.0; // staggered initial heights
        let life = rng.gen_range(2.4..3.6);
        let start = rng.gen_range(40.0..60.0);
        let end = rng.gen_range(95.0..130.0);
        let vy = rng.gen_range(45.0..70.0);
        let vx = rng.gen_range(-12.0..12.0);
        commands.spawn((
            SpriteBundle {
                texture: assets.smoke.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(start)),
                    color: Color::srgba(0.45, 0.42, 0.40, 0.0),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x + dx, pos.y + dy, 6.5),
                ..default()
            },
            SmokePuff {
                lifetime: life,
                max_lifetime: life,
                velocity: Vec2::new(vx, vy),
                start_size: start,
                end_size: end,
            },
        ));
    }
}

/// Per-system accumulators that pace smolder emission.  Smoke ticks slowly
/// (a lazy plume); flame ticks fast (a lively flicker).  Held in one `Local`
/// so the system stays a single param.
#[derive(Default)]
struct SmolderEmit {
    smoke: f32,
    flame: f32,
}

/// Wrecks farther than this from the camera skip smolder FX entirely —
/// comfortably beyond the visible half-diagonal at any zoom, so on-screen
/// plumes are never clipped while off-screen ones cost nothing.
const SMOLDER_CULL_RADIUS: f32 = 800.0;

/// Continuously emits the damage FX for every smoldering explodable: a rising
/// smoke plume once a wreck is hurt (`stage >= 1`), plus licking flames once
/// it's badly damaged and about to blow (`stage == 2`).  Runs on every machine
/// — the host derives the stage from HP, clients read it off the snapshot, so
/// the same wreck smokes and burns identically for everyone.
fn animate_explodable_smolder(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<BulletAssets>,
    q: Query<(&Transform, &ExplodableSmolder)>,
    cameras: Query<&Transform, With<Camera>>,
    mut emit: Local<SmolderEmit>,
) {
    let dt = time.delta_seconds();
    emit.smoke += dt;
    emit.flame += dt;
    // Smoke about 3×/s, flame about 8×/s — fast enough to read as a live fire
    // without flooding the scene with sprites when several wrecks burn at once.
    let do_smoke = emit.smoke >= 0.32;
    let do_flame = emit.flame >= 0.12;
    if do_smoke {
        emit.smoke = 0.0;
    }
    if do_flame {
        emit.flame = 0.0;
    }
    if !do_smoke && !do_flame {
        return;
    }
    // The FX is per-machine cosmetic (stage syncs via snapshot), so cull to
    // what this camera can see — damaged wrecks accumulate over a match and
    // never heal, and off-screen plumes are pure spawn/animate/despawn
    // churn.  Mirrors the ambient-ember cull above.
    let Ok(cam) = cameras.get_single() else {
        return;
    };
    let cam_pos = cam.translation.truncate();
    for (t, sm) in &q {
        if sm.stage == 0 {
            continue;
        }
        let pos = t.translation.truncate();
        if pos.distance_squared(cam_pos) > SMOLDER_CULL_RADIUS * SMOLDER_CULL_RADIUS {
            continue;
        }
        if do_smoke {
            spawn_damage_smoke(&mut commands, &assets, pos, sm.stage);
        }
        if do_flame && sm.stage >= 2 {
            spawn_damage_flame(&mut commands, &assets, pos);
        }
    }
}

/// One rising smoke puff off a damaged wreck.  Darker and fatter once the
/// wreck is burning (`stage == 2`) so the plume visibly thickens as it nears
/// detonation.  Reuses the `SmokePuff` drift/grow/fade animation.
fn spawn_damage_smoke(commands: &mut Commands, assets: &BulletAssets, pos: Vec2, stage: u8) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let burning = stage >= 2;
    let dx = rng.gen_range(-11.0..11.0);
    let start = if burning {
        rng.gen_range(20.0..28.0)
    } else {
        rng.gen_range(12.0..18.0)
    };
    let end = start + rng.gen_range(30.0..46.0);
    let life = rng.gen_range(1.1..1.7);
    let vy = rng.gen_range(28.0..46.0);
    let vx = rng.gen_range(-9.0..9.0);
    // Sooty grey-brown; darker when actively on fire.
    let tint = if burning { 0.26 } else { 0.42 };
    commands.spawn((
        SpriteBundle {
            texture: assets.smoke.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::splat(start)),
                color: Color::srgba(tint, tint * 0.95, tint * 0.9, 0.0),
                ..default()
            },
            transform: Transform::from_xyz(pos.x + dx, pos.y + 6.0, 6.4),
            ..default()
        },
        SmokePuff {
            lifetime: life,
            max_lifetime: life,
            velocity: Vec2::new(vx, vy),
            start_size: start,
            end_size: end,
        },
    ));
}

/// One short-lived flame lick on a wreck that's about to explode.  Small,
/// fast-rising, barely grows — reads as fire rather than smoke.  The warm
/// tint catches the bloom pass on desktop for an extra glow.
fn spawn_damage_flame(commands: &mut Commands, assets: &BulletAssets, pos: Vec2) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let dx = rng.gen_range(-10.0..10.0);
    let dy = rng.gen_range(-4.0..4.0);
    let size = rng.gen_range(16.0..26.0);
    let life = rng.gen_range(0.34..0.55);
    let vy = rng.gen_range(42.0..72.0);
    let vx = rng.gen_range(-12.0..12.0);
    commands.spawn((
        SpriteBundle {
            texture: assets.flame.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::splat(size)),
                color: Color::srgba(1.0, 0.85, 0.5, 0.0),
                ..default()
            },
            transform: Transform::from_xyz(pos.x + dx, pos.y + dy + 4.0, 6.6),
            ..default()
        },
        SmokePuff {
            lifetime: life,
            max_lifetime: life,
            velocity: Vec2::new(vx, vy),
            start_size: size,
            end_size: size * 1.15,
        },
    ));
}

fn update_bullet_shells(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut BulletShell, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_seconds();
    for (e, mut shell, mut transform, mut sprite) in &mut q {
        shell.lifetime -= dt;
        if shell.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            continue;
        }
        // Aerodynamic drag pulls velocity & spin down so the casing rolls
        // to a stop after about 0.4 s.
        shell.velocity *= 1.0 - 4.5 * dt;
        shell.spin *= 1.0 - 4.0 * dt;
        transform.translation += (shell.velocity * dt).extend(0.0);
        transform.rotate_z(shell.spin * dt);
        let pct = (shell.lifetime / shell.max_lifetime).clamp(0.0, 1.0);
        // Hold full alpha for the first 70% of life, then fade smoothly.
        let alpha = if pct > 0.3 { 1.0 } else { pct / 0.3 };
        sprite.color.set_alpha(alpha);
    }
}

/// Spawns a brass casing ejected to the right of the firing direction with
/// a touch of randomness.  Lifetime ~3.5 s so the floor briefly fills with
/// spent brass in heavy combat.
fn spawn_bullet_shell(
    commands: &mut Commands,
    assets: &BulletAssets,
    origin: Vec2,
    direction: Vec2,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let dir = if direction.length_squared() > 0.0001 {
        direction.normalize()
    } else {
        Vec2::X
    };
    // Eject sideways relative to firing direction (rotated 90°).
    let side_sign: f32 = if rng.gen_bool(0.5) { 1.0 } else { -1.0 };
    let perp = Vec2::new(-dir.y * side_sign, dir.x * side_sign);
    // Mild random spread along the ejection direction.
    let speed = rng.gen_range(120.0..200.0);
    let velocity = perp * speed + Vec2::new(rng.gen_range(-20.0..20.0), rng.gen_range(-15.0..30.0));
    let life = rng.gen_range(3.0..4.5);
    let initial_angle = dir.y.atan2(dir.x) + side_sign * std::f32::consts::FRAC_PI_2;
    commands.spawn((
        SpriteBundle {
            texture: assets.shell.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(7.0, 3.0)),
                color: Color::srgba(1.0, 1.0, 1.0, 1.0),
                ..default()
            },
            transform: Transform::from_xyz(origin.x, origin.y, -8.0)
                .with_rotation(Quat::from_rotation_z(initial_angle)),
            ..default()
        },
        BulletShell {
            velocity,
            spin: rng.gen_range(8.0..18.0) * side_sign,
            lifetime: life,
            max_lifetime: life,
        },
    ));
}

/// Spawns a short, fading streak in the direction the bullet is travelling.
/// Renders right at the gun tip; pure cosmetic so the firing path is
/// visible even when the bullet itself is small.
fn spawn_bullet_tracer(
    commands: &mut Commands,
    assets: &BulletAssets,
    origin: Vec2,
    direction: Vec2,
) {
    let dir = if direction.length_squared() > 0.0001 {
        direction.normalize()
    } else {
        Vec2::X
    };
    let angle = dir.y.atan2(dir.x);
    let length = 60.0;
    // Origin at the bullet start, sprite anchored centred so we offset
    // forward by half the length.
    let offset = dir * (length * 0.5);
    let life = 0.18;
    commands.spawn((
        SpriteBundle {
            texture: assets.tracer.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(length, 4.0)),
                color: Color::srgba(1.0, 0.95, 0.55, 0.85),
                ..default()
            },
            transform: Transform::from_xyz(origin.x + offset.x, origin.y + offset.y, 8.7)
                .with_rotation(Quat::from_rotation_z(angle)),
            ..default()
        },
        BulletTracer {
            lifetime: life,
            max_lifetime: life,
        },
    ));
}

// ── FX sprite builders ────────────────────────────────────────────────

fn build_muzzle_flash_image() -> Image {
    let core: Rgba = [255, 255, 240, 255];
    let hot: Rgba = [255, 240, 160, 255];
    let mid: Rgba = [255, 180, 60, 255];
    let edge: Rgba = [220, 90, 30, 220];
    let smoke: Rgba = [180, 130, 80, 120];

    let mut c = Canvas::new(28, 16);
    // Star burst in the centre — tip points to +X (gun barrel direction).
    c.fill_rect(0, 0, 28, 16, [0, 0, 0, 0]);
    c.fill_circle(12, 8, 6, edge);
    c.fill_circle(13, 8, 5, mid);
    c.fill_circle(14, 8, 4, hot);
    c.fill_circle(14, 8, 2, core);
    // Forward jet of flame (toward muzzle tip)
    c.fill_rect(18, 7, 6, 2, hot);
    c.fill_rect(20, 7, 4, 2, mid);
    c.fill_rect(22, 8, 4, 1, edge);
    c.put(26, 8, edge);
    // A few smoke pixels trailing the burst.
    c.put(2, 9, smoke);
    c.put(4, 6, smoke);
    c.put(6, 11, smoke);
    // Star spikes (4 cardinal) for that classic burst silhouette.
    c.fill_rect(13, 1, 2, 14, hot);
    c.fill_rect(7, 7, 14, 2, hot);
    c.fill_rect(13, 3, 2, 10, core);
    c.fill_rect(9, 7, 10, 2, core);

    c.into_image()
}

fn build_spark_image() -> Image {
    // A single bright pixel-cluster — the sprite is small and gets scaled;
    // we use a 5×5 canvas so it's a clean diamond-ish shape.
    let core: Rgba = [255, 250, 220, 255];
    let mid: Rgba = [255, 220, 120, 255];
    let edge: Rgba = [255, 150, 60, 255];

    let mut c = Canvas::new(5, 5);
    c.fill_rect(0, 0, 5, 5, [0, 0, 0, 0]);
    c.put(2, 0, edge);
    c.put(2, 4, edge);
    c.put(0, 2, edge);
    c.put(4, 2, edge);
    c.put(1, 1, mid);
    c.put(3, 1, mid);
    c.put(1, 3, mid);
    c.put(3, 3, mid);
    c.put(2, 1, mid);
    c.put(1, 2, mid);
    c.put(3, 2, mid);
    c.put(2, 3, mid);
    c.put(2, 2, core);
    c.into_image()
}

fn build_ember_image() -> Image {
    // Tiny 3×3 glowing pixel — bright core, dim halo.  Sprite is scaled
    // to 2-4× at spawn so the pixel feel remains.
    let core: Rgba = [255, 235, 180, 255];
    let mid: Rgba = [255, 170, 90, 255];
    let edge: Rgba = [180, 90, 30, 200];
    let mut c = Canvas::new(3, 3);
    c.fill_rect(0, 0, 3, 3, [0, 0, 0, 0]);
    c.put(1, 1, core);
    c.put(0, 1, mid);
    c.put(2, 1, mid);
    c.put(1, 0, mid);
    c.put(1, 2, mid);
    c.put(0, 0, edge);
    c.put(2, 0, edge);
    c.put(0, 2, edge);
    c.put(2, 2, edge);
    c.into_image()
}

fn build_shell_image() -> Image {
    // Brass casing — small, gold-coloured, with a darker rim at the case
    // mouth.  Anchored centred so rotation looks natural.
    let body: Rgba = [205, 170, 60, 255];
    let body_light: Rgba = [240, 215, 110, 255];
    let body_dark: Rgba = [120, 90, 30, 255];
    let rim: Rgba = [60, 44, 18, 255];
    let mut c = Canvas::new(7, 3);
    c.fill_rect(0, 0, 7, 3, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 7, 3, body);
    c.fill_rect(0, 0, 7, 1, body_light);
    c.fill_rect(0, 2, 7, 1, body_dark);
    c.fill_rect(6, 0, 1, 3, rim);
    c.into_image()
}

fn build_smoke_puff_image() -> Image {
    // Soft round blob biased toward grey-brown — multiplied by sprite color
    // (0.45, 0.42, 0.40) at spawn so the texture itself stays pale.
    let outer: Rgba = [220, 220, 220, 255];
    let mid: Rgba = [240, 240, 240, 255];
    let core: Rgba = [255, 255, 255, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_circle(16, 16, 14, outer);
    c.fill_circle(16, 16, 11, mid);
    c.fill_circle(15, 15, 7, core);
    // Ragged edge so the blob reads as smoke, not a planet.
    for &(x, y) in &[(4, 18), (28, 14), (15, 4), (16, 28), (8, 8), (24, 24)] {
        c.fill_circle(x, y, 3, mid);
    }
    c.into_image()
}

fn build_tracer_image() -> Image {
    // Horizontal streak — bright core, fading toward both ends.
    let edge: Rgba = [255, 200, 80, 220];
    let mid: Rgba = [255, 230, 140, 255];
    let core: Rgba = [255, 255, 220, 255];
    let mut c = Canvas::new(32, 4);
    c.fill_rect(0, 0, 32, 4, [0, 0, 0, 0]);
    c.fill_rect(0, 1, 32, 2, edge);
    c.fill_rect(2, 1, 28, 2, mid);
    c.fill_rect(6, 1, 20, 2, core);
    // Faint bookends.
    c.put(0, 1, edge);
    c.put(31, 1, edge);
    c.put(0, 2, edge);
    c.put(31, 2, edge);
    c.into_image()
}

fn build_flame_puff_image() -> Image {
    // Soft, irregular flame blob — bright core fading through orange to a
    // smoky outer rim.  Slight vertical asymmetry so it doesn't read as a
    // perfect circle when scaled up.
    let core: Rgba = [255, 250, 220, 255];
    let hot: Rgba = [255, 215, 120, 255];
    let mid: Rgba = [255, 150, 50, 255];
    let edge: Rgba = [220, 70, 25, 230];
    let smoke: Rgba = [110, 70, 40, 140];

    let w: i32 = 28;
    let h: i32 = 22;
    let mut c = Canvas::new(w, h);
    c.fill_rect(0, 0, w, h, [0, 0, 0, 0]);
    let cx = w / 2;
    let cy = h / 2;
    // Smoky outer edge
    c.fill_circle(cx, cy, 10, smoke);
    c.fill_circle(cx + 1, cy - 1, 8, edge);
    c.fill_circle(cx - 1, cy + 1, 7, edge);
    // Mid orange ring
    c.fill_circle(cx, cy, 6, mid);
    c.fill_circle(cx + 2, cy - 2, 4, hot);
    // Bright core (off-centre, biased forward toward +x — flames point
    // along the velocity direction, which the bullet rotates into).
    c.fill_circle(cx + 3, cy, 3, hot);
    c.fill_circle(cx + 4, cy, 2, core);
    // Few bright sparks scattered through.
    c.put(cx - 4, cy - 3, hot);
    c.put(cx + 6, cy + 3, core);
    c.put(cx - 2, cy + 4, mid);
    c.into_image()
}

fn build_shockwave_image() -> Image {
    // Hollow ring — bright on the rim, transparent inside.  Final scaling
    // is animated; this is just the silhouette.
    let outer: Rgba = [255, 220, 140, 255];
    let mid: Rgba = [255, 180, 80, 255];
    let inner: Rgba = [220, 110, 40, 180];

    let size = 64;
    let mut c = Canvas::new(size, size);
    c.fill_rect(0, 0, size, size, [0, 0, 0, 0]);
    let cx = size / 2;
    let cy = size / 2;
    for y in 0..size {
        for x in 0..size {
            let dx = (x - cx) as f32;
            let dy = (y - cy) as f32;
            let d = (dx * dx + dy * dy).sqrt();
            if d > 27.0 && d < 31.0 {
                c.put(x, y, outer);
            } else if d > 25.0 && d <= 27.0 {
                c.put(x, y, mid);
            } else if d > 22.0 && d <= 25.0 {
                c.put(x, y, inner);
            }
        }
    }
    c.into_image()
}

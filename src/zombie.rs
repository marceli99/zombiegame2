use bevy::prelude::*;
use rand::Rng;

use crate::bullet::{ExplodeEvent, EXPLODER_EXPLOSION_PLAYER_DAMAGE, EXPLODER_EXPLOSION_RADIUS, EXPLODER_EXPLOSION_ZOMBIE_DAMAGE};
use crate::map::{
    bfs_distance_field, in_bounds, nav_idx, spawn_point_world, tile_center, world_to_tile,
    MapObstacles, MapSegmentUnlockState, NavGrid, SpawnPointSpec, SPAWN_POINTS, TILE_SIZE,
};
use crate::zones::ZoneState;
use crate::net::{is_authoritative, NetContext, NetEntities, NetId};
use crate::pixelart::{Canvas, Rgba};
use crate::player::{DeadPlayers, Player, PlayerDamagedEvent, PlayerDiedEvent, PLAYER_RADIUS};
use crate::{gameplay_active, GameState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ZombieKind {
    #[default]
    Normal = 0,
    Fast = 1,
    Exploder = 2,
    Burning = 3,
    Giant = 4,
}

impl ZombieKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Fast,
            2 => Self::Exploder,
            3 => Self::Burning,
            4 => Self::Giant,
            _ => Self::Normal,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn base_hp(self) -> i32 {
        match self {
            Self::Normal => 5,
            Self::Fast => 3,
            Self::Exploder => 12,
            Self::Burning => 8,
            Self::Giant => 150,
        }
    }

    pub fn base_speed(self) -> f32 {
        match self {
            Self::Normal => 100.0,
            Self::Fast => 210.0,
            Self::Exploder => 70.0,
            Self::Burning => 130.0,
            Self::Giant => 45.0,
        }
    }

    pub fn radius(self) -> f32 {
        match self {
            Self::Normal => 10.0,
            Self::Fast => 9.0,
            Self::Exploder => 13.0,
            Self::Burning => 10.0,
            Self::Giant => 20.0,
        }
    }

    pub fn sprite_size(self) -> Vec2 {
        match self {
            Self::Normal => Vec2::new(32.0, 32.0),
            Self::Fast => Vec2::new(28.0, 28.0),
            Self::Exploder => Vec2::new(42.0, 42.0),
            Self::Burning => Vec2::new(32.0, 32.0),
            Self::Giant => Vec2::new(64.0, 64.0),
        }
    }

    pub fn contact_damage(self) -> i32 {
        match self {
            Self::Normal => 20,
            Self::Fast => 15,
            Self::Exploder => 0,
            Self::Burning => 10,
            Self::Giant => 40,
        }
    }

    pub fn kill_reward(self) -> u32 {
        match self {
            Self::Giant => 500,
            _ => 20,
        }
    }
}

#[derive(Component)]
pub struct Zombie {
    pub hp: i32,
    pub speed: f32,
    pub kind: ZombieKind,
    /// Decaying seconds of hit-reaction flash — sprite tints toward white
    /// while > 0.  Set on every damage instance, faded in
    /// `update_zombie_hit_flash`.
    pub hit_flash: f32,
    /// Counts down between blood drips when the zombie is wounded; when
    /// it hits zero a tiny blood splat is dropped at their feet.
    pub bleed_timer: f32,
}

pub const ZOMBIE_HIT_FLASH_DURATION: f32 = 0.12;

/// Floating damage number above a hit zombie — spawned at the bullet impact
/// point, drifts upward while fading.  Pure cosmetic.
#[derive(Component)]
pub struct DamageNumber {
    pub lifetime: f32,
    pub max_lifetime: f32,
    pub velocity: Vec2,
}

/// Merge bookkeeping for a hit-damage popup: further hits landing near
/// `spawn_pos` while the popup is still young are added into `total` (and
/// the glyphs restyled) instead of spawning another overlapping number.
/// Score popups reuse `DamageNumber` for their drift but deliberately don't
/// carry this, so damage is never folded into a "+$N" label.
#[derive(Component)]
pub struct DamageNumberAccum {
    pub total: i32,
    pub spawn_pos: Vec2,
}

/// Hits within this radius of a popup's spawn point count as "the same
/// zombie" for merging.
const DAMAGE_MERGE_RADIUS_SQ: f32 = 24.0 * 24.0;
/// A popup only absorbs new hits during its first fraction of a second —
/// after that a fresh number spawns, so sustained fire reads as a slow
/// series of climbing totals instead of 20-80 glyph spawns per second.
const DAMAGE_MERGE_WINDOW: f32 = 0.3;

/// Event raised on every successful hit so the FX system can spawn a
/// floating number without each damage site needing to know about UI.
#[derive(Event)]
pub struct DamageNumberEvent {
    pub pos: Vec2,
    pub amount: i32,
}

#[derive(Event)]
pub struct ZombieKilledEvent {
    pub kind: ZombieKind,
    pub by_explosion: bool,
    /// World-space position where the zombie died — used by the blood-stain
    /// FX (and is otherwise ignored by score / achievement listeners).
    pub pos: Vec2,
}

/// Fading blood splat dropped at a zombie's last position.  Pure cosmetic
/// — no obstacle, no replication.  Lifetime ticks down in
/// `update_blood_stains` and the entity self-despawns when it hits zero.
#[derive(Component)]
pub struct BloodStain {
    pub lifetime: f32,
    pub max_lifetime: f32,
}

#[derive(Resource)]
pub struct BloodAssets {
    pub stains: [Handle<Image>; 3],
}

#[derive(Event, Default)]
pub struct SpawnZombieEvent {
    pub kind: ZombieKind,
}

pub const BURN_DURATION: f32 = 10.0;
pub const BURN_DPS: f32 = 3.5;

const GIANT_ATTACK_RANGE: f32 = 120.0;
const GIANT_ATTACK_COOLDOWN: f32 = 4.0;
const TOXIC_CLOUD_RADIUS: f32 = 40.0;
const TOXIC_CLOUD_LIFETIME: f32 = 3.0;
const TOXIC_CLOUD_DPS: f32 = 8.0;
const GIANT_HP_BAR_WIDTH: f32 = 50.0;

#[derive(Component)]
pub struct BurnEffect {
    pub remaining: f32,
    pub accumulated: f32,
}

#[derive(Component)]
pub struct GiantAttack {
    pub cooldown: f32,
}

#[derive(Component)]
pub struct ToxicCloud {
    pub lifetime: f32,
    pub tick: f32,
}

#[derive(Component)]
pub struct ZombieHpBar;

#[derive(Resource)]
pub struct ZombieAssets {
    pub normal: Handle<Image>,
    pub fast: Handle<Image>,
    pub exploder: Handle<Image>,
    pub burning: Handle<Image>,
    pub giant: Handle<Image>,
    pub toxic_cloud: Handle<Image>,
}

impl ZombieAssets {
    pub fn image_for(&self, kind: ZombieKind) -> Handle<Image> {
        match kind {
            ZombieKind::Normal => self.normal.clone(),
            ZombieKind::Fast => self.fast.clone(),
            ZombieKind::Exploder => self.exploder.clone(),
            ZombieKind::Burning => self.burning.clone(),
            ZombieKind::Giant => self.giant.clone(),
        }
    }
}

pub struct ZombiePlugin;

impl Plugin for ZombiePlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<SpawnZombieEvent>()
            .add_event::<ZombieKilledEvent>()
            .add_event::<DamageNumberEvent>()
            .add_systems(Startup, (setup_zombie_assets, setup_blood_assets))
            .add_systems(OnExit(GameState::Playing), despawn_all_zombies)
            .add_systems(
                FixedUpdate,
                (
                    spawn_zombie_listener,
                    // Nav concern, but chained here: the obstacle overlay must
                    // land before the BFS rebuild that consumes it.
                    crate::map::refresh_nav_blocked,
                    update_nav_flow,
                    zombie_movement,
                    zombie_attack,
                    giant_toxic_attack,
                    toxic_cloud_tick,
                    burn_tick_system,
                )
                    .chain()
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            )
            .add_systems(
                Update,
                (
                    update_zombie_hp_bars,
                    spawn_blood_on_kill,
                    spawn_blood_on_player_death,
                    update_blood_stains,
                    update_zombie_hit_flash,
                    spawn_damage_numbers,
                    update_damage_numbers,
                    spawn_score_popups,
                    drip_wounded_blood,
                )
                    // Cosmetic / feedback only — freeze on a singleplayer pause
                    // (no blood/number/flash churn while paused); live in MP.
                    .run_if(gameplay_active),
            );
    }
}

/// Wounded zombies drip small blood drops behind them on the ground.  The
/// drip rate is tied to the per-zombie `bleed_timer` field which is reset
/// to a randomised cooldown after every drop.
fn drip_wounded_blood(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<BloodAssets>,
    mut q: Query<(&Transform, &mut Zombie)>,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let dt = time.delta_seconds();
    for (t, mut zombie) in &mut q {
        if zombie.hp <= 0 {
            continue;
        }
        let max_hp = zombie.kind.base_hp();
        // Only drip while wounded — under 60% HP.
        if zombie.hp * 100 / max_hp.max(1) > 60 {
            continue;
        }
        zombie.bleed_timer -= dt;
        if zombie.bleed_timer > 0.0 {
            continue;
        }
        // Faster drip rate the more wounded the zombie is, scaled by HP.
        let severity = 1.0 - (zombie.hp as f32 / max_hp as f32).clamp(0.0, 1.0);
        zombie.bleed_timer = (1.4 - severity * 0.9).max(0.35) + rng.gen_range(-0.1..0.1);

        let pos = t.translation.truncate()
            + Vec2::new(rng.gen_range(-4.0..4.0), rng.gen_range(-6.0..2.0));
        let size = rng.gen_range(8.0..14.0);
        let life = rng.gen_range(2.5..4.0);
        let variant = rng.gen_range(0..3);
        commands.spawn((
            SpriteBundle {
                texture: assets.stains[variant].clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(size)),
                    color: Color::srgba(1.0, 1.0, 1.0, 0.85),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, -12.7)
                    .with_rotation(Quat::from_rotation_z(
                        rng.gen_range(-std::f32::consts::PI..std::f32::consts::PI),
                    )),
                ..default()
            },
            BloodStain {
                lifetime: life,
                max_lifetime: life,
            },
        ));
    }
}

/// Spawns a green floating "+$N" label whenever a zombie dies, where N is
/// the kill reward.  Uses the same `DamageNumber` lifetime + drift system
/// for free animation, just with a green tint and a slightly longer life.
fn spawn_score_popups(
    mut commands: Commands,
    mut events: EventReader<ZombieKilledEvent>,
    ui: Res<crate::UiAssets>,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    for ev in events.read() {
        let reward = ev.kind.kill_reward();
        let lifetime = 1.1;
        // Bigger / brighter for the Giant since they pay out 25× normal.
        let (font_size, color) = if reward >= 100 {
            (22.0, Color::srgba(1.0, 0.95, 0.55, 1.0))
        } else {
            (15.0, Color::srgba(0.55, 0.95, 0.5, 1.0))
        };
        commands.spawn((
            Text2dBundle {
                text: Text::from_section(
                    format!("+${}", reward),
                    TextStyle {
                        font: ui.font.clone(),
                        font_size,
                        color,
                    },
                ),
                transform: Transform::from_xyz(
                    ev.pos.x + rng.gen_range(-6.0..6.0),
                    ev.pos.y + 26.0,
                    9.85,
                ),
                ..default()
            },
            DamageNumber {
                lifetime,
                max_lifetime: lifetime,
                velocity: Vec2::new(rng.gen_range(-18.0..18.0), 50.0),
            },
        ));
    }
}

/// Font size + tint tier for a damage total — bigger / brighter numbers for
/// big damage, readable from far away.
fn damage_number_style(amount: i32) -> (f32, Color) {
    if amount >= 18 {
        (18.0, Color::srgba(1.0, 0.85, 0.30, 1.0))
    } else if amount >= 6 {
        (14.0, Color::srgba(1.0, 0.95, 0.55, 1.0))
    } else {
        (12.0, Color::srgba(1.0, 1.0, 0.85, 0.95))
    }
}

fn spawn_damage_numbers(
    mut commands: Commands,
    mut events: EventReader<DamageNumberEvent>,
    ui: Res<crate::UiAssets>,
    mut live: Query<(&DamageNumber, &mut DamageNumberAccum, &mut Text)>,
) {
    use rand::Rng;
    // Rapid-fire weapons land 20-80 hits/s; a Text2d spawn per hit is both
    // unreadable glyph soup and real layout/entity churn on mobile.
    // Coalesce instead: merge this frame's events by proximity first (a
    // shotgun blast puts up to 8 pellets on one zombie in a single tick),
    // then fold each merged hit into a nearby still-fresh popup so
    // sustained fire updates one climbing number per zombie.
    let mut clusters: Vec<(Vec2, i32)> = Vec::new();
    for ev in events.read() {
        if ev.amount <= 0 {
            continue;
        }
        match clusters
            .iter_mut()
            .find(|(p, _)| p.distance_squared(ev.pos) < DAMAGE_MERGE_RADIUS_SQ)
        {
            Some((_, total)) => *total += ev.amount,
            None => clusters.push((ev.pos, ev.amount)),
        }
    }
    if clusters.is_empty() {
        return;
    }
    let mut rng = rand::thread_rng();
    'clusters: for (pos, amount) in clusters {
        for (dn, mut accum, mut text) in &mut live {
            let age = dn.max_lifetime - dn.lifetime;
            if age > DAMAGE_MERGE_WINDOW
                || accum.spawn_pos.distance_squared(pos) >= DAMAGE_MERGE_RADIUS_SQ
            {
                continue;
            }
            accum.total += amount;
            let (font_size, color) = damage_number_style(accum.total);
            if let Some(sec) = text.sections.first_mut() {
                sec.value = format!("{}", accum.total);
                sec.style.font_size = font_size;
                sec.style.color = color;
            }
            continue 'clusters;
        }
        let (font_size, color) = damage_number_style(amount);
        let lifetime = 0.7;
        let jitter = Vec2::new(rng.gen_range(-6.0..6.0), rng.gen_range(0.0..6.0));
        commands.spawn((
            Text2dBundle {
                text: Text::from_section(
                    format!("{}", amount),
                    TextStyle {
                        font: ui.font.clone(),
                        font_size,
                        color,
                    },
                ),
                transform: Transform::from_xyz(pos.x + jitter.x, pos.y + 14.0 + jitter.y, 9.8),
                ..default()
            },
            DamageNumber {
                lifetime,
                max_lifetime: lifetime,
                velocity: Vec2::new(rng.gen_range(-30.0..30.0), 60.0),
            },
            DamageNumberAccum {
                total: amount,
                spawn_pos: pos,
            },
        ));
    }
}

fn update_damage_numbers(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut DamageNumber, &mut Transform, &mut Text)>,
) {
    let dt = time.delta_seconds();
    for (e, mut dn, mut transform, mut text) in &mut q {
        dn.lifetime -= dt;
        if dn.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            continue;
        }
        // Drift up + decelerate so the numbers settle at the apex.
        dn.velocity *= 1.0 - 1.5 * dt;
        transform.translation += (dn.velocity * dt).extend(0.0);
        let pct = (dn.lifetime / dn.max_lifetime).clamp(0.0, 1.0);
        for sec in &mut text.sections {
            sec.style.color.set_alpha(pct.powf(1.2));
        }
    }
}

/// Drops a chunky blood pool plus several scattered drops at the spot where
/// a player died.  Bigger and longer-lasting than the zombie blood splats.
fn spawn_blood_on_player_death(
    mut commands: Commands,
    assets: Res<BloodAssets>,
    mut events: EventReader<PlayerDiedEvent>,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    for ev in events.read() {
        // Big central pool — generous lifetime so the body's mark persists
        // through a respawn cycle.
        let main_size = 80.0;
        let main_life = 12.0;
        commands.spawn((
            SpriteBundle {
                texture: assets.stains[1].clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(main_size)),
                    color: Color::srgba(1.0, 1.0, 1.0, 0.95),
                    ..default()
                },
                transform: Transform::from_xyz(ev.pos.x, ev.pos.y, -12.4)
                    .with_rotation(Quat::from_rotation_z(
                        rng.gen_range(-std::f32::consts::PI..std::f32::consts::PI),
                    )),
                ..default()
            },
            BloodStain {
                lifetime: main_life,
                max_lifetime: main_life,
            },
        ));
        // Scatter 6 smaller drops in a ~80 px radius around the body.
        for _ in 0..6 {
            let angle: f32 = rng.gen_range(0.0..std::f32::consts::TAU);
            let dist = rng.gen_range(28.0..70.0);
            let pos = ev.pos + Vec2::new(angle.cos(), angle.sin()) * dist;
            let size = rng.gen_range(20.0..38.0);
            let variant = rng.gen_range(0..3);
            let life = rng.gen_range(8.0..11.0);
            commands.spawn((
                SpriteBundle {
                    texture: assets.stains[variant].clone(),
                    sprite: Sprite {
                        custom_size: Some(Vec2::splat(size)),
                        color: Color::srgba(1.0, 1.0, 1.0, 0.9),
                        ..default()
                    },
                    transform: Transform::from_xyz(pos.x, pos.y, -12.45)
                        .with_rotation(Quat::from_rotation_z(
                            rng.gen_range(-std::f32::consts::PI..std::f32::consts::PI),
                        )),
                    ..default()
                },
                BloodStain {
                    lifetime: life,
                    max_lifetime: life,
                },
            ));
        }
    }
}

/// Tints damaged zombies briefly toward white so the hit reads visually
/// even when the HP bar isn't visible.  The `hit_flash` field is bumped
/// at every damage site in `bullet.rs`; we fade it back to 0 here.
fn update_zombie_hit_flash(
    time: Res<Time>,
    mut q: Query<(&mut Zombie, &mut Sprite)>,
) {
    let dt = time.delta_seconds();
    for (mut zombie, mut sprite) in &mut q {
        if zombie.hit_flash <= 0.0 {
            // Reset to default tint once the flash is over.  Using
            // `Color::WHITE` keeps the texture's own colours intact.
            if sprite.color != Color::WHITE {
                sprite.color = Color::WHITE;
            }
            continue;
        }
        zombie.hit_flash = (zombie.hit_flash - dt).max(0.0);
        let pct = (zombie.hit_flash / ZOMBIE_HIT_FLASH_DURATION).clamp(0.0, 1.0);
        // Lerp toward bright over-saturated white.  Stronger at the start
        // of the flash, fading back to white (default).
        let mix = pct;
        let r = 1.0 + mix * 1.4;
        let g = 1.0 + mix * 1.4;
        let b = 1.0 + mix * 1.4;
        sprite.color = Color::srgba(r, g, b, 1.0);
    }
}

fn setup_blood_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(BloodAssets {
        stains: [
            images.add(build_blood_stain_image(0)),
            images.add(build_blood_stain_image(1)),
            images.add(build_blood_stain_image(2)),
        ],
    });
}

const BLOOD_STAIN_LIFETIME: f32 = 5.0;

fn spawn_blood_on_kill(
    mut commands: Commands,
    assets: Res<BloodAssets>,
    mut events: EventReader<ZombieKilledEvent>,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    for ev in events.read() {
        // Bigger splat for explosion / giant deaths so the chunkier kills
        // read appropriately gory.
        let base_size = if ev.by_explosion {
            42.0
        } else {
            match ev.kind {
                ZombieKind::Giant => 60.0,
                ZombieKind::Exploder => 44.0,
                _ => 30.0,
            }
        };
        let scatter = rng.gen_range(0.85..1.15);
        let jitter = Vec2::new(rng.gen_range(-3.0..3.0), rng.gen_range(-3.0..3.0));
        let variant = rng.gen_range(0..3);
        let rot = rng.gen_range(-std::f32::consts::PI..std::f32::consts::PI);
        commands.spawn((
            SpriteBundle {
                texture: assets.stains[variant].clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(base_size * scatter)),
                    color: Color::srgba(1.0, 1.0, 1.0, 0.95),
                    ..default()
                },
                transform: Transform::from_xyz(ev.pos.x + jitter.x, ev.pos.y + jitter.y, -12.5)
                    .with_rotation(Quat::from_rotation_z(rot)),
                ..default()
            },
            BloodStain {
                lifetime: BLOOD_STAIN_LIFETIME,
                max_lifetime: BLOOD_STAIN_LIFETIME,
            },
        ));
    }
}

fn update_blood_stains(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut BloodStain, &mut Sprite)>,
) {
    let dt = time.delta_seconds();
    for (e, mut stain, mut sprite) in &mut q {
        stain.lifetime -= dt;
        if stain.lifetime <= 0.0 {
            commands.entity(e).despawn_recursive();
            continue;
        }
        let pct = (stain.lifetime / stain.max_lifetime).clamp(0.0, 1.0);
        // Hold full alpha for the first ~30%, then fade out smoothly.
        let alpha = if pct > 0.7 { 0.95 } else { (pct / 0.7) * 0.95 };
        sprite.color.set_alpha(alpha);
    }
}

fn build_blood_stain_image(variant: u32) -> Image {
    let outline: Rgba = [40, 6, 6, 255];
    let dark: Rgba = [110, 14, 14, 255];
    let mid: Rgba = [170, 22, 22, 255];
    let bright: Rgba = [210, 36, 36, 255];

    let size: i32 = 32;
    let mut c = Canvas::new(size, size);
    c.fill_rect(0, 0, size, size, [0, 0, 0, 0]);
    let cx = size / 2;
    let cy = size / 2;
    // Main pool — irregular blob.
    c.fill_circle(cx, cy, 9, outline);
    c.fill_circle(cx, cy, 8, dark);
    c.fill_circle(cx - 1, cy - 1, 6, mid);
    c.fill_circle(cx - 2, cy - 2, 3, bright);
    // Splatter droplets — pattern varies by `variant` so successive deaths
    // don't drop identical blobs on top of each other.
    let droplets: &[(i32, i32, i32)] = match variant {
        0 => &[
            (cx + 11, cy - 4, 2),
            (cx + 8, cy + 9, 2),
            (cx - 12, cy + 2, 2),
            (cx - 6, cy + 12, 1),
            (cx + 5, cy - 12, 1),
        ],
        1 => &[
            (cx + 13, cy + 3, 2),
            (cx - 11, cy - 5, 2),
            (cx - 4, cy - 13, 1),
            (cx + 9, cy + 11, 1),
            (cx - 9, cy + 9, 2),
        ],
        _ => &[
            (cx - 13, cy + 1, 2),
            (cx + 12, cy - 6, 2),
            (cx + 4, cy + 13, 1),
            (cx - 7, cy - 11, 1),
            (cx + 11, cy + 8, 1),
            (cx - 11, cy - 9, 1),
        ],
    };
    for &(x, y, r) in droplets {
        if (0..size).contains(&x) && (0..size).contains(&y) {
            c.fill_circle(x, y, r, dark);
            c.fill_circle(x, y, r.saturating_sub(1), mid);
        }
    }
    c.into_image()
}

fn setup_zombie_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(ZombieAssets {
        normal: images.add(build_normal_zombie_image()),
        fast: images.add(build_fast_zombie_image()),
        exploder: images.add(build_exploder_zombie_image()),
        burning: images.add(build_burning_zombie_image()),
        giant: images.add(build_giant_zombie_image()),
        toxic_cloud: images.add(build_toxic_cloud_image()),
    });
}

fn build_normal_zombie_image() -> Image {
    let outline: Rgba = [8, 14, 6, 255];
    let body_main: Rgba = [72, 115, 42, 255];
    let body_light: Rgba = [98, 142, 58, 255];
    let body_dark: Rgba = [45, 72, 26, 255];
    let flesh: Rgba = [110, 148, 58, 255];
    let flesh_dark: Rgba = [68, 96, 38, 255];
    let shirt: Rgba = [68, 42, 22, 255];
    let shirt_dark: Rgba = [38, 22, 10, 255];
    let shirt_torn: Rgba = [52, 32, 14, 255];
    let eye: Rgba = [255, 40, 20, 255];
    let eye_glow: Rgba = [255, 80, 40, 255];
    let wound: Rgba = [120, 14, 14, 255];
    let wound_dark: Rgba = [72, 8, 8, 255];
    let bone: Rgba = [200, 195, 170, 255];
    let claw: Rgba = [180, 175, 150, 255];
    let teeth: Rgba = [210, 200, 170, 255];

    let mut c = Canvas::new(25, 25);

    // Body - hunched shape
    c.fill_circle(10, 12, 8, outline);
    c.fill_circle(10, 12, 7, body_dark);
    c.fill_circle(10, 12, 6, body_main);
    c.fill_circle(8, 10, 2, body_light);

    // Torn shirt/clothing
    c.fill_rect(7, 8, 6, 9, shirt_dark);
    c.fill_rect(8, 9, 4, 7, shirt);
    c.put(8, 10, shirt_torn);
    c.put(11, 13, shirt_torn);
    c.put(9, 15, shirt_dark);
    // Shirt tears revealing flesh
    c.put(9, 9, flesh);
    c.put(10, 12, flesh_dark);
    c.put(8, 14, flesh);

    // Wounds
    c.fill_rect(12, 7, 3, 2, wound);
    c.put(12, 7, wound_dark);
    c.put(14, 8, wound_dark);
    c.put(7, 15, wound);
    c.put(10, 16, wound_dark);
    // Exposed bone
    c.put(13, 7, bone);

    // Upper arm reaching forward + claws
    c.fill_rect(11, 4, 9, 3, outline);
    c.fill_rect(12, 5, 7, 1, flesh);
    c.fill_rect(12, 6, 7, 1, flesh_dark);
    c.fill_rect(18, 3, 3, 4, outline);
    c.put(19, 4, claw);
    c.put(20, 4, claw);
    c.put(19, 5, flesh);
    c.put(20, 5, claw);

    // Lower arm + claws
    c.fill_rect(11, 18, 9, 3, outline);
    c.fill_rect(12, 19, 7, 1, flesh);
    c.fill_rect(12, 20, 7, 1, flesh_dark);
    c.fill_rect(18, 18, 3, 4, outline);
    c.put(19, 19, claw);
    c.put(20, 19, claw);
    c.put(19, 20, flesh);
    c.put(20, 20, claw);

    // Head
    c.fill_circle(15, 12, 4, outline);
    c.fill_circle(15, 12, 3, flesh);
    c.fill_circle(14, 11, 1, body_light);
    // Rotting patches
    c.put(13, 11, flesh_dark);
    c.put(16, 14, wound);

    // Eyes - glowing
    c.put(17, 11, eye);
    c.put(17, 13, eye);
    c.put(18, 11, eye_glow);
    c.put(18, 13, eye_glow);
    // Mouth
    c.put(17, 12, outline);
    c.put(18, 12, teeth);

    c.into_image()
}

fn build_fast_zombie_image() -> Image {
    let outline: Rgba = [6, 4, 4, 255];
    let body_main: Rgba = [145, 152, 128, 255];
    let body_light: Rgba = [180, 186, 160, 255];
    let body_dark: Rgba = [78, 84, 66, 255];
    let flesh: Rgba = [160, 168, 138, 255];
    let flesh_dark: Rgba = [100, 108, 84, 255];
    let rag: Rgba = [48, 32, 18, 255];
    let rag_dark: Rgba = [28, 16, 8, 255];
    let eye: Rgba = [255, 220, 40, 255];
    let eye_glow: Rgba = [255, 240, 80, 255];
    let blood: Rgba = [150, 16, 12, 255];
    let blood_dark: Rgba = [90, 8, 6, 255];
    let claw: Rgba = [240, 238, 210, 255];
    let claw_dark: Rgba = [200, 198, 170, 255];
    let bone: Rgba = [210, 205, 180, 255];
    let scar: Rgba = [120, 80, 70, 255];

    let mut c = Canvas::new(25, 25);

    // Lean body
    c.fill_circle(11, 12, 6, outline);
    c.fill_circle(11, 12, 5, body_dark);
    c.fill_circle(11, 12, 4, body_main);
    c.fill_circle(9, 10, 1, body_light);

    // Torn rags
    c.fill_rect(9, 9, 4, 6, rag_dark);
    c.fill_rect(10, 10, 2, 4, rag);
    c.put(9, 11, flesh);
    c.put(12, 13, flesh_dark);
    c.put(10, 14, blood);
    c.put(11, 9, flesh);

    // Ribs/bones showing through
    c.put(9, 12, bone);
    c.put(9, 13, bone);
    c.put(12, 10, bone);
    c.put(10, 15, scar);

    // Upper arm + long claws
    c.fill_rect(12, 4, 8, 2, outline);
    c.fill_rect(13, 5, 6, 1, flesh);
    c.put(13, 4, flesh_dark);
    c.fill_rect(19, 3, 4, 3, outline);
    c.put(20, 4, claw);
    c.put(21, 4, claw);
    c.put(22, 4, claw);
    c.put(20, 3, claw_dark);
    c.put(21, 5, claw_dark);

    // Lower arm + long claws
    c.fill_rect(12, 19, 8, 2, outline);
    c.fill_rect(13, 19, 6, 1, flesh);
    c.put(13, 20, flesh_dark);
    c.fill_rect(19, 19, 4, 3, outline);
    c.put(20, 20, claw);
    c.put(21, 20, claw);
    c.put(22, 20, claw);
    c.put(20, 21, claw_dark);
    c.put(21, 19, claw_dark);

    // Head - gaunt
    c.fill_circle(15, 12, 3, outline);
    c.fill_circle(15, 12, 2, flesh);
    c.put(14, 11, body_light);
    // Sunken features
    c.put(13, 12, flesh_dark);
    c.put(16, 13, scar);

    // Glowing eyes
    c.put(17, 11, eye);
    c.put(17, 13, eye);
    c.put(16, 11, eye_glow);
    c.put(16, 13, eye_glow);
    // Bloody mouth
    c.put(17, 12, blood);
    c.put(16, 12, blood_dark);

    c.into_image()
}

fn build_exploder_zombie_image() -> Image {
    let outline: Rgba = [12, 6, 2, 255];
    let body_main: Rgba = [150, 62, 32, 255];
    let body_light: Rgba = [190, 95, 45, 255];
    let body_dark: Rgba = [82, 30, 14, 255];
    let belly: Rgba = [230, 155, 40, 255];
    let belly_hot: Rgba = [255, 210, 70, 255];
    let belly_core: Rgba = [255, 240, 140, 255];
    let vein: Rgba = [200, 60, 20, 255];
    let vein_dark: Rgba = [140, 30, 10, 255];
    let head_main: Rgba = [168, 82, 40, 255];
    let head_light: Rgba = [200, 120, 52, 255];
    let head_dark: Rgba = [100, 44, 20, 255];
    let eye: Rgba = [255, 230, 100, 255];
    let eye_glow: Rgba = [255, 255, 180, 255];
    let crack: Rgba = [255, 100, 20, 255];
    let crack_hot: Rgba = [255, 180, 60, 255];
    let arm: Rgba = [140, 58, 26, 255];
    let arm_dark: Rgba = [90, 36, 14, 255];
    let blister: Rgba = [210, 130, 50, 255];

    let mut c = Canvas::new(31, 31);

    // Bloated body
    c.fill_circle(14, 15, 12, outline);
    c.fill_circle(14, 15, 11, body_dark);
    c.fill_circle(14, 15, 10, body_main);
    c.fill_circle(12, 12, 5, body_light);

    // Veins spreading from belly
    c.put(7, 12, vein_dark);
    c.put(8, 11, vein);
    c.put(9, 10, vein);
    c.put(6, 15, vein_dark);
    c.put(7, 16, vein);
    c.put(20, 10, vein_dark);
    c.put(19, 11, vein);
    c.put(20, 18, vein_dark);
    c.put(19, 19, vein);
    c.put(10, 20, vein);
    c.put(9, 21, vein_dark);
    c.put(16, 21, vein);
    c.put(17, 22, vein_dark);

    // Blisters on skin
    c.put(8, 9, blister);
    c.put(18, 20, blister);
    c.put(6, 17, blister);
    c.put(20, 12, blister);

    // Glowing belly
    c.fill_circle(14, 15, 7, outline);
    c.fill_circle(14, 15, 6, vein_dark);
    c.fill_circle(14, 15, 5, belly);
    c.fill_circle(14, 15, 3, belly_hot);
    c.fill_circle(14, 15, 1, belly_core);

    // Cracks radiating from center
    c.put(14, 15, crack_hot);
    c.put(13, 14, crack);
    c.put(15, 16, crack);
    c.put(12, 16, crack);
    c.put(16, 14, crack);
    c.put(11, 13, crack);
    c.put(17, 17, crack);
    c.put(12, 18, crack);
    c.put(16, 12, crack);
    c.put(10, 15, crack_hot);
    c.put(18, 15, crack_hot);
    c.put(14, 10, crack_hot);
    c.put(14, 20, crack_hot);

    // Upper arm (stubby)
    c.fill_rect(14, 4, 9, 3, outline);
    c.fill_rect(15, 5, 7, 1, arm);
    c.fill_rect(15, 6, 7, 1, arm_dark);
    c.fill_rect(21, 3, 3, 4, outline);
    c.put(22, 4, body_light);
    c.put(22, 5, arm);

    // Lower arm (stubby)
    c.fill_rect(14, 24, 9, 3, outline);
    c.fill_rect(15, 25, 7, 1, arm);
    c.fill_rect(15, 26, 7, 1, arm_dark);
    c.fill_rect(21, 24, 3, 4, outline);
    c.put(22, 25, body_light);
    c.put(22, 26, arm);

    // Head - small relative to body
    c.fill_circle(19, 15, 5, outline);
    c.fill_circle(19, 15, 4, head_dark);
    c.fill_circle(19, 15, 3, head_main);
    c.fill_circle(18, 13, 1, head_light);

    // Eyes - intense glow
    c.put(22, 13, eye_glow);
    c.put(22, 16, eye_glow);
    c.put(21, 13, eye);
    c.put(21, 16, eye);
    // Cracked face
    c.put(21, 15, crack);
    c.put(23, 14, crack);
    c.put(20, 14, head_dark);

    c.into_image()
}

fn build_burning_zombie_image() -> Image {
    let outline: Rgba = [20, 8, 2, 255];
    let body_main: Rgba = [160, 50, 20, 255];
    let body_dark: Rgba = [100, 30, 10, 255];
    let body_light: Rgba = [200, 80, 30, 255];
    let flame1: Rgba = [255, 160, 20, 255];
    let flame2: Rgba = [255, 100, 10, 255];
    let flame3: Rgba = [255, 220, 50, 255];
    let flame_tip: Rgba = [255, 240, 120, 255];
    let eye: Rgba = [255, 255, 100, 255];
    let eye_glow: Rgba = [255, 255, 200, 255];
    let char_skin: Rgba = [60, 20, 8, 255];
    let ember: Rgba = [255, 120, 30, 255];
    let claw: Rgba = [80, 40, 15, 255];

    let mut c = Canvas::new(25, 25);

    // Flame aura
    c.fill_circle(12, 12, 11, flame2);
    c.fill_circle(12, 12, 9, flame1);
    c.put(12, 1, flame_tip);
    c.put(10, 2, flame3);
    c.put(14, 2, flame3);
    c.put(8, 3, flame_tip);
    c.put(16, 3, flame_tip);
    c.put(3, 8, flame3);
    c.put(21, 8, flame3);
    c.put(2, 12, flame_tip);
    c.put(22, 12, flame_tip);

    // Body
    c.fill_circle(12, 12, 7, outline);
    c.fill_circle(12, 12, 6, body_dark);
    c.fill_circle(12, 12, 5, body_main);
    c.fill_circle(10, 10, 2, body_light);

    // Charred patches
    c.put(9, 14, char_skin);
    c.put(14, 10, char_skin);
    c.put(11, 16, char_skin);

    // Embers
    c.put(6, 5, ember);
    c.put(18, 6, ember);
    c.put(5, 16, ember);
    c.put(19, 15, ember);

    // Upper arm + claws
    c.fill_rect(13, 4, 8, 3, outline);
    c.fill_rect(14, 5, 6, 1, body_main);
    c.fill_rect(14, 6, 6, 1, body_dark);
    c.fill_rect(19, 3, 3, 4, outline);
    c.put(20, 4, claw);
    c.put(21, 4, claw);
    c.put(20, 5, body_main);

    // Lower arm + claws
    c.fill_rect(13, 18, 8, 3, outline);
    c.fill_rect(14, 19, 6, 1, body_main);
    c.fill_rect(14, 20, 6, 1, body_dark);
    c.fill_rect(19, 18, 3, 4, outline);
    c.put(20, 19, claw);
    c.put(21, 19, claw);
    c.put(20, 20, body_main);

    // Head
    c.fill_circle(16, 12, 3, outline);
    c.fill_circle(16, 12, 2, body_main);
    c.put(15, 11, body_light);

    // Eyes - bright fire
    c.put(18, 11, eye_glow);
    c.put(18, 13, eye_glow);
    c.put(17, 11, eye);
    c.put(17, 13, eye);

    // Flame on head
    c.put(16, 8, flame3);
    c.put(15, 9, flame1);
    c.put(17, 9, flame1);

    c.into_image()
}

fn build_giant_zombie_image() -> Image {
    let outline: Rgba = [6, 10, 4, 255];
    let body_main: Rgba = [55, 85, 35, 255];
    let body_dark: Rgba = [32, 52, 20, 255];
    let body_light: Rgba = [78, 115, 50, 255];
    let flesh: Rgba = [90, 120, 55, 255];
    let flesh_dark: Rgba = [50, 70, 30, 255];
    let scar: Rgba = [110, 40, 30, 255];
    let eye: Rgba = [255, 50, 20, 255];
    let eye_glow: Rgba = [255, 90, 40, 255];
    let teeth: Rgba = [200, 195, 165, 255];
    let claw: Rgba = [160, 155, 130, 255];
    let rag: Rgba = [40, 30, 18, 255];
    let rag_dark: Rgba = [24, 16, 8, 255];
    let wound: Rgba = [100, 20, 15, 255];
    let chain: Rgba = [120, 118, 110, 255];
    let bone: Rgba = [190, 185, 160, 255];

    let mut c = Canvas::new(48, 48);

    // Massive body
    c.fill_circle(22, 24, 18, outline);
    c.fill_circle(22, 24, 17, body_dark);
    c.fill_circle(22, 24, 15, body_main);
    c.fill_circle(18, 20, 8, body_light);

    // Scars
    c.fill_rect(14, 18, 4, 2, scar);
    c.fill_rect(26, 28, 3, 3, wound);
    c.put(12, 30, scar);
    c.put(30, 22, scar);

    // Torn rags
    c.fill_rect(12, 16, 18, 14, rag_dark);
    c.fill_rect(14, 18, 14, 10, rag);
    c.put(15, 20, flesh);
    c.put(20, 25, flesh_dark);
    c.put(25, 22, flesh);

    // Ribs
    c.put(18, 22, bone);
    c.put(18, 24, bone);
    c.put(18, 26, bone);

    // Chains
    c.put(10, 20, chain);
    c.put(11, 21, chain);
    c.put(32, 20, chain);
    c.put(33, 21, chain);

    // Upper arm
    c.fill_rect(22, 6, 14, 6, outline);
    c.fill_rect(23, 7, 12, 4, body_dark);
    c.fill_rect(24, 8, 10, 2, body_main);
    c.fill_rect(35, 5, 6, 7, outline);
    c.fill_rect(36, 6, 4, 5, flesh);
    c.put(39, 6, claw);
    c.put(40, 6, claw);
    c.put(39, 9, claw);
    c.put(40, 9, claw);

    // Lower arm
    c.fill_rect(22, 36, 14, 6, outline);
    c.fill_rect(23, 37, 12, 4, body_dark);
    c.fill_rect(24, 38, 10, 2, body_main);
    c.fill_rect(35, 36, 6, 7, outline);
    c.fill_rect(36, 37, 4, 5, flesh);
    c.put(39, 37, claw);
    c.put(40, 37, claw);
    c.put(39, 40, claw);
    c.put(40, 40, claw);

    // Head (small relative to body)
    c.fill_circle(30, 24, 7, outline);
    c.fill_circle(30, 24, 6, body_dark);
    c.fill_circle(30, 24, 5, flesh);
    c.fill_circle(28, 22, 2, body_light);

    // Brow
    c.fill_rect(32, 20, 6, 2, outline);
    c.fill_rect(33, 20, 4, 1, flesh_dark);

    // Eyes
    c.put(34, 22, eye_glow);
    c.put(34, 26, eye_glow);
    c.put(35, 22, eye);
    c.put(35, 26, eye);

    // Mouth
    c.fill_rect(34, 23, 3, 2, outline);
    c.put(35, 23, teeth);
    c.put(35, 24, teeth);
    c.put(34, 24, wound);

    c.into_image()
}

fn build_toxic_cloud_image() -> Image {
    let outer: Rgba = [60, 90, 30, 60];
    let mid: Rgba = [50, 110, 25, 120];
    let inner: Rgba = [40, 130, 20, 180];
    let core: Rgba = [30, 150, 15, 200];

    let mut c = Canvas::new(16, 16);
    c.fill_circle(8, 8, 7, outer);
    c.fill_circle(8, 8, 5, mid);
    c.fill_circle(6, 7, 3, inner);
    c.fill_circle(10, 9, 3, inner);
    c.fill_circle(8, 8, 2, core);
    c.into_image()
}

pub fn spawn_zombie_entity(
    commands: &mut Commands,
    assets: &ZombieAssets,
    pos: Vec2,
    net_id: u32,
    hp: i32,
    speed: f32,
    kind: ZombieKind,
) -> Entity {
    let entity = commands
        .spawn((
            SpriteBundle {
                texture: assets.image_for(kind),
                sprite: Sprite {
                    custom_size: Some(kind.sprite_size()),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 5.0),
                ..default()
            },
            Zombie {
                hp,
                speed,
                kind,
                hit_flash: 0.0,
                bleed_timer: 0.0,
            },
            ZombieNav::default(),
            NetId(net_id),
        ))
        .id();

    if kind == ZombieKind::Giant {
        commands
            .entity(entity)
            .insert(GiantAttack { cooldown: 2.0 })
            .with_children(|parent| {
                // HP bar background
                parent.spawn(SpriteBundle {
                    sprite: Sprite {
                        custom_size: Some(Vec2::new(GIANT_HP_BAR_WIDTH, 5.0)),
                        color: Color::srgba(0.1, 0.1, 0.1, 0.7),
                        ..default()
                    },
                    transform: Transform::from_xyz(0.0, 40.0, 1.0),
                    ..default()
                });
                // HP bar fill
                parent.spawn((
                    SpriteBundle {
                        sprite: Sprite {
                            custom_size: Some(Vec2::new(GIANT_HP_BAR_WIDTH, 5.0)),
                            color: Color::srgb(0.8, 0.15, 0.15),
                            anchor: bevy::sprite::Anchor::CenterLeft,
                            ..default()
                        },
                        transform: Transform::from_xyz(
                            -GIANT_HP_BAR_WIDTH / 2.0,
                            40.0,
                            1.1,
                        ),
                        ..default()
                    },
                    ZombieHpBar,
                ));
            });
    }

    entity
}

const MAX_ALIVE_ZOMBIES: usize = 70;

#[allow(clippy::too_many_arguments)]
fn spawn_zombie_listener(
    mut commands: Commands,
    mut events: EventReader<SpawnZombieEvent>,
    assets: Res<ZombieAssets>,
    mut ctx: ResMut<NetContext>,
    _zone_state: Res<ZoneState>,
    nav: Res<NavGrid>,
    segments: Res<MapSegmentUnlockState>,
    existing: Query<(), With<Zombie>>,
    players: Query<&Transform, With<Player>>,
) {
    // Waves feed at most a few spawns per second while this runs at 60 Hz —
    // skip the alive count and spawn-point Vec builds on event-less ticks.
    if events.is_empty() {
        return;
    }
    let alive = existing.iter().count();
    let mut spawned = 0;
    let mut rng = rand::thread_rng();

    // Pick a spawn point spec for this zombie.  Spawn points belonging to
    // a locked segment are excluded — zombies don't emerge from areas
    // the players haven't paid to open up yet.
    let exterior: Vec<&SpawnPointSpec> = SPAWN_POINTS
        .iter()
        .filter(|s| !s.interior_only && segments.is_unlocked(s.segment_idx))
        .collect();
    let interior: Vec<&SpawnPointSpec> = SPAWN_POINTS
        .iter()
        .filter(|s| s.interior_only && segments.is_unlocked(s.segment_idx))
        .collect();

    // Detect whether any player has descended into the metro level — if so
    // we route a fraction of spawns to a fixed underground spawn so the
    // tunnel actually fills up with zombies "leaking down from upstairs".
    let any_player_underground = players
        .iter()
        .any(|t| t.translation.y < crate::underground::UNDER_TOP);

    for ev in events.read() {
        if alive + spawned >= MAX_ALIVE_ZOMBIES {
            continue;
        }
        spawned += 1;

        // 60% of the time, when any player is in the metro, spawn this
        // zombie underground at the manhole drop point (on the platform).
        if any_player_underground && rng.gen_bool(0.6) {
            let pos = Vec2::new(
                crate::underground::MANHOLE_X + rng.gen_range(-30.0..30.0),
                crate::underground::UNDER_TOP - 100.0 + rng.gen_range(-20.0..20.0),
            );
            let base = ev.kind.base_speed();
            let jitter: f32 = rng.gen_range(-12.0..18.0);
            let speed = base + jitter;
            let net_id = ctx.alloc_zombie_id();
            spawn_zombie_entity(
                &mut commands,
                &assets,
                pos,
                net_id,
                ev.kind.base_hp(),
                speed,
                ev.kind,
            );
            continue;
        }

        let pool: &Vec<&SpawnPointSpec> = if matches!(ev.kind, ZombieKind::Fast)
            && !interior.is_empty()
            && rng.gen_bool(0.35)
        {
            &interior
        } else if !exterior.is_empty() {
            &exterior
        } else {
            &interior
        };
        let sp = pool[rng.gen_range(0..pool.len())];
        let base_pos = spawn_point_world(sp);

        // Nudge a few tiles into the room so the zombie isn't intersecting the wall.
        let inward = match sp.side {
            crate::map::WallSide::N => Vec2::new(0.0, -TILE_SIZE * 1.5),
            crate::map::WallSide::S => Vec2::new(0.0, TILE_SIZE * 1.5),
            crate::map::WallSide::E => Vec2::new(-TILE_SIZE * 1.5, 0.0),
            crate::map::WallSide::W => Vec2::new(TILE_SIZE * 1.5, 0.0),
        };
        let mut pos = base_pos + inward;

        // Snap to the nearest walkable tile within a small radius if we landed
        // on something solid (a wall column, decor, etc.).
        let (c0, r0) = world_to_tile(pos);
        if !(in_bounds(c0, r0) && nav.effective[nav_idx(c0, r0)]) {
            'snap: for ring in 1_i32..=4 {
                for dr in -ring..=ring {
                    for dc in -ring..=ring {
                        if dc.abs() != ring && dr.abs() != ring {
                            continue;
                        }
                        let (c, r) = (c0 + dc, r0 + dr);
                        if in_bounds(c, r) && nav.effective[nav_idx(c, r)] {
                            pos = tile_center(c, r);
                            break 'snap;
                        }
                    }
                }
            }
        }

        let base = ev.kind.base_speed();
        let jitter: f32 = match ev.kind {
            ZombieKind::Normal => rng.gen_range(-15.0..35.0),
            ZombieKind::Fast => rng.gen_range(-18.0..25.0),
            ZombieKind::Exploder => rng.gen_range(-10.0..18.0),
            ZombieKind::Burning => rng.gen_range(-12.0..20.0),
            ZombieKind::Giant => rng.gen_range(-5.0..5.0),
        };
        let speed = base + jitter;
        let net_id = ctx.alloc_zombie_id();
        spawn_zombie_entity(
            &mut commands,
            &assets,
            pos,
            net_id,
            ev.kind.base_hp(),
            speed,
            ev.kind,
        );
    }
}

fn update_nav_flow(mut nav: ResMut<NavGrid>, players: Query<(&Transform, &Player)>) {
    let mut alive: Vec<u8> = Vec::new();
    let mut rebuilds: Vec<(u8, Vec2, (i32, i32))> = Vec::new();

    for (t, p) in &players {
        if p.hp <= 0 {
            continue;
        }
        alive.push(p.id);
        let pos = t.translation.truncate();
        let tile = world_to_tile(pos);
        let needs_rebuild = nav
            .player_flow_tile
            .get(&p.id)
            .copied()
            .is_none_or(|prev| prev != tile);
        if needs_rebuild {
            rebuilds.push((p.id, pos, tile));
        }
    }

    if !rebuilds.is_empty() {
        // Run all BFS passes against the live `nav.effective` slice (no
        // 11.5k-bool clone), then write the results back to the same nav
        // resource.  Split-borrowing field-by-field keeps the borrow checker
        // happy: the borrow of `nav.effective` and the mutation of
        // `nav.player_flow` are on disjoint fields.
        let nav = &mut *nav;
        for (id, pos, tile) in rebuilds {
            let field = bfs_distance_field(&nav.effective, pos);
            nav.player_flow.insert(id, field);
            nav.player_flow_tile.insert(id, tile);
        }
    }

    let stale: Vec<u8> = nav
        .player_flow
        .keys()
        .filter(|k| !alive.contains(k))
        .copied()
        .collect();
    for k in stale {
        nav.player_flow.remove(&k);
        nav.player_flow_tile.remove(&k);
    }
}

fn zombie_flow_direction(nav: &NavGrid, zombie_pos: Vec2, player_pos: Vec2) -> Option<Vec2> {
    let (zc, zr) = world_to_tile(zombie_pos);
    let flow = nav.player_flow.values().min_by_key(|field| {
        if !in_bounds(zc, zr) {
            return u16::MAX;
        }
        field[nav_idx(zc, zr)]
    })?;
    if !in_bounds(zc, zr) {
        return None;
    }
    let my_d = flow[nav_idx(zc, zr)];
    if my_d == 0 {
        return Some((player_pos - zombie_pos).normalize_or_zero());
    }
    // A zombie can legally stand in the outer band of a nav-blocked tile
    // (the bake tests only the tile centre), where its own field entry reads
    // MAX.  Recover by heading for the cheapest finite neighbour instead of
    // bailing to the straight-line fallback, which happily aims through
    // building walls; the diagonal corner-cut test is skipped here because
    // it consults the (MAX) entries around the blocked tile.
    let recovering = my_d == u16::MAX;
    let dirs: [(i32, i32); 8] = [
        (-1, 0), (1, 0), (0, -1), (0, 1),
        (-1, -1), (-1, 1), (1, -1), (1, 1),
    ];
    let mut best: Option<(u16, (i32, i32))> = None;
    for &(dc, dr) in &dirs {
        let (nc, nr) = (zc + dc, zr + dr);
        if !in_bounds(nc, nr) {
            continue;
        }
        let d = flow[nav_idx(nc, nr)];
        if d == u16::MAX {
            continue;
        }
        if !recovering && dc != 0 && dr != 0 {
            let idx_a = nav_idx(zc + dc, zr);
            let idx_b = nav_idx(zc, zr + dr);
            if flow[idx_a] == u16::MAX || flow[idx_b] == u16::MAX {
                continue;
            }
        }
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, (nc, nr)));
        }
    }
    let (_, (nc, nr)) = best?;
    let target = tile_center(nc, nr);
    Some((target - zombie_pos).normalize_or_zero())
}

fn rotate_vec(v: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// Per-zombie local-steering state.  Host-side only (movement is
/// authoritative there); never serialized.
#[derive(Component, Default)]
pub struct ZombieNav {
    /// Detour side committed to while an obstacle blocks the desired
    /// direction: +1.0 / -1.0, or 0.0 when the path ahead is clear.  Without
    /// this the probe loop re-picks a side every frame and near-symmetric
    /// setups (a tree dead ahead) make the zombie flip-flop in place.
    avoid_sign: f32,
    /// Seconds of consecutive near-zero forward progress.
    stuck_time: f32,
    /// While > 0, movement is committed to `escape_dir` regardless of the
    /// flow field — breaks concave prop pockets the probe loop can't solve.
    escape_timer: f32,
    escape_dir: Vec2,
    /// Consecutive escape triggers without real progress in between — after
    /// two failed sideways tries the third backs straight out of the pocket.
    escape_attempts: u8,
}

/// Returns the steered direction plus the detour sign it used (0.0 = went
/// straight).  `prefer` biases the probe loop to re-try the previously
/// chosen side first so a committed detour is stable frame-to-frame.
fn steer_around_obstacles(
    pos: Vec2,
    desired: Vec2,
    obstacles: &MapObstacles,
    radius: f32,
    prefer: f32,
) -> (Vec2, f32) {
    if desired == Vec2::ZERO {
        return (desired, 0.0);
    }
    let near = radius + 6.0;
    let far = radius + 22.0;
    if !obstacles.hits(pos + desired * near, radius)
        && !obstacles.hits(pos + desired * far, radius * 0.85)
    {
        return (desired, 0.0);
    }
    let angle_steps: [f32; 5] = [
        std::f32::consts::FRAC_PI_8,
        std::f32::consts::FRAC_PI_4,
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4,
        std::f32::consts::PI * 0.85,
    ];
    let signs = if prefer < 0.0 {
        [-1.0_f32, 1.0]
    } else {
        [1.0_f32, -1.0]
    };
    for &mag in &angle_steps {
        for sign in signs {
            let alt = rotate_vec(desired, sign * mag);
            if obstacles.hits(pos + alt * near, radius) {
                continue;
            }
            if obstacles.hits(pos + alt * far, radius * 0.85) {
                continue;
            }
            return (alt, sign);
        }
    }
    for sign in signs {
        let alt = rotate_vec(desired, sign * std::f32::consts::FRAC_PI_2);
        if !obstacles.hits(pos + alt * near, radius) {
            return (alt, sign);
        }
    }
    // Totally blocked: keep the caller's committed side (0.0 would erase it
    // and make every stuck-watchdog escape re-roll a random side).
    (desired, prefer)
}

/// Progress ratio (moved / expected) below which a tick counts as "stuck".
const STUCK_PROGRESS_RATIO: f32 = 0.35;
/// Consecutive stuck seconds before a committed sideways escape kicks in.
const STUCK_TRIGGER_SECS: f32 = 0.5;
/// How long a triggered escape stays committed.
const ESCAPE_SECS: f32 = 0.45;

fn zombie_movement(
    time: Res<Time>,
    obstacles: Res<MapObstacles>,
    nav: Res<NavGrid>,
    mut zombies: Query<(&mut Transform, &Zombie, &mut ZombieNav), Without<Player>>,
    players: Query<(&Transform, &Player)>,
) {
    let dt = time.delta_seconds();
    let mut rng = rand::thread_rng();
    for (mut transform, zombie, mut znav) in &mut zombies {
        let pos = transform.translation.truncate();
        let radius = zombie.kind.radius();

        let mut nearest: Option<Vec2> = None;
        let mut best_d2 = f32::INFINITY;
        for (pt, p) in &players {
            if p.hp <= 0 {
                continue;
            }
            let pp = pt.translation.truncate();
            let d2 = pp.distance_squared(pos);
            if d2 < best_d2 {
                best_d2 = d2;
                nearest = Some(pp);
            }
        }
        let Some(target) = nearest else {
            continue;
        };

        let flow = zombie_flow_direction(&nav, pos, target)
            .unwrap_or_else(|| (target - pos).normalize_or_zero());
        let escaping = znav.escape_timer > 0.0;
        let dir = if escaping {
            znav.escape_timer -= dt;
            znav.escape_dir
        } else {
            let (dir, sign) = steer_around_obstacles(pos, flow, &obstacles, radius, znav.avoid_sign);
            znav.avoid_sign = sign;
            dir
        };

        // While escaping keep facing the flow direction — the escape is a
        // sidestep, not a change of heart, and snapping the sprite ±90°/180°
        // for half a second reads as twitching (snapshots ship it to clients
        // too).
        let facing = if escaping && flow != Vec2::ZERO { flow } else { dir };
        if facing != Vec2::ZERO {
            transform.rotation = Quat::from_rotation_z(facing.y.atan2(facing.x));
        }
        transform.translation += (dir * zombie.speed * dt).extend(0.0);

        let mut new_pos = transform.translation.truncate();
        obstacles.resolve(&mut new_pos, radius);
        transform.translation.x = new_pos.x;
        transform.translation.y = new_pos.y;

        // Stuck watchdog: steering can still deadlock in concave prop
        // pockets (probe passes, resolve cancels the motion, repeat).  When
        // actual displacement stays far below `speed * dt` for a while,
        // commit to a sideways escape for a few tenths of a second — long
        // enough to clear the pocket and pick up a fresh flow direction.
        // Escapes escalate: two sideways tries (alternating sides), then a
        // longer straight back-out; real progress resets the ladder.
        let expected = zombie.speed * dt;
        if expected <= f32::EPSILON {
            continue;
        }
        let progressing = new_pos.distance(pos) >= expected * STUCK_PROGRESS_RATIO;
        if progressing {
            znav.stuck_time = 0.0;
            if !escaping {
                znav.escape_attempts = 0;
            }
        } else if escaping {
            // The committed escape is itself pinned (it was probed only one
            // step ahead) — abort it early and flip the preferred side so
            // the next attempt tries the other way instead of burning the
            // rest of the commit window against the same prop.
            znav.escape_timer = 0.0;
            if znav.avoid_sign != 0.0 {
                znav.avoid_sign = -znav.avoid_sign;
            }
            znav.stuck_time = STUCK_TRIGGER_SECS * 0.6;
        } else {
            znav.stuck_time += dt;
        }
        if znav.stuck_time >= STUCK_TRIGGER_SECS && znav.escape_timer <= 0.0 {
            znav.stuck_time = 0.0;
            znav.escape_attempts = znav.escape_attempts.saturating_add(1);
            if znav.escape_attempts >= 3 {
                znav.escape_dir = -flow;
                znav.escape_timer = ESCAPE_SECS * 2.0;
                znav.escape_attempts = 0;
            } else {
                let side = if znav.avoid_sign != 0.0 {
                    znav.avoid_sign
                } else if rng.gen_bool(0.5) {
                    1.0
                } else {
                    -1.0
                };
                let probe = radius + 8.0;
                let mut esc = rotate_vec(flow, side * std::f32::consts::FRAC_PI_2);
                if obstacles.hits(new_pos + esc * probe, radius) {
                    esc = rotate_vec(flow, -side * std::f32::consts::FRAC_PI_2);
                }
                if obstacles.hits(new_pos + esc * probe, radius) {
                    esc = -flow;
                }
                znav.escape_dir = esc;
                znav.escape_timer = ESCAPE_SECS;
                znav.avoid_sign = side;
            }
        }
    }
}

fn zombie_attack(
    mut commands: Commands,
    mut zombies: Query<(Entity, &Transform, &mut Zombie), Without<Player>>,
    players: Query<(Entity, &Transform, &Player, Option<&BurnEffect>)>,
    mut dmg: EventWriter<PlayerDamagedEvent>,
    mut explode: EventWriter<ExplodeEvent>,
    mut killed: EventWriter<ZombieKilledEvent>,
) {
    for (z_ent, z_t, mut zombie) in &mut zombies {
        if zombie.hp <= 0 {
            continue;
        }
        let zp = z_t.translation.truncate();
        let zr = zombie.kind.radius();
        let mut triggered = false;
        for (p_ent, pt, player, burn) in &players {
            if player.hp <= 0 {
                continue;
            }
            let p = pt.translation.truncate();
            let r = PLAYER_RADIUS + zr;
            if p.distance_squared(zp) < r * r {
                match zombie.kind {
                    ZombieKind::Exploder => {
                        triggered = true;
                        break;
                    }
                    ZombieKind::Burning => {
                        dmg.send(PlayerDamagedEvent {
                            target_id: player.id,
                            amount: zombie.kind.contact_damage(),
                        });
                        // Insert burn DoT only if not already burning — otherwise
                        // continuous contact would refresh `remaining` every tick,
                        // making the effect impossible to outlast and discarding
                        // the partial `accumulated` damage from the prior frame.
                        if burn.is_none() {
                            commands.entity(p_ent).insert(BurnEffect {
                                remaining: BURN_DURATION,
                                accumulated: 0.0,
                            });
                        }
                    }
                    _ => {
                        dmg.send(PlayerDamagedEvent {
                            target_id: player.id,
                            amount: zombie.kind.contact_damage(),
                        });
                    }
                }
            }
        }
        if triggered {
            zombie.hp = 0;
            explode.send(ExplodeEvent {
                pos: zp,
                radius: EXPLODER_EXPLOSION_RADIUS,
                zombie_damage: EXPLODER_EXPLOSION_ZOMBIE_DAMAGE,
                player_damage: EXPLODER_EXPLOSION_PLAYER_DAMAGE,
            });
            killed.send(ZombieKilledEvent {
                kind: zombie.kind,
                by_explosion: false,
                pos: zp,
            });
            commands.entity(z_ent).despawn_recursive();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn burn_tick_system(
    mut commands: Commands,
    time: Res<Time>,
    mut burn_query: Query<(Entity, &Transform, &mut Player, &mut BurnEffect)>,
    other_players: Query<&Player, Without<BurnEffect>>,
    mut net_entities: ResMut<NetEntities>,
    mut dead_players: ResMut<DeadPlayers>,
    mut died_evw: EventWriter<PlayerDiedEvent>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let dt = time.delta_seconds();
    // Players killed in this tick — kept so we can both fire death events and
    // exclude them when checking for game-over below.
    let mut newly_dead: Vec<u8> = Vec::new();
    for (entity, transform, mut player, mut burn) in &mut burn_query {
        if player.hp <= 0 {
            commands.entity(entity).remove::<BurnEffect>();
            continue;
        }
        burn.remaining -= dt;
        burn.accumulated += BURN_DPS * dt;
        if burn.accumulated >= 1.0 {
            let dmg = burn.accumulated.floor() as i32;
            player.hp -= dmg;
            burn.accumulated -= dmg as f32;
        }
        if burn.remaining <= 0.0 {
            commands.entity(entity).remove::<BurnEffect>();
        }
        if player.hp <= 0 {
            newly_dead.push(player.id);
            died_evw.send(PlayerDiedEvent {
                player_id: player.id,
                pos: transform.translation.truncate(),
                aim_rot: player.aim.y.atan2(player.aim.x),
            });
            commands.entity(entity).despawn_recursive();
            net_entities.players.remove(&player.id);
            // Track in DeadPlayers so the wave-clear respawn (and revive
            // system) actually brings them back — without this, a burn-death
            // permanently removes the player from the round.
            if !dead_players.0.contains(&player.id) {
                dead_players.0.push(player.id);
            }
        }
    }
    if !newly_dead.is_empty() {
        // Despawn is queued (not yet flushed), so the just-killed entities are
        // still visible in `burn_query.iter()` with hp <= 0; filter them out.
        let alive = burn_query
            .iter()
            .filter(|(_, _, p, _)| p.hp > 0 && !newly_dead.contains(&p.id))
            .count()
            + other_players.iter().filter(|p| p.hp > 0).count();
        if alive == 0 {
            next_state.set(GameState::GameOver);
        }
    }
}

fn giant_toxic_attack(
    time: Res<Time>,
    mut commands: Commands,
    assets: Res<ZombieAssets>,
    mut giants: Query<(&Transform, &mut GiantAttack)>,
    players: Query<(&Transform, &Player)>,
) {
    let dt = time.delta_seconds();
    for (zt, mut attack) in &mut giants {
        attack.cooldown -= dt;
        if attack.cooldown > 0.0 {
            continue;
        }
        let zp = zt.translation.truncate();
        for (pt, player) in &players {
            if player.hp <= 0 {
                continue;
            }
            let pp = pt.translation.truncate();
            if pp.distance_squared(zp) < GIANT_ATTACK_RANGE * GIANT_ATTACK_RANGE {
                attack.cooldown = GIANT_ATTACK_COOLDOWN;
                commands.spawn((
                    SpriteBundle {
                        texture: assets.toxic_cloud.clone(),
                        sprite: Sprite {
                            custom_size: Some(Vec2::splat(TOXIC_CLOUD_RADIUS * 2.0)),
                            color: Color::srgba(1.0, 1.0, 1.0, 0.7),
                            ..default()
                        },
                        transform: Transform::from_xyz(pp.x, pp.y, 4.0),
                        ..default()
                    },
                    ToxicCloud {
                        lifetime: TOXIC_CLOUD_LIFETIME,
                        tick: 0.0,
                    },
                ));
                break;
            }
        }
    }
}

fn toxic_cloud_tick(
    mut commands: Commands,
    time: Res<Time>,
    mut clouds: Query<(Entity, &Transform, &mut ToxicCloud, &mut Sprite)>,
    players: Query<(Entity, &Transform, &Player)>,
    mut dmg: EventWriter<PlayerDamagedEvent>,
) {
    let dt = time.delta_seconds();
    for (entity, ct, mut cloud, mut sprite) in &mut clouds {
        cloud.lifetime -= dt;
        if cloud.lifetime <= 0.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        // Fade out
        let alpha = (cloud.lifetime / TOXIC_CLOUD_LIFETIME).clamp(0.0, 0.7);
        sprite.color = Color::srgba(1.0, 1.0, 1.0, alpha);

        // Damage tick
        cloud.tick += dt;
        if cloud.tick >= 0.5 {
            cloud.tick -= 0.5;
            let cp = ct.translation.truncate();
            for (_p_ent, pt, player) in &players {
                if player.hp <= 0 {
                    continue;
                }
                let pp = pt.translation.truncate();
                if pp.distance_squared(cp) < TOXIC_CLOUD_RADIUS * TOXIC_CLOUD_RADIUS {
                    dmg.send(PlayerDamagedEvent {
                        target_id: player.id,
                        amount: (TOXIC_CLOUD_DPS * 0.5) as i32,
                    });
                }
            }
        }
    }
}

fn update_zombie_hp_bars(
    zombies: Query<(&Zombie, &Children)>,
    mut bars: Query<&mut Sprite, With<ZombieHpBar>>,
) {
    for (zombie, children) in &zombies {
        if zombie.kind != ZombieKind::Giant {
            continue;
        }
        let max_hp = zombie.kind.base_hp() as f32;
        let pct = (zombie.hp as f32 / max_hp).clamp(0.0, 1.0);
        for child in children.iter() {
            if let Ok(mut sprite) = bars.get_mut(*child) {
                sprite.custom_size = Some(Vec2::new(GIANT_HP_BAR_WIDTH * pct, 5.0));
            }
        }
    }
}

fn despawn_all_zombies(
    mut commands: Commands,
    q: Query<Entity, With<Zombie>>,
    clouds: Query<Entity, With<ToxicCloud>>,
    stains: Query<Entity, With<BloodStain>>,
) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
    for e in &clouds {
        commands.entity(e).despawn_recursive();
    }
    for e in &stains {
        commands.entity(e).despawn_recursive();
    }
}

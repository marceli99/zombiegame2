use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::audio::SfxEvent;
use crate::map::{is_walkable_tile, tile_center, MapObstacles, MAP_COLS, MAP_ROWS};
use crate::net::{is_authoritative, NetContext, NetEntities, NetId};
use crate::pixelart::{Canvas, Rgba};
use crate::player::{Player, PLAYER_RADIUS};
use crate::{gameplay_active, GameState};

const PICKUP_SPRITE_SIZE: Vec2 = Vec2::new(30.0, 16.0);
const PICKUP_PICK_RADIUS: f32 = 16.0;
const TARGET_PICKUP_COUNT: usize = 10;
const RESPAWN_INTERVAL: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Weapon {
    #[default]
    Pistol = 0,
    Smg = 1,
    Shotgun = 2,
    Rifle = 3,
}

impl Weapon {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Weapon::Smg,
            2 => Weapon::Shotgun,
            3 => Weapon::Rifle,
            _ => Weapon::Pistol,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn fire_cooldown(self) -> f32 {
        match self {
            Weapon::Pistol => 0.18,
            Weapon::Smg => 0.07,
            Weapon::Shotgun => 0.55,
            Weapon::Rifle => 0.42,
        }
    }

    pub fn bullet_damage(self) -> i32 {
        match self {
            Weapon::Pistol => 2,
            Weapon::Smg => 1,
            Weapon::Shotgun => 2,
            Weapon::Rifle => 6,
        }
    }

    pub fn bullet_speed(self) -> f32 {
        match self {
            Weapon::Pistol => 720.0,
            Weapon::Smg => 820.0,
            Weapon::Shotgun => 620.0,
            Weapon::Rifle => 1080.0,
        }
    }

    pub fn bullet_count(self) -> u32 {
        match self {
            Weapon::Shotgun => 6,
            _ => 1,
        }
    }

    pub fn spread(self) -> f32 {
        match self {
            Weapon::Pistol => 0.0,
            Weapon::Smg => 0.08,
            Weapon::Shotgun => 0.34,
            Weapon::Rifle => 0.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Weapon::Pistol => "PISTOLET",
            Weapon::Smg => "SMG",
            Weapon::Shotgun => "SHOTGUN",
            Weapon::Rifle => "KARABIN",
        }
    }
}

#[derive(Component)]
pub struct WeaponPickup {
    pub kind: Weapon,
}

#[derive(Resource)]
pub struct WeaponAssets {
    pub images: [Handle<Image>; 4],
}

#[derive(Resource)]
struct PickupRespawnTimer(f32);

impl Default for PickupRespawnTimer {
    fn default() -> Self {
        Self(RESPAWN_INTERVAL)
    }
}

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PickupRespawnTimer>()
            .add_systems(Startup, setup_weapon_assets)
            .add_systems(
                OnEnter(GameState::Playing),
                initial_pickup_spawn.run_if(is_authoritative),
            )
            .add_systems(OnExit(GameState::Playing), despawn_all_pickups)
            .add_systems(
                FixedUpdate,
                (pickup_collection, pickup_respawn)
                    .chain()
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

fn setup_weapon_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let imgs = [
        images.add(build_pistol_image()),
        images.add(build_smg_image()),
        images.add(build_shotgun_image()),
        images.add(build_rifle_image()),
    ];
    commands.insert_resource(WeaponAssets { images: imgs });
}

const WEIGHTS: [(Weapon, u32); 4] = [
    (Weapon::Pistol, 36),
    (Weapon::Smg, 30),
    (Weapon::Shotgun, 22),
    (Weapon::Rifle, 12),
];

fn pick_weapon<R: Rng>(rng: &mut R) -> Weapon {
    let total: u32 = WEIGHTS.iter().map(|(_, w)| w).sum();
    let mut roll = rng.gen_range(0..total);
    for (w, wt) in &WEIGHTS {
        if roll < *wt {
            return *w;
        }
        roll -= wt;
    }
    Weapon::Pistol
}

fn find_pickup_spot<R: Rng>(rng: &mut R, obstacles: &MapObstacles) -> Option<Vec2> {
    for _ in 0..80 {
        let col = rng.gen_range(0..MAP_COLS);
        let row = rng.gen_range(0..MAP_ROWS);
        if !is_walkable_tile(col, row) {
            continue;
        }
        let center = tile_center(col, row);
        let p = Vec2::new(
            center.x + rng.gen_range(-22.0..22.0),
            center.y + rng.gen_range(-22.0..22.0),
        );
        if obstacles.hits(p, 18.0) {
            continue;
        }
        if p.x.abs() < 70.0 && p.y.abs() < 70.0 {
            continue;
        }
        return Some(p);
    }
    None
}

pub fn spawn_pickup_entity(
    commands: &mut Commands,
    assets: &WeaponAssets,
    pos: Vec2,
    kind: Weapon,
    net_id: u32,
) -> Entity {
    commands
        .spawn((
            SpriteBundle {
                texture: assets.images[kind.as_u8() as usize].clone(),
                sprite: Sprite {
                    custom_size: Some(PICKUP_SPRITE_SIZE),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 0.5),
                ..default()
            },
            WeaponPickup { kind },
            NetId(net_id),
        ))
        .id()
}

fn initial_pickup_spawn(
    mut commands: Commands,
    assets: Res<WeaponAssets>,
    obstacles: Res<MapObstacles>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
    mut timer: ResMut<PickupRespawnTimer>,
) {
    timer.0 = RESPAWN_INTERVAL;
    let mut rng = StdRng::seed_from_u64(0xC0FFEE);
    for _ in 0..TARGET_PICKUP_COUNT {
        let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
            continue;
        };
        let kind = pick_weapon(&mut rng);
        let net_id = ctx.alloc_pickup_id();
        let entity = spawn_pickup_entity(&mut commands, &assets, p, kind, net_id);
        net_entities.pickups.insert(net_id, entity);
    }
}

fn despawn_all_pickups(
    mut commands: Commands,
    q: Query<Entity, With<WeaponPickup>>,
    mut net_entities: ResMut<NetEntities>,
) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
    net_entities.pickups.clear();
}

fn pickup_collection(
    mut commands: Commands,
    pickups: Query<(Entity, &Transform, &WeaponPickup, &NetId)>,
    mut players: Query<(&Transform, &mut Player)>,
    mut net_entities: ResMut<NetEntities>,
    mut sfx: EventWriter<SfxEvent>,
) {
    for (p_t, mut player) in &mut players {
        if player.hp <= 0 {
            continue;
        }
        let pp = p_t.translation.truncate();
        for (entity, pk_t, pickup, net_id) in &pickups {
            let d = pp.distance(pk_t.translation.truncate());
            if d < PLAYER_RADIUS + PICKUP_PICK_RADIUS && player.weapon != pickup.kind {
                player.weapon = pickup.kind;
                player.fire_cooldown = 0.0;
                net_entities.pickups.remove(&net_id.0);
                commands.entity(entity).despawn_recursive();
                sfx.send(SfxEvent::Hit);
                break;
            }
        }
    }
}

fn pickup_respawn(
    mut commands: Commands,
    assets: Res<WeaponAssets>,
    obstacles: Res<MapObstacles>,
    existing: Query<&WeaponPickup>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
    mut timer: ResMut<PickupRespawnTimer>,
    time: Res<Time>,
) {
    timer.0 -= time.delta_seconds();
    if timer.0 > 0.0 {
        return;
    }
    timer.0 = RESPAWN_INTERVAL;

    let count = existing.iter().count();
    if count >= TARGET_PICKUP_COUNT {
        return;
    }
    let mut rng = rand::thread_rng();
    let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
        return;
    };
    let kind = pick_weapon(&mut rng);
    let net_id = ctx.alloc_pickup_id();
    let entity = spawn_pickup_entity(&mut commands, &assets, p, kind, net_id);
    net_entities.pickups.insert(net_id, entity);
}

fn build_pistol_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let frame: Rgba = [32, 32, 38, 255];
    let frame_light: Rgba = [60, 62, 70, 255];
    let slide: Rgba = [46, 48, 54, 255];
    let slide_light: Rgba = [82, 86, 94, 255];
    let grip: Rgba = [54, 36, 18, 255];
    let grip_light: Rgba = [84, 58, 30, 255];
    let muzzle: Rgba = [14, 14, 16, 255];

    let mut c = Canvas::new(22, 11);

    c.fill_rect(3, 2, 16, 5, outline);
    c.fill_rect(4, 3, 14, 3, slide);
    c.fill_rect(4, 3, 14, 1, slide_light);
    c.put(16, 4, outline);
    c.put(14, 4, outline);

    c.fill_rect(18, 4, 3, 2, outline);
    c.put(19, 4, muzzle);
    c.put(20, 4, muzzle);

    c.fill_rect(5, 6, 4, 4, outline);
    c.fill_rect(6, 7, 2, 3, grip);
    c.put(6, 7, grip_light);

    c.fill_rect(9, 6, 3, 2, outline);
    c.put(10, 7, frame);

    c.fill_rect(4, 4, 1, 2, frame_light);

    c.into_image()
}

fn build_smg_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let body: Rgba = [38, 40, 46, 255];
    let body_light: Rgba = [70, 74, 82, 255];
    let body_dark: Rgba = [22, 24, 28, 255];
    let mag: Rgba = [28, 28, 32, 255];
    let stock: Rgba = [50, 34, 16, 255];
    let tip: Rgba = [14, 14, 16, 255];

    let mut c = Canvas::new(24, 12);

    c.fill_rect(4, 3, 16, 4, outline);
    c.fill_rect(5, 4, 14, 2, body);
    c.fill_rect(5, 4, 14, 1, body_light);
    c.fill_rect(5, 5, 14, 1, body_dark);

    c.fill_rect(19, 4, 3, 2, outline);
    c.put(20, 4, tip);
    c.put(21, 4, tip);

    c.fill_rect(8, 6, 5, 4, outline);
    c.fill_rect(9, 7, 3, 3, mag);

    c.fill_rect(13, 6, 3, 2, outline);

    c.fill_rect(1, 3, 4, 4, outline);
    c.fill_rect(2, 4, 2, 2, stock);

    c.into_image()
}

fn build_shotgun_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let barrel: Rgba = [36, 38, 44, 255];
    let barrel_light: Rgba = [68, 72, 80, 255];
    let barrel_dark: Rgba = [18, 20, 24, 255];
    let wood: Rgba = [72, 44, 18, 255];
    let wood_light: Rgba = [108, 70, 30, 255];
    let wood_dark: Rgba = [42, 24, 10, 255];
    let pump: Rgba = [60, 38, 16, 255];
    let tip: Rgba = [12, 12, 14, 255];

    let mut c = Canvas::new(30, 10);

    c.fill_rect(13, 3, 16, 3, outline);
    c.fill_rect(14, 4, 14, 1, barrel);
    c.fill_rect(14, 4, 14, 1, barrel_light);
    c.fill_rect(14, 5, 14, 1, barrel_dark);

    c.fill_rect(28, 3, 2, 3, outline);
    c.put(28, 4, tip);

    c.fill_rect(10, 3, 4, 4, outline);
    c.fill_rect(11, 4, 2, 2, pump);

    c.fill_rect(2, 3, 10, 4, outline);
    c.fill_rect(3, 4, 8, 2, wood);
    c.fill_rect(3, 4, 8, 1, wood_light);
    c.fill_rect(3, 5, 8, 1, wood_dark);
    c.put(4, 4, wood_light);

    c.fill_rect(2, 6, 2, 2, outline);
    c.put(2, 7, wood_dark);

    c.into_image()
}

fn build_rifle_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let barrel: Rgba = [32, 34, 40, 255];
    let barrel_light: Rgba = [60, 64, 72, 255];
    let barrel_dark: Rgba = [14, 14, 18, 255];
    let wood: Rgba = [58, 36, 14, 255];
    let wood_light: Rgba = [92, 58, 24, 255];
    let wood_dark: Rgba = [30, 18, 6, 255];
    let scope: Rgba = [18, 20, 24, 255];
    let scope_light: Rgba = [52, 58, 66, 255];
    let tip: Rgba = [10, 10, 12, 255];

    let mut c = Canvas::new(32, 11);

    c.fill_rect(11, 5, 20, 2, outline);
    c.fill_rect(12, 5, 18, 1, barrel);
    c.fill_rect(12, 5, 18, 1, barrel_light);
    c.fill_rect(12, 6, 18, 1, barrel_dark);

    c.fill_rect(30, 5, 2, 2, outline);
    c.put(30, 5, tip);

    c.fill_rect(14, 2, 7, 3, outline);
    c.fill_rect(15, 3, 5, 1, scope);
    c.put(16, 3, scope_light);
    c.put(18, 3, scope_light);
    c.put(14, 4, outline);
    c.put(20, 4, outline);

    c.fill_rect(2, 4, 10, 4, outline);
    c.fill_rect(3, 5, 8, 2, wood);
    c.fill_rect(3, 5, 8, 1, wood_light);
    c.fill_rect(3, 6, 8, 1, wood_dark);
    c.put(4, 5, wood_light);

    c.fill_rect(1, 5, 2, 3, outline);
    c.put(1, 6, wood_dark);

    c.fill_rect(11, 7, 3, 3, outline);
    c.put(12, 8, wood);

    c.into_image()
}

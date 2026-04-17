use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::audio::SfxEvent;
use crate::map::{is_walkable_tile, tile_center, MapObstacles, MAP_COLS, MAP_ROWS};
use crate::net::{is_authoritative, NetContext, NetEntities, NetId};
use crate::pixelart::{Canvas, Rgba};
use crate::player::{Player, PLAYER_MAX_HP, PLAYER_RADIUS};
use crate::{gameplay_active, GameState};

const PICKUP_SPRITE_SIZE: Vec2 = Vec2::new(30.0, 16.0);
const PICKUP_PICK_RADIUS: f32 = 16.0;
const TARGET_PICKUP_COUNT: usize = 10;
const RESPAWN_INTERVAL: f32 = 5.0;

const HEALTH_SPRITE_SIZE: Vec2 = Vec2::new(22.0, 16.0);
const TARGET_HEALTH_COUNT: usize = 6;
const HEALTH_RESPAWN_INTERVAL: f32 = 8.0;
const HEAL_AMOUNT: i32 = 30;

pub const HEALTH_PICKUP_KIND: u8 = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Weapon {
    #[default]
    Pistol = 0,
    Smg = 1,
    Shotgun = 2,
    Rifle = 3,
    RocketLauncher = 4,
    Minigun = 5,
    Flamethrower = 6,
    Sniper = 7,
    Uzi = 8,
    AutoShotgun = 9,
    MarksmanRifle = 10,
}

pub const WEAPON_COUNT: usize = 11;

impl Weapon {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Weapon::Smg,
            2 => Weapon::Shotgun,
            3 => Weapon::Rifle,
            4 => Weapon::RocketLauncher,
            5 => Weapon::Minigun,
            6 => Weapon::Flamethrower,
            7 => Weapon::Sniper,
            8 => Weapon::Uzi,
            9 => Weapon::AutoShotgun,
            10 => Weapon::MarksmanRifle,
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
            Weapon::RocketLauncher => 1.15,
            Weapon::Minigun => 0.05,
            Weapon::Flamethrower => 0.06,
            Weapon::Sniper => 0.90,
            Weapon::Uzi => 0.055,
            Weapon::AutoShotgun => 0.28,
            Weapon::MarksmanRifle => 0.50,
        }
    }

    pub fn bullet_damage(self) -> i32 {
        match self {
            Weapon::Pistol => 2,
            Weapon::Smg => 1,
            Weapon::Shotgun => 2,
            Weapon::Rifle => 6,
            Weapon::RocketLauncher => 0,
            Weapon::Minigun => 1,
            Weapon::Flamethrower => 1,
            Weapon::Sniper => 18,
            Weapon::Uzi => 1,
            Weapon::AutoShotgun => 2,
            Weapon::MarksmanRifle => 10,
        }
    }

    pub fn bullet_speed(self) -> f32 {
        match self {
            Weapon::Pistol => 720.0,
            Weapon::Smg => 820.0,
            Weapon::Shotgun => 620.0,
            Weapon::Rifle => 1080.0,
            Weapon::RocketLauncher => 520.0,
            Weapon::Minigun => 850.0,
            Weapon::Flamethrower => 300.0,
            Weapon::Sniper => 1400.0,
            Weapon::Uzi => 780.0,
            Weapon::AutoShotgun => 580.0,
            Weapon::MarksmanRifle => 1200.0,
        }
    }

    pub fn bullet_count(self) -> u32 {
        match self {
            Weapon::Shotgun => 6,
            Weapon::AutoShotgun => 4,
            Weapon::Flamethrower => 2,
            _ => 1,
        }
    }

    pub fn spread(self) -> f32 {
        match self {
            Weapon::Pistol => 0.0,
            Weapon::Smg => 0.08,
            Weapon::Shotgun => 0.34,
            Weapon::Rifle => 0.0,
            Weapon::RocketLauncher => 0.0,
            Weapon::Minigun => 0.09,
            Weapon::Flamethrower => 0.38,
            Weapon::Sniper => 0.0,
            Weapon::Uzi => 0.12,
            Weapon::AutoShotgun => 0.26,
            Weapon::MarksmanRifle => 0.0,
        }
    }

    pub fn is_rocket(self) -> bool {
        matches!(self, Weapon::RocketLauncher)
    }

    pub fn label(self) -> &'static str {
        match self {
            Weapon::Pistol => "PISTOL",
            Weapon::Smg => "SMG",
            Weapon::Shotgun => "SHOTGUN",
            Weapon::Rifle => "RIFLE",
            Weapon::RocketLauncher => "RPG",
            Weapon::Minigun => "MINIGUN",
            Weapon::Flamethrower => "FLAMETHROWER",
            Weapon::Sniper => "SNIPER",
            Weapon::Uzi => "UZI",
            Weapon::AutoShotgun => "AUTO SHOTGUN",
            Weapon::MarksmanRifle => "DMR",
        }
    }

    pub fn magazine_size(self) -> u32 {
        match self {
            Weapon::Pistol => 999,
            Weapon::Smg => 35,
            Weapon::Shotgun => 8,
            Weapon::Rifle => 20,
            Weapon::RocketLauncher => 3,
            Weapon::Minigun => 100,
            Weapon::Flamethrower => 50,
            Weapon::Sniper => 5,
            Weapon::Uzi => 40,
            Weapon::AutoShotgun => 12,
            Weapon::MarksmanRifle => 10,
        }
    }

    pub fn reserve_ammo(self) -> u32 {
        match self {
            Weapon::Pistol => 999,
            Weapon::Smg => 2100,
            Weapon::Shotgun => 600,
            Weapon::Rifle => 1200,
            Weapon::RocketLauncher => 150,
            Weapon::Minigun => 3000,
            Weapon::Flamethrower => 2000,
            Weapon::Sniper => 300,
            Weapon::Uzi => 2400,
            Weapon::AutoShotgun => 720,
            Weapon::MarksmanRifle => 600,
        }
    }

    pub fn reload_time(self) -> f32 {
        match self {
            Weapon::Pistol => 0.0,
            Weapon::Smg => 1.5,
            Weapon::Shotgun => 2.0,
            Weapon::Rifle => 2.0,
            Weapon::RocketLauncher => 2.8,
            Weapon::Minigun => 3.5,
            Weapon::Flamethrower => 2.5,
            Weapon::Sniper => 2.5,
            Weapon::Uzi => 1.2,
            Weapon::AutoShotgun => 2.2,
            Weapon::MarksmanRifle => 1.8,
        }
    }

    pub fn has_infinite_ammo(self) -> bool {
        matches!(self, Weapon::Pistol)
    }
}

// ── Throwables ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ThrowableKind {
    #[default]
    Grenade = 0,
    Smoke = 1,
    Molotov = 2,
}

impl ThrowableKind {
    #[allow(dead_code)]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Smoke,
            2 => Self::Molotov,
            _ => Self::Grenade,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Grenade => "GRENADE",
            Self::Smoke => "SMOKE",
            Self::Molotov => "MOLOTOV",
        }
    }

    #[allow(dead_code)]
    pub fn fuse_time(self) -> f32 {
        match self {
            Self::Grenade => 1.8,
            Self::Smoke => 1.2,
            Self::Molotov => 0.0, // explodes on contact/range
        }
    }

    pub fn throw_speed(self) -> f32 {
        match self {
            Self::Grenade => 480.0,
            Self::Smoke => 420.0,
            Self::Molotov => 400.0,
        }
    }
}

#[derive(Component)]
pub struct WeaponPickup {
    pub kind: Weapon,
}

#[derive(Component)]
pub struct HealthPickup;

#[derive(Resource)]
pub struct WeaponAssets {
    pub images: [Handle<Image>; WEAPON_COUNT],
}

#[derive(Resource)]
pub struct ThrowableAssets {
    pub grenade: Handle<Image>,
    pub smoke: Handle<Image>,
    pub molotov: Handle<Image>,
}

#[derive(Component)]
pub struct ThrowablePickup {
    pub kind: ThrowableKind,
    pub count: u32,
}

const THROWABLE_SPRITE_SIZE: Vec2 = Vec2::new(16.0, 14.0);
const TARGET_THROWABLE_COUNT: usize = 5;
const THROWABLE_RESPAWN_INTERVAL: f32 = 12.0;

#[derive(Resource)]
struct ThrowableRespawnTimer(f32);

impl Default for ThrowableRespawnTimer {
    fn default() -> Self {
        Self(THROWABLE_RESPAWN_INTERVAL)
    }
}

#[derive(Resource)]
pub struct HealthPickupAssets {
    pub image: Handle<Image>,
}

#[derive(Resource)]
struct PickupRespawnTimer(f32);

impl Default for PickupRespawnTimer {
    fn default() -> Self {
        Self(RESPAWN_INTERVAL)
    }
}

#[derive(Resource)]
struct HealthRespawnTimer(f32);

impl Default for HealthRespawnTimer {
    fn default() -> Self {
        Self(HEALTH_RESPAWN_INTERVAL)
    }
}

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PickupRespawnTimer>()
            .init_resource::<HealthRespawnTimer>()
            .init_resource::<ThrowableRespawnTimer>()
            .add_systems(Startup, (setup_weapon_assets, setup_health_assets, setup_throwable_assets))
            .add_systems(
                OnEnter(GameState::Playing),
                initial_pickup_spawn.run_if(is_authoritative),
            )
            .add_systems(OnExit(GameState::Playing), despawn_all_pickups)
            .add_systems(
                FixedUpdate,
                (
                    pickup_collection,
                    pickup_respawn,
                    health_collection,
                    health_respawn,
                    throwable_collection,
                    throwable_respawn,
                )
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
        images.add(build_rocket_launcher_image()),
        images.add(build_minigun_image()),
        images.add(build_flamethrower_image()),
        images.add(build_sniper_image()),
        images.add(build_uzi_image()),
        images.add(build_auto_shotgun_image()),
        images.add(build_marksman_rifle_image()),
    ];
    commands.insert_resource(WeaponAssets { images: imgs });
}

fn setup_throwable_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(ThrowableAssets {
        grenade: images.add(build_grenade_image()),
        smoke: images.add(build_smoke_grenade_image()),
        molotov: images.add(build_molotov_image()),
    });
}

fn setup_health_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(HealthPickupAssets {
        image: images.add(build_health_pickup_image()),
    });
}

const WEIGHTS: [(Weapon, u32); WEAPON_COUNT] = [
    (Weapon::Pistol, 12),
    (Weapon::Smg, 12),
    (Weapon::Shotgun, 10),
    (Weapon::Rifle, 8),
    (Weapon::RocketLauncher, 5),
    (Weapon::Minigun, 8),
    (Weapon::Flamethrower, 7),
    (Weapon::Sniper, 5),
    (Weapon::Uzi, 12),
    (Weapon::AutoShotgun, 10),
    (Weapon::MarksmanRifle, 7),
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

fn pick_throwable<R: Rng>(rng: &mut R) -> ThrowableKind {
    match rng.gen_range(0..3) {
        0 => ThrowableKind::Grenade,
        1 => ThrowableKind::Smoke,
        _ => ThrowableKind::Molotov,
    }
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

pub fn spawn_health_entity(
    commands: &mut Commands,
    assets: &HealthPickupAssets,
    pos: Vec2,
    net_id: u32,
) -> Entity {
    commands
        .spawn((
            SpriteBundle {
                texture: assets.image.clone(),
                sprite: Sprite {
                    custom_size: Some(HEALTH_SPRITE_SIZE),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 0.5),
                ..default()
            },
            HealthPickup,
            NetId(net_id),
        ))
        .id()
}

#[allow(clippy::too_many_arguments)]
fn initial_pickup_spawn(
    mut commands: Commands,
    assets: Res<WeaponAssets>,
    health_assets: Res<HealthPickupAssets>,
    throwable_assets: Res<ThrowableAssets>,
    obstacles: Res<MapObstacles>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
    mut timer: ResMut<PickupRespawnTimer>,
    mut health_timer: ResMut<HealthRespawnTimer>,
) {
    timer.0 = RESPAWN_INTERVAL;
    health_timer.0 = HEALTH_RESPAWN_INTERVAL;
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
    for _ in 0..TARGET_HEALTH_COUNT {
        let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
            continue;
        };
        let net_id = ctx.alloc_pickup_id();
        let entity = spawn_health_entity(&mut commands, &health_assets, p, net_id);
        net_entities.pickups.insert(net_id, entity);
    }
    for _ in 0..TARGET_THROWABLE_COUNT {
        let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
            continue;
        };
        let kind = pick_throwable(&mut rng);
        let count = rng.gen_range(1..=3);
        let net_id = ctx.alloc_pickup_id();
        let entity = spawn_throwable_pickup_entity(&mut commands, &throwable_assets, p, kind, count, net_id);
        net_entities.pickups.insert(net_id, entity);
    }
}

#[allow(clippy::type_complexity)]
fn despawn_all_pickups(
    mut commands: Commands,
    q: Query<Entity, Or<(With<WeaponPickup>, With<HealthPickup>, With<ThrowablePickup>)>>,
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
            if d < PLAYER_RADIUS + PICKUP_PICK_RADIUS {
                // Put in active weapon slot, or empty slot if available
                let slot = if player.active_slot <= 1 {
                    player.active_slot as usize
                } else if player.slots[1].is_none() {
                    1
                } else {
                    0
                };
                // Skip if same weapon already in slot
                if player.slots[slot] == Some(pickup.kind) {
                    continue;
                }
                player.slots[slot] = Some(pickup.kind);
                player.ammo[slot] = pickup.kind.magazine_size();
                player.reserve_ammo[slot] = pickup.kind.reserve_ammo();
                player.reload_timer = 0.0;
                player.fire_cooldown = 0.0;
                player.active_slot = slot as u8;
                net_entities.pickups.remove(&net_id.0);
                commands.entity(entity).despawn_recursive();
                sfx.send(SfxEvent::Hit);
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
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

fn health_collection(
    mut commands: Commands,
    pickups: Query<(Entity, &Transform, &NetId), With<HealthPickup>>,
    mut players: Query<(&Transform, &mut Player)>,
    mut net_entities: ResMut<NetEntities>,
    mut sfx: EventWriter<SfxEvent>,
) {
    for (p_t, mut player) in &mut players {
        if player.hp <= 0 || player.hp >= PLAYER_MAX_HP {
            continue;
        }
        let pp = p_t.translation.truncate();
        for (entity, pk_t, net_id) in &pickups {
            let d = pp.distance(pk_t.translation.truncate());
            if d < PLAYER_RADIUS + PICKUP_PICK_RADIUS {
                player.hp = (player.hp + HEAL_AMOUNT).min(PLAYER_MAX_HP);
                net_entities.pickups.remove(&net_id.0);
                commands.entity(entity).despawn_recursive();
                sfx.send(SfxEvent::Heal);
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn health_respawn(
    mut commands: Commands,
    health_assets: Res<HealthPickupAssets>,
    obstacles: Res<MapObstacles>,
    existing: Query<(), With<HealthPickup>>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
    mut timer: ResMut<HealthRespawnTimer>,
    time: Res<Time>,
) {
    timer.0 -= time.delta_seconds();
    if timer.0 > 0.0 {
        return;
    }
    timer.0 = HEALTH_RESPAWN_INTERVAL;

    let count = existing.iter().count();
    if count >= TARGET_HEALTH_COUNT {
        return;
    }
    let mut rng = rand::thread_rng();
    let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
        return;
    };
    let net_id = ctx.alloc_pickup_id();
    let entity = spawn_health_entity(&mut commands, &health_assets, p, net_id);
    net_entities.pickups.insert(net_id, entity);
}

// ── Throwable pickup helpers ──────────────────────────────────────

pub fn spawn_throwable_pickup_entity(
    commands: &mut Commands,
    assets: &ThrowableAssets,
    pos: Vec2,
    kind: ThrowableKind,
    count: u32,
    net_id: u32,
) -> Entity {
    let texture = match kind {
        ThrowableKind::Grenade => assets.grenade.clone(),
        ThrowableKind::Smoke => assets.smoke.clone(),
        ThrowableKind::Molotov => assets.molotov.clone(),
    };
    commands
        .spawn((
            SpriteBundle {
                texture,
                sprite: Sprite {
                    custom_size: Some(THROWABLE_SPRITE_SIZE),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 0.5),
                ..default()
            },
            ThrowablePickup { kind, count },
            NetId(net_id),
        ))
        .id()
}

fn throwable_collection(
    mut commands: Commands,
    pickups: Query<(Entity, &Transform, &ThrowablePickup, &NetId)>,
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
            if d < PLAYER_RADIUS + PICKUP_PICK_RADIUS {
                if player.throwable_kind == pickup.kind {
                    player.throwable_count += pickup.count;
                } else {
                    player.throwable_kind = pickup.kind;
                    player.throwable_count = pickup.count;
                }
                net_entities.pickups.remove(&net_id.0);
                commands.entity(entity).despawn_recursive();
                sfx.send(SfxEvent::Hit);
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn throwable_respawn(
    mut commands: Commands,
    throwable_assets: Res<ThrowableAssets>,
    obstacles: Res<MapObstacles>,
    existing: Query<(), With<ThrowablePickup>>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
    mut timer: ResMut<ThrowableRespawnTimer>,
    time: Res<Time>,
) {
    timer.0 -= time.delta_seconds();
    if timer.0 > 0.0 {
        return;
    }
    timer.0 = THROWABLE_RESPAWN_INTERVAL;

    let count = existing.iter().count();
    if count >= TARGET_THROWABLE_COUNT {
        return;
    }
    let mut rng = rand::thread_rng();
    let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
        return;
    };
    let kind = pick_throwable(&mut rng);
    let num = rng.gen_range(1..=2);
    let net_id = ctx.alloc_pickup_id();
    let entity = spawn_throwable_pickup_entity(&mut commands, &throwable_assets, p, kind, num, net_id);
    net_entities.pickups.insert(net_id, entity);
}

// ── Weapon pickup sprites ──────────────────────────────────────────

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

fn build_rocket_launcher_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let tube: Rgba = [60, 62, 52, 255];
    let tube_light: Rgba = [104, 108, 90, 255];
    let tube_dark: Rgba = [30, 32, 24, 255];
    let stripe: Rgba = [220, 60, 30, 255];
    let grip: Rgba = [48, 28, 12, 255];
    let grip_light: Rgba = [84, 52, 22, 255];
    let sight: Rgba = [20, 22, 26, 255];
    let muzzle: Rgba = [12, 12, 14, 255];
    let warhead: Rgba = [230, 80, 30, 255];

    let mut c = Canvas::new(32, 12);

    c.fill_rect(3, 3, 26, 5, outline);
    c.fill_rect(4, 4, 24, 3, tube);
    c.fill_rect(4, 4, 24, 1, tube_light);
    c.fill_rect(4, 6, 24, 1, tube_dark);

    c.fill_rect(10, 4, 3, 3, stripe);
    c.fill_rect(18, 4, 3, 3, stripe);

    c.fill_rect(26, 3, 3, 5, outline);
    c.put(27, 4, warhead);
    c.put(28, 4, warhead);
    c.put(28, 5, muzzle);
    c.put(27, 5, muzzle);

    c.fill_rect(2, 3, 2, 5, outline);
    c.put(1, 5, muzzle);

    c.fill_rect(13, 1, 5, 2, outline);
    c.fill_rect(14, 2, 3, 1, sight);

    c.fill_rect(8, 7, 4, 4, outline);
    c.fill_rect(9, 8, 2, 3, grip);
    c.put(9, 8, grip_light);

    c.fill_rect(19, 7, 3, 2, outline);
    c.put(20, 8, grip_light);

    c.into_image()
}

fn build_minigun_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let body: Rgba = [48, 50, 56, 255];
    let body_light: Rgba = [80, 84, 92, 255];
    let body_dark: Rgba = [24, 26, 30, 255];
    let barrel_light: Rgba = [66, 70, 78, 255];
    let barrel: Rgba = [38, 40, 46, 255];
    let grip: Rgba = [52, 34, 16, 255];
    let grip_light: Rgba = [82, 54, 24, 255];
    let ammo: Rgba = [180, 155, 40, 255];
    let tip: Rgba = [14, 14, 16, 255];

    let mut c = Canvas::new(30, 12);

    // Main body housing
    c.fill_rect(6, 3, 14, 6, outline);
    c.fill_rect(7, 4, 12, 4, body);
    c.fill_rect(7, 4, 12, 1, body_light);
    c.fill_rect(7, 7, 12, 1, body_dark);

    // Upper barrel cluster
    c.fill_rect(19, 3, 10, 2, outline);
    c.fill_rect(20, 3, 8, 1, barrel_light);
    c.fill_rect(20, 4, 8, 1, barrel);
    // Lower barrel cluster
    c.fill_rect(19, 6, 10, 2, outline);
    c.fill_rect(20, 6, 8, 1, barrel_light);
    c.fill_rect(20, 7, 8, 1, barrel);
    // Muzzle tips
    c.put(28, 3, tip);
    c.put(28, 4, tip);
    c.put(28, 6, tip);
    c.put(28, 7, tip);

    // Rear housing
    c.fill_rect(2, 3, 5, 6, outline);
    c.fill_rect(3, 4, 3, 4, body);
    c.put(3, 4, body_light);

    // Grip
    c.fill_rect(8, 8, 4, 3, outline);
    c.fill_rect(9, 9, 2, 2, grip);
    c.put(9, 9, grip_light);

    // Ammo belt
    c.put(4, 8, ammo);
    c.put(5, 9, ammo);
    c.put(6, 8, ammo);
    c.put(7, 9, ammo);

    c.into_image()
}

fn build_flamethrower_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let tank: Rgba = [68, 72, 60, 255];
    let tank_light: Rgba = [100, 106, 88, 255];
    let tank_dark: Rgba = [38, 42, 32, 255];
    let nozzle: Rgba = [42, 44, 50, 255];
    let nozzle_light: Rgba = [72, 76, 84, 255];
    let pipe: Rgba = [52, 54, 48, 255];
    let flame1: Rgba = [255, 180, 40, 255];
    let flame2: Rgba = [255, 100, 20, 255];
    let grip: Rgba = [48, 30, 14, 255];

    let mut c = Canvas::new(30, 12);

    // Fuel tank
    c.fill_rect(2, 2, 10, 8, outline);
    c.fill_rect(3, 3, 8, 6, tank_dark);
    c.fill_rect(3, 3, 8, 4, tank);
    c.fill_rect(3, 3, 8, 1, tank_light);

    // Connecting pipe
    c.fill_rect(11, 4, 5, 3, outline);
    c.fill_rect(12, 5, 3, 1, pipe);

    // Nozzle
    c.fill_rect(15, 3, 10, 5, outline);
    c.fill_rect(16, 4, 8, 3, nozzle);
    c.fill_rect(16, 4, 8, 1, nozzle_light);

    // Flame tip
    c.fill_rect(24, 3, 4, 5, outline);
    c.put(25, 4, flame1);
    c.put(26, 4, flame1);
    c.put(25, 5, flame2);
    c.put(26, 5, flame2);
    c.put(25, 6, flame1);
    c.put(26, 6, flame1);
    c.put(27, 5, flame2);

    // Grip
    c.fill_rect(13, 7, 4, 4, outline);
    c.fill_rect(14, 8, 2, 3, grip);

    c.into_image()
}

fn build_sniper_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let barrel_light: Rgba = [56, 60, 68, 255];
    let barrel_dark: Rgba = [12, 14, 18, 255];
    let wood: Rgba = [62, 38, 16, 255];
    let wood_light: Rgba = [96, 62, 26, 255];
    let wood_dark: Rgba = [34, 20, 8, 255];
    let scope: Rgba = [16, 18, 22, 255];
    let scope_light: Rgba = [48, 54, 62, 255];
    let scope_rim: Rgba = [38, 42, 50, 255];
    let tip: Rgba = [10, 10, 12, 255];
    let bipod: Rgba = [36, 38, 44, 255];

    let mut c = Canvas::new(36, 12);

    // Long barrel
    c.fill_rect(12, 5, 23, 2, outline);
    c.fill_rect(13, 5, 21, 1, barrel_light);
    c.fill_rect(13, 6, 21, 1, barrel_dark);

    // Muzzle brake
    c.fill_rect(33, 4, 3, 4, outline);
    c.put(34, 5, tip);
    c.put(35, 5, tip);
    c.put(34, 6, tip);

    // Large scope
    c.fill_rect(14, 1, 10, 4, outline);
    c.fill_rect(15, 2, 8, 2, scope);
    c.put(16, 2, scope_light);
    c.put(18, 2, scope_light);
    c.put(20, 2, scope_light);
    c.put(14, 4, scope_rim);
    c.put(23, 4, scope_rim);

    // Stock
    c.fill_rect(1, 4, 12, 4, outline);
    c.fill_rect(2, 5, 10, 2, wood);
    c.fill_rect(2, 5, 10, 1, wood_light);
    c.fill_rect(2, 6, 10, 1, wood_dark);

    // Cheek rest
    c.fill_rect(1, 4, 4, 2, outline);
    c.put(2, 4, wood_light);
    c.put(3, 4, wood);

    // Grip
    c.fill_rect(12, 7, 3, 4, outline);
    c.put(13, 8, wood);
    c.put(13, 9, wood_dark);

    // Bipod
    c.fill_rect(22, 7, 2, 4, outline);
    c.put(22, 8, bipod);
    c.put(23, 8, bipod);
    c.put(22, 10, bipod);
    c.put(23, 10, bipod);

    c.into_image()
}

// ── Health pickup sprite ───────────────────────────────────────────

fn build_health_pickup_image() -> Image {
    let outline: Rgba = [8, 36, 8, 255];
    let box_main: Rgba = [34, 148, 40, 255];
    let box_light: Rgba = [52, 186, 58, 255];
    let box_dark: Rgba = [22, 96, 26, 255];
    let cross: Rgba = [245, 245, 240, 255];
    let cross_hi: Rgba = [255, 255, 250, 255];
    let latch: Rgba = [160, 135, 55, 255];

    let mut c = Canvas::new(16, 12);

    // Box body
    c.fill_rect(1, 2, 14, 8, outline);
    c.fill_rect(2, 3, 12, 6, box_dark);
    c.fill_rect(2, 3, 12, 4, box_main);
    c.fill_rect(2, 3, 12, 1, box_light);

    // White cross - horizontal bar
    c.fill_rect(5, 5, 6, 2, cross);
    // White cross - vertical bar
    c.fill_rect(7, 4, 2, 4, cross);
    // Cross highlight
    c.put(7, 4, cross_hi);
    c.put(8, 4, cross_hi);

    // Latches
    c.fill_rect(6, 2, 4, 1, latch);
    c.fill_rect(6, 9, 4, 1, latch);

    c.into_image()
}

// ── New weapon sprites ────────────────────────────────────────────

fn build_uzi_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let body: Rgba = [34, 36, 42, 255];
    let body_light: Rgba = [62, 66, 74, 255];
    let body_dark: Rgba = [18, 20, 24, 255];
    let mag: Rgba = [26, 26, 30, 255];
    let grip: Rgba = [44, 28, 12, 255];
    let tip: Rgba = [14, 14, 16, 255];

    let mut c = Canvas::new(20, 12);
    c.fill_rect(4, 3, 12, 4, outline);
    c.fill_rect(5, 4, 10, 2, body);
    c.fill_rect(5, 4, 10, 1, body_light);
    c.fill_rect(5, 5, 10, 1, body_dark);
    c.fill_rect(15, 3, 4, 3, outline);
    c.put(16, 4, tip);
    c.put(17, 4, tip);
    c.fill_rect(7, 6, 4, 5, outline);
    c.fill_rect(8, 7, 2, 4, mag);
    c.fill_rect(11, 6, 3, 3, outline);
    c.put(12, 7, grip);
    c.fill_rect(2, 4, 3, 3, outline);
    c.put(3, 5, body_light);
    c.into_image()
}

fn build_auto_shotgun_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let barrel: Rgba = [32, 34, 40, 255];
    let barrel_light: Rgba = [62, 66, 74, 255];
    let barrel_dark: Rgba = [16, 18, 22, 255];
    let body: Rgba = [42, 44, 50, 255];
    let body_light: Rgba = [74, 78, 86, 255];
    let wood: Rgba = [68, 42, 18, 255];
    let wood_light: Rgba = [98, 64, 26, 255];
    let wood_dark: Rgba = [38, 22, 8, 255];
    let mag: Rgba = [28, 28, 32, 255];
    let tip: Rgba = [12, 12, 14, 255];

    let mut c = Canvas::new(30, 11);
    c.fill_rect(12, 3, 16, 3, outline);
    c.fill_rect(13, 3, 14, 1, barrel_light);
    c.fill_rect(13, 4, 14, 1, barrel);
    c.fill_rect(13, 5, 14, 1, barrel_dark);
    c.fill_rect(27, 3, 2, 3, outline);
    c.put(27, 4, tip);
    // Body/receiver
    c.fill_rect(8, 2, 6, 5, outline);
    c.fill_rect(9, 3, 4, 3, body);
    c.put(9, 3, body_light);
    // Drum magazine
    c.fill_rect(9, 6, 5, 4, outline);
    c.fill_rect(10, 7, 3, 2, mag);
    // Stock
    c.fill_rect(1, 3, 8, 4, outline);
    c.fill_rect(2, 4, 6, 2, wood);
    c.fill_rect(2, 4, 6, 1, wood_light);
    c.fill_rect(2, 5, 6, 1, wood_dark);
    c.into_image()
}

fn build_marksman_rifle_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let barrel: Rgba = [30, 32, 38, 255];
    let barrel_light: Rgba = [58, 62, 70, 255];
    let barrel_dark: Rgba = [12, 14, 18, 255];
    let body: Rgba = [44, 46, 52, 255];
    let body_light: Rgba = [72, 76, 84, 255];
    let wood: Rgba = [56, 34, 14, 255];
    let wood_light: Rgba = [88, 56, 22, 255];
    let wood_dark: Rgba = [28, 16, 6, 255];
    let scope: Rgba = [16, 18, 22, 255];
    let scope_light: Rgba = [46, 52, 60, 255];
    let tip: Rgba = [10, 10, 12, 255];

    let mut c = Canvas::new(32, 11);
    // Barrel
    c.fill_rect(14, 4, 16, 2, outline);
    c.fill_rect(15, 4, 14, 1, barrel_light);
    c.fill_rect(15, 5, 14, 1, barrel_dark);
    c.fill_rect(29, 4, 2, 2, outline);
    c.put(29, 4, tip);
    // Scope (smaller than sniper)
    c.fill_rect(16, 1, 7, 3, outline);
    c.fill_rect(17, 2, 5, 1, scope);
    c.put(18, 2, scope_light);
    c.put(20, 2, scope_light);
    // Receiver
    c.fill_rect(10, 3, 6, 5, outline);
    c.fill_rect(11, 4, 4, 3, body);
    c.put(11, 4, body_light);
    // Magazine
    c.fill_rect(12, 7, 3, 3, outline);
    c.put(13, 8, barrel);
    // Stock
    c.fill_rect(1, 3, 10, 4, outline);
    c.fill_rect(2, 4, 8, 2, wood);
    c.fill_rect(2, 4, 8, 1, wood_light);
    c.fill_rect(2, 5, 8, 1, wood_dark);
    c.into_image()
}

// ── Throwable sprites ─────────────────────────────────────────────

fn build_grenade_image() -> Image {
    let outline: Rgba = [10, 12, 8, 255];
    let body: Rgba = [62, 72, 48, 255];
    let body_light: Rgba = [88, 100, 68, 255];
    let body_dark: Rgba = [38, 46, 28, 255];
    let lever: Rgba = [140, 135, 110, 255];
    let pin: Rgba = [180, 170, 130, 255];

    let mut c = Canvas::new(11, 11);
    c.fill_circle(5, 6, 4, outline);
    c.fill_circle(5, 6, 3, body_dark);
    c.fill_circle(5, 6, 2, body);
    c.put(4, 5, body_light);
    // Top
    c.fill_rect(4, 1, 3, 2, outline);
    c.put(5, 2, lever);
    c.put(6, 1, pin);
    c.into_image()
}

fn build_smoke_grenade_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let body: Rgba = [100, 105, 115, 255];
    let body_light: Rgba = [140, 145, 155, 255];
    let body_dark: Rgba = [60, 64, 70, 255];
    let band: Rgba = [180, 180, 190, 255];
    let top: Rgba = [70, 74, 80, 255];

    let mut c = Canvas::new(11, 11);
    c.fill_rect(2, 3, 7, 7, outline);
    c.fill_rect(3, 4, 5, 5, body_dark);
    c.fill_rect(3, 4, 5, 3, body);
    c.fill_rect(3, 4, 5, 1, body_light);
    c.fill_rect(3, 6, 5, 1, band);
    c.fill_rect(3, 1, 5, 3, outline);
    c.fill_rect(4, 2, 3, 1, top);
    c.into_image()
}

fn build_molotov_image() -> Image {
    let outline: Rgba = [10, 8, 4, 255];
    let glass: Rgba = [80, 120, 60, 180];
    let glass_light: Rgba = [120, 160, 90, 200];
    let liquid: Rgba = [180, 100, 30, 220];
    let wick: Rgba = [200, 180, 120, 255];
    let flame: Rgba = [255, 180, 40, 255];
    let flame_tip: Rgba = [255, 100, 20, 255];

    let mut c = Canvas::new(11, 14);
    // Bottle body
    c.fill_rect(3, 6, 5, 6, outline);
    c.fill_rect(4, 7, 3, 4, glass);
    c.put(4, 7, glass_light);
    c.fill_rect(4, 9, 3, 2, liquid);
    // Neck
    c.fill_rect(4, 3, 3, 4, outline);
    c.put(5, 4, glass);
    c.put(5, 5, glass);
    // Wick
    c.put(5, 2, wick);
    c.put(5, 1, flame);
    c.put(5, 0, flame_tip);
    c.put(6, 1, flame_tip);
    c.into_image()
}

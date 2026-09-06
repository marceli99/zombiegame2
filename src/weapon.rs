use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::achievements::AchievementTracker;
use crate::audio::SfxEvent;
use crate::map::{is_walkable_tile, tile_center, MapObstacles, MAP_COLS, MAP_ROWS};
use crate::net::{is_authoritative, NetContext, NetEntities, NetId};
use crate::pixelart::{Canvas, Rgba};
use crate::player::{Player, PLAYER_ARMOR_MAX, PLAYER_MAX_HP, PLAYER_RADIUS};
use crate::{gameplay_active, GameState};

const PICKUP_SPRITE_SIZE: Vec2 = Vec2::new(30.0, 16.0);
const PICKUP_PICK_RADIUS: f32 = 16.0;
/// Number of weapon pickups placed across the whole 5-segment world.  At ~5
/// per segment a player who has unlocked all 5 finds a healthy spread of
/// each tier; lower-tier segments still feel hand-armable too.
const TARGET_PICKUP_COUNT: usize = 28;
const RESPAWN_INTERVAL: f32 = 5.0;

const HEALTH_SPRITE_SIZE: Vec2 = Vec2::new(22.0, 16.0);
const TARGET_HEALTH_COUNT: usize = 6;
const HEALTH_RESPAWN_INTERVAL: f32 = 8.0;
const HEAL_AMOUNT: i32 = 30;

const ARMOR_SPRITE_SIZE: Vec2 = Vec2::new(22.0, 18.0);
const TARGET_ARMOR_COUNT: usize = 3;
const ARMOR_RESPAWN_INTERVAL: f32 = 18.0;

const MONEY_SPRITE_SIZE: Vec2 = Vec2::new(22.0, 16.0);
const TARGET_MONEY_COUNT: usize = 2;
const MONEY_RESPAWN_INTERVAL: f32 = 25.0;
pub const MONEY_MULT_DURATION: f32 = 30.0;

pub const HEALTH_PICKUP_KIND: u8 = 255;
pub const ARMOR_PICKUP_KIND: u8 = 254;
pub const MONEY2X_PICKUP_KIND: u8 = 253;
pub const MONEY3X_PICKUP_KIND: u8 = 252;

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
    AssaultRifle = 11,
    GrenadeLauncher = 12,
    /// "Pistolet ppanc" — armor-piercing pistol, fully automatic, 36-round mag.
    AntiTankPistol = 13,
    /// "Obrzyn" — sawed-off shotgun, 4 shells, brutal damage, huge spread.
    SawedOff = 14,
    /// "Rewolwer" — 6-shot revolver, slow but devastating per-hit.
    Revolver = 15,
    /// Fast rapid-fire grenade launcher with 10-round drum.
    AutoGrenadeLauncher = 16,
    /// Homing RPG that locks onto the nearest enemy at fire time and steers
    /// toward them.
    HomingRpg = 17,
}

pub const WEAPON_COUNT: usize = 18;

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
            11 => Weapon::AssaultRifle,
            12 => Weapon::GrenadeLauncher,
            13 => Weapon::AntiTankPistol,
            14 => Weapon::SawedOff,
            15 => Weapon::Revolver,
            16 => Weapon::AutoGrenadeLauncher,
            17 => Weapon::HomingRpg,
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
            Weapon::AssaultRifle => 0.10,
            Weapon::GrenadeLauncher => 0.85,
            Weapon::AntiTankPistol => 0.07,
            Weapon::SawedOff => 0.65,
            Weapon::Revolver => 0.45,
            Weapon::AutoGrenadeLauncher => 0.22,
            Weapon::HomingRpg => 1.5,
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
            Weapon::AssaultRifle => 4,
            Weapon::GrenadeLauncher => 0,
            Weapon::AntiTankPistol => 4,
            Weapon::SawedOff => 5,
            Weapon::Revolver => 14,
            Weapon::AutoGrenadeLauncher => 0,
            Weapon::HomingRpg => 0,
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
            Weapon::Flamethrower => 360.0,
            Weapon::Sniper => 1400.0,
            Weapon::Uzi => 780.0,
            Weapon::AutoShotgun => 580.0,
            Weapon::MarksmanRifle => 1200.0,
            Weapon::AssaultRifle => 980.0,
            Weapon::GrenadeLauncher => 460.0,
            Weapon::AntiTankPistol => 950.0,
            Weapon::SawedOff => 580.0,
            Weapon::Revolver => 920.0,
            Weapon::AutoGrenadeLauncher => 540.0,
            Weapon::HomingRpg => 480.0,
        }
    }

    pub fn bullet_count(self) -> u32 {
        match self {
            Weapon::Shotgun => 6,
            Weapon::AutoShotgun => 4,
            // Flamethrower spits out a thick cone of small flame puffs —
            // more particles per pull makes the stream feel real.
            Weapon::Flamethrower => 5,
            Weapon::SawedOff => 8,
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
            Weapon::Flamethrower => 0.45,
            Weapon::Sniper => 0.0,
            Weapon::Uzi => 0.12,
            Weapon::AutoShotgun => 0.26,
            Weapon::MarksmanRifle => 0.0,
            Weapon::AssaultRifle => 0.04,
            Weapon::GrenadeLauncher => 0.02,
            Weapon::AntiTankPistol => 0.06,
            Weapon::SawedOff => 0.55,
            Weapon::Revolver => 0.0,
            Weapon::AutoGrenadeLauncher => 0.04,
            Weapon::HomingRpg => 0.0,
        }
    }

    pub fn is_rocket(self) -> bool {
        matches!(
            self,
            Weapon::RocketLauncher
                | Weapon::GrenadeLauncher
                | Weapon::AutoGrenadeLauncher
                | Weapon::HomingRpg,
        )
    }

    /// True for rockets that lock onto the nearest enemy and steer toward it.
    pub fn is_homing(self) -> bool {
        matches!(self, Weapon::HomingRpg)
    }

    /// Flamethrower-style projectile — short-range flame puffs that grow
    /// and fade as they travel.  Different rendering from regular bullets.
    pub fn is_flame(self) -> bool {
        matches!(self, Weapon::Flamethrower)
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
            Weapon::AssaultRifle => "ASSAULT RIFLE",
            Weapon::GrenadeLauncher => "GRENADE LAUNCHER",
            Weapon::AntiTankPistol => "AP PISTOL",
            Weapon::SawedOff => "OBRZYN",
            Weapon::Revolver => "REVOLVER",
            Weapon::AutoGrenadeLauncher => "AUTO GL",
            Weapon::HomingRpg => "HOMING RPG",
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
            Weapon::AssaultRifle => 30,
            Weapon::GrenadeLauncher => 4,
            Weapon::AntiTankPistol => 36,
            Weapon::SawedOff => 4,
            Weapon::Revolver => 6,
            Weapon::AutoGrenadeLauncher => 10,
            Weapon::HomingRpg => 4,
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
            Weapon::AssaultRifle => 1500,
            Weapon::GrenadeLauncher => 96,
            Weapon::AntiTankPistol => 1080,
            Weapon::SawedOff => 64,
            Weapon::Revolver => 90,
            Weapon::AutoGrenadeLauncher => 120,
            Weapon::HomingRpg => 60,
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
            Weapon::AssaultRifle => 1.8,
            Weapon::GrenadeLauncher => 3.0,
            Weapon::AntiTankPistol => 1.6,
            Weapon::SawedOff => 1.8,
            Weapon::Revolver => 2.2,
            Weapon::AutoGrenadeLauncher => 3.2,
            Weapon::HomingRpg => 3.0,
        }
    }

    pub fn has_infinite_ammo(self) -> bool {
        matches!(self, Weapon::Pistol)
    }

    /// Tier 1..=5; weapon spawns only in segments whose id >= tier.
    /// Tier 5 weapons (best) only appear in segment 5 (Military).
    pub fn tier(self) -> u8 {
        match self {
            Weapon::Pistol => 1,
            Weapon::Smg | Weapon::Shotgun | Weapon::Uzi | Weapon::SawedOff => 2,
            Weapon::Rifle
            | Weapon::AutoShotgun
            | Weapon::AssaultRifle
            | Weapon::Revolver => 3,
            Weapon::MarksmanRifle
            | Weapon::Flamethrower
            | Weapon::AntiTankPistol => 4,
            Weapon::Sniper
            | Weapon::RocketLauncher
            | Weapon::Minigun
            | Weapon::GrenadeLauncher
            | Weapon::AutoGrenadeLauncher
            | Weapon::HomingRpg => 5,
        }
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
    pub fn label(self) -> &'static str {
        match self {
            Self::Grenade => "GRENADE",
            Self::Smoke => "SMOKE",
            Self::Molotov => "MOLOTOV",
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

/// Updated each frame from `refresh_pickup_prompt` — the HUD reads this
/// to show a "PRESS E" hint over the screen when the local player is
/// standing on a weapon pickup but already has something in slot 2.
#[derive(Resource, Default, Clone, Copy)]
pub struct PickupPromptHint {
    pub weapon: Option<Weapon>,
}

#[derive(Component)]
pub struct HealthPickup;

#[derive(Component)]
pub struct ArmorPickup;

#[derive(Component)]
pub struct MoneyMultPickup {
    pub factor: u8, // 2 or 3
}

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
pub struct ExtraPickupAssets {
    pub health: Handle<Image>,
    pub armor: Handle<Image>,
    pub money_2x: Handle<Image>,
    pub money_3x: Handle<Image>,
}

#[derive(Resource)]
struct ArmorRespawnTimer(f32);

impl Default for ArmorRespawnTimer {
    fn default() -> Self {
        Self(ARMOR_RESPAWN_INTERVAL)
    }
}

#[derive(Resource)]
struct MoneyRespawnTimer(f32);

impl Default for MoneyRespawnTimer {
    fn default() -> Self {
        Self(MONEY_RESPAWN_INTERVAL)
    }
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
            .init_resource::<ArmorRespawnTimer>()
            .init_resource::<MoneyRespawnTimer>()
            .init_resource::<PickupPromptHint>()
            .add_systems(
                Startup,
                (
                    setup_weapon_assets,
                    setup_throwable_assets,
                    setup_extra_pickup_assets,
                ),
            )
            .add_systems(
                OnEnter(GameState::Playing),
                initial_pickup_spawn.run_if(is_authoritative),
            )
            .add_systems(OnExit(GameState::Playing), despawn_all_pickups)
            // pickup_collection consumes the interact flag when swapping a
            // held weapon, so it joins the InteractConsumers set — the
            // post-tick `clear_interact_flag` system in MapPlugin runs
            // strictly after this set.
            .add_systems(
                FixedUpdate,
                pickup_collection
                    .in_set(crate::map::InteractConsumers)
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            )
            .add_systems(
                FixedUpdate,
                (
                    pickup_respawn,
                    health_collection,
                    health_respawn,
                    throwable_collection,
                    throwable_respawn,
                    armor_collection,
                    armor_respawn,
                    money_collection,
                    money_respawn,
                )
                    .chain()
                    .after(pickup_collection)
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            )
            // Pickup prompt runs in Update so the HUD updates per-frame
            // for the local player; it reads the public WeaponPickup
            // positions which are replicated to clients.
            .add_systems(
                Update,
                (refresh_pickup_prompt, animate_weapon_pickups)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

/// Slow Z-rotation + alpha pulse so weapon pickups stand out against the
/// world props.  Phase is offset by the Bevy entity index so multiple
/// pickups in view don't sway in lockstep.
fn animate_weapon_pickups(
    time: Res<Time>,
    mut q: Query<(Entity, &mut Transform, &mut Sprite), With<WeaponPickup>>,
) {
    // Wrapped variant: plain `elapsed_seconds()` loses sub-frame precision
    // after hours of uptime, turning these phases visibly steppy.
    let t = time.elapsed_seconds_wrapped();
    for (entity, mut transform, mut sprite) in &mut q {
        let phase = (entity.index() as f32) * 0.37;
        transform.rotation = Quat::from_rotation_z(t * 0.7 + phase);
        let pulse = (t * 2.4 + phase).sin() * 0.5 + 0.5;
        // Slight scale bob + warm glow tint that strengthens with the
        // pulse — reads as a soft halo without an actual light shader.
        let scale = 1.0 + pulse * 0.08;
        transform.scale = Vec3::new(scale, scale, 1.0);
        let glow = 1.0 + pulse * 0.25;
        sprite.color = Color::srgba(glow, glow * 0.96, glow * 0.78, 1.0);
    }
}

/// Refreshes the `PickupPromptHint` for the local player every frame.
/// Sets the `weapon` field whenever the player overlaps a weapon pickup
/// AND already has something in slot 2 (so a swap requires E).  Clears
/// otherwise.
fn refresh_pickup_prompt(
    mut hint: ResMut<PickupPromptHint>,
    pickups: Query<(&Transform, &WeaponPickup)>,
    players: Query<(&Transform, &Player)>,
    ctx: Res<crate::net::NetContext>,
) {
    let local = players.iter().find(|(_, p)| p.id == ctx.my_id);
    let Some((p_t, p)) = local else {
        hint.weapon = None;
        return;
    };
    if p.hp <= 0 {
        hint.weapon = None;
        return;
    }
    // Slot 2 empty → auto-pickup, no prompt needed.
    if p.slots[1].is_none() {
        hint.weapon = None;
        return;
    }
    let pp = p_t.translation.truncate();
    let mut nearest: Option<Weapon> = None;
    let r = PLAYER_RADIUS + PICKUP_PICK_RADIUS;
    let mut best_d2 = r * r;
    for (pk_t, pickup) in pickups.iter() {
        let d2 = pp.distance_squared(pk_t.translation.truncate());
        if d2 < best_d2 && p.slots[1] != Some(pickup.kind) {
            best_d2 = d2;
            nearest = Some(pickup.kind);
        }
    }
    hint.weapon = nearest;
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
        images.add(build_assault_rifle_image()),
        images.add(build_grenade_launcher_image()),
        images.add(build_anti_tank_pistol_image()),
        images.add(build_sawed_off_image()),
        images.add(build_revolver_image()),
        images.add(build_auto_grenade_launcher_image()),
        images.add(build_homing_rpg_image()),
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

fn setup_extra_pickup_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(ExtraPickupAssets {
        health: images.add(build_health_pickup_image()),
        armor: images.add(build_armor_pickup_image()),
        money_2x: images.add(build_money_pickup_image(2)),
        money_3x: images.add(build_money_pickup_image(3)),
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
    (Weapon::AssaultRifle, 9),
    (Weapon::GrenadeLauncher, 5),
    (Weapon::AntiTankPistol, 7),
    (Weapon::SawedOff, 9),
    (Weapon::Revolver, 8),
    (Weapon::AutoGrenadeLauncher, 4),
    (Weapon::HomingRpg, 4),
];

/// Pick a weapon whose tier ≤ `seg_id`.  Each segment exposes the tiers
/// available to it: seg 1 only Pistol, seg 5 sees the entire arsenal.
fn pick_weapon<R: Rng>(rng: &mut R, seg_id: u8) -> Weapon {
    let candidates: Vec<&(Weapon, u32)> =
        WEIGHTS.iter().filter(|(w, _)| w.tier() <= seg_id).collect();
    let total: u32 = candidates.iter().map(|(_, w)| w).sum();
    if total == 0 {
        return Weapon::Pistol;
    }
    let mut roll = rng.gen_range(0..total);
    for (w, wt) in &candidates {
        if roll < *wt {
            return *w;
        }
        roll -= wt;
    }
    Weapon::Pistol
}

/// Map a world-x coord to its segment id (1..=5).
fn segment_for_world_x(world_x: f32) -> u8 {
    let (col, _) = crate::map::world_to_tile(Vec2::new(world_x, 0.0));
    ((col / crate::map::SEG_TILES) + 1).clamp(1, 5) as u8
}

fn pick_throwable<R: Rng>(rng: &mut R) -> ThrowableKind {
    match rng.gen_range(0..3) {
        0 => ThrowableKind::Grenade,
        1 => ThrowableKind::Smoke,
        _ => ThrowableKind::Molotov,
    }
}

/// Per-axis pixel offset added to a tile centre when placing a pickup,
/// so multiple pickups in the same tile don't overlap exactly.
const PICKUP_JITTER_RADIUS: f32 = 22.0;
/// Max tile picks before giving up — generous because the village map
/// has lots of unwalkable tiles (forest, building interiors).
const PICKUP_FIND_ATTEMPTS: usize = 80;
/// Half-extent of a "near spawn" exclusion zone — pickups don't spawn
/// here so the player isn't dropping into them on round start.
const PICKUP_SPAWN_EXCLUSION: f32 = 70.0;
/// Collision radius used for the obstacle hit-test when placing a pickup.
const PICKUP_FIT_RADIUS: f32 = 18.0;

fn find_pickup_spot<R: Rng>(rng: &mut R, obstacles: &MapObstacles) -> Option<Vec2> {
    for _ in 0..PICKUP_FIND_ATTEMPTS {
        let col = rng.gen_range(0..MAP_COLS);
        let row = rng.gen_range(0..MAP_ROWS);
        if !is_walkable_tile(col, row) {
            continue;
        }
        let center = tile_center(col, row);
        let p = Vec2::new(
            center.x + rng.gen_range(-PICKUP_JITTER_RADIUS..PICKUP_JITTER_RADIUS),
            center.y + rng.gen_range(-PICKUP_JITTER_RADIUS..PICKUP_JITTER_RADIUS),
        );
        if obstacles.hits(p, PICKUP_FIT_RADIUS) {
            continue;
        }
        if p.x.abs() < PICKUP_SPAWN_EXCLUSION && p.y.abs() < PICKUP_SPAWN_EXCLUSION {
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
    assets: &ExtraPickupAssets,
    pos: Vec2,
    net_id: u32,
) -> Entity {
    commands
        .spawn((
            SpriteBundle {
                texture: assets.health.clone(),
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
    extra_assets: Res<ExtraPickupAssets>,
    throwable_assets: Res<ThrowableAssets>,
    obstacles: Res<MapObstacles>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
    mut timer: ResMut<PickupRespawnTimer>,
    mut health_timer: ResMut<HealthRespawnTimer>,
    mut armor_timer: ResMut<ArmorRespawnTimer>,
    mut money_timer: ResMut<MoneyRespawnTimer>,
) {
    timer.0 = RESPAWN_INTERVAL;
    health_timer.0 = HEALTH_RESPAWN_INTERVAL;
    armor_timer.0 = ARMOR_RESPAWN_INTERVAL;
    money_timer.0 = MONEY_RESPAWN_INTERVAL;
    let mut rng = StdRng::seed_from_u64(0xC0FFEE);
    for _ in 0..TARGET_PICKUP_COUNT {
        let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
            continue;
        };
        let seg_id = segment_for_world_x(p.x);
        let kind = pick_weapon(&mut rng, seg_id);
        let net_id = ctx.alloc_pickup_id();
        let entity = spawn_pickup_entity(&mut commands, &assets, p, kind, net_id);
        net_entities.pickups.insert(net_id, entity);
    }
    for _ in 0..TARGET_HEALTH_COUNT {
        let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
            continue;
        };
        let net_id = ctx.alloc_pickup_id();
        let entity = spawn_health_entity(&mut commands, &extra_assets, p, net_id);
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
    for _ in 0..TARGET_ARMOR_COUNT {
        let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
            continue;
        };
        let net_id = ctx.alloc_pickup_id();
        let entity = spawn_armor_entity(&mut commands, &extra_assets, p, net_id);
        net_entities.pickups.insert(net_id, entity);
    }
    for _ in 0..TARGET_MONEY_COUNT {
        let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
            continue;
        };
        let factor = if rng.gen_bool(0.5) { 2 } else { 3 };
        let net_id = ctx.alloc_pickup_id();
        let entity = spawn_money_entity(&mut commands, &extra_assets, p, factor, net_id);
        net_entities.pickups.insert(net_id, entity);
    }
}

#[allow(clippy::type_complexity)]
fn despawn_all_pickups(
    mut commands: Commands,
    q: Query<
        Entity,
        Or<(
            With<WeaponPickup>,
            With<HealthPickup>,
            With<ThrowablePickup>,
            With<ArmorPickup>,
            With<MoneyMultPickup>,
        )>,
    >,
    mut net_entities: ResMut<NetEntities>,
) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
    net_entities.pickups.clear();
}

#[allow(clippy::too_many_arguments)]
fn pickup_collection(
    mut commands: Commands,
    pickups: Query<(Entity, &Transform, &WeaponPickup, &NetId)>,
    mut players: Query<(&Transform, &mut Player)>,
    mut net_entities: ResMut<NetEntities>,
    mut sfx: EventWriter<SfxEvent>,
    mut local: ResMut<crate::net::LocalInput>,
    remote: Res<crate::net::RemoteInputs>,
    ctx: Res<crate::net::NetContext>,
    mut tracker: ResMut<AchievementTracker>,
) {
    for (p_t, mut player) in &mut players {
        if player.hp <= 0 {
            continue;
        }
        let pp = p_t.translation.truncate();
        // Did this specific player press E this tick?  Local player reads
        // its own latched flag; remote players read their replicated input.
        // We only consume the flag below if the swap actually fires, so an
        // E press near a non-pickup spot still rolls forward to gate /
        // staircase systems.
        let want_swap = if player.id == ctx.my_id {
            local.0.interact
        } else {
            remote.0.get(&player.id).map(|i| i.interact).unwrap_or(false)
        };
        for (entity, pk_t, pickup, net_id) in &pickups {
            let r = PLAYER_RADIUS + PICKUP_PICK_RADIUS;
            let d2 = pp.distance_squared(pk_t.translation.truncate());
            if d2 < r * r {
                // Already holding this exact weapon — skip silently so we
                // don't keep "picking it up".
                if player.slots[1] == Some(pickup.kind) {
                    continue;
                }
                // Slot 2 occupied with something else — only swap on an
                // explicit E press so the player doesn't lose a weapon by
                // accident.  Slot 2 empty: auto-pickup as before.
                if player.slots[1].is_some() && !want_swap {
                    continue;
                }
                // Claim the pickup by removing its net-id first: the
                // despawn below is deferred, so a second player overlapping
                // the same pickup this tick still sees the entity — the
                // failed remove tells us it was already collected.
                if net_entities.pickups.remove(&net_id.0).is_none() {
                    continue;
                }
                player.slots[1] = Some(pickup.kind);
                player.ammo[1] = pickup.kind.magazine_size();
                player.reserve_ammo[1] = pickup.kind.reserve_ammo();
                player.reload_timer = 0.0;
                // Mirror the slot-switch swap delay (player.rs ~line 929) so
                // grabbing a new weapon doesn't let the player fire on the
                // same frame they ran over the pickup.
                player.fire_cooldown = 0.15;
                player.active_slot = 1;
                if player.id == ctx.my_id {
                    local.0.interact = false;
                    // FullArsenal progress — only weapons the local player
                    // picked up themselves count toward the achievement.
                    tracker.weapons_held |= 1u32 << pickup.kind.as_u8();
                }
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
    let seg_id = segment_for_world_x(p.x);
    let kind = pick_weapon(&mut rng, seg_id);
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
            let r = PLAYER_RADIUS + PICKUP_PICK_RADIUS;
            let d2 = pp.distance_squared(pk_t.translation.truncate());
            if d2 < r * r {
                // Skip pickups another player already claimed this tick —
                // the deferred despawn hasn't applied yet.
                if net_entities.pickups.remove(&net_id.0).is_none() {
                    continue;
                }
                player.hp = (player.hp + HEAL_AMOUNT).min(PLAYER_MAX_HP);
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
    extra_assets: Res<ExtraPickupAssets>,
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
    let entity = spawn_health_entity(&mut commands, &extra_assets, p, net_id);
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
            let r = PLAYER_RADIUS + PICKUP_PICK_RADIUS;
            let d2 = pp.distance_squared(pk_t.translation.truncate());
            if d2 < r * r {
                // Skip pickups another player already claimed this tick.
                if net_entities.pickups.remove(&net_id.0).is_none() {
                    continue;
                }
                if player.throwable_kind == pickup.kind {
                    player.throwable_count += pickup.count;
                } else {
                    player.throwable_kind = pickup.kind;
                    player.throwable_count = pickup.count;
                }
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
    let num = rng.gen_range(1..=3);
    let net_id = ctx.alloc_pickup_id();
    let entity = spawn_throwable_pickup_entity(&mut commands, &throwable_assets, p, kind, num, net_id);
    net_entities.pickups.insert(net_id, entity);
}

// ── Armor pickup ─────────────────────────────────────────────────

pub fn spawn_armor_entity(
    commands: &mut Commands,
    assets: &ExtraPickupAssets,
    pos: Vec2,
    net_id: u32,
) -> Entity {
    commands
        .spawn((
            SpriteBundle {
                texture: assets.armor.clone(),
                sprite: Sprite {
                    custom_size: Some(ARMOR_SPRITE_SIZE),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 0.5),
                ..default()
            },
            ArmorPickup,
            NetId(net_id),
        ))
        .id()
}

fn armor_collection(
    mut commands: Commands,
    pickups: Query<(Entity, &Transform, &NetId), With<ArmorPickup>>,
    mut players: Query<(&Transform, &mut Player)>,
    mut net_entities: ResMut<NetEntities>,
    mut sfx: EventWriter<SfxEvent>,
) {
    for (p_t, mut player) in &mut players {
        if player.hp <= 0 || player.armor >= PLAYER_ARMOR_MAX {
            continue;
        }
        let pp = p_t.translation.truncate();
        for (entity, pk_t, net_id) in &pickups {
            let r = PLAYER_RADIUS + PICKUP_PICK_RADIUS;
            let d2 = pp.distance_squared(pk_t.translation.truncate());
            if d2 < r * r {
                // Skip pickups another player already claimed this tick.
                if net_entities.pickups.remove(&net_id.0).is_none() {
                    continue;
                }
                // Armor pickups refill the pool to full — picking one up
                // effectively doubles your HP, so the bar should snap to
                // max rather than incrementing in tiny steps.
                player.armor = PLAYER_ARMOR_MAX;
                commands.entity(entity).despawn_recursive();
                sfx.send(SfxEvent::Hit);
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn armor_respawn(
    mut commands: Commands,
    extra_assets: Res<ExtraPickupAssets>,
    obstacles: Res<MapObstacles>,
    existing: Query<(), With<ArmorPickup>>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
    mut timer: ResMut<ArmorRespawnTimer>,
    time: Res<Time>,
) {
    timer.0 -= time.delta_seconds();
    if timer.0 > 0.0 {
        return;
    }
    timer.0 = ARMOR_RESPAWN_INTERVAL;
    if existing.iter().count() >= TARGET_ARMOR_COUNT {
        return;
    }
    let mut rng = rand::thread_rng();
    let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
        return;
    };
    let net_id = ctx.alloc_pickup_id();
    let entity = spawn_armor_entity(&mut commands, &extra_assets, p, net_id);
    net_entities.pickups.insert(net_id, entity);
}

// ── Money multiplier pickup (2x/3x for 30s) ──────────────────────

pub fn spawn_money_entity(
    commands: &mut Commands,
    assets: &ExtraPickupAssets,
    pos: Vec2,
    factor: u8,
    net_id: u32,
) -> Entity {
    let tex = if factor >= 3 {
        assets.money_3x.clone()
    } else {
        assets.money_2x.clone()
    };
    commands
        .spawn((
            SpriteBundle {
                texture: tex,
                sprite: Sprite {
                    custom_size: Some(MONEY_SPRITE_SIZE),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 0.5),
                ..default()
            },
            MoneyMultPickup { factor },
            NetId(net_id),
        ))
        .id()
}

fn money_collection(
    mut commands: Commands,
    pickups: Query<(Entity, &Transform, &MoneyMultPickup, &NetId)>,
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
            let r = PLAYER_RADIUS + PICKUP_PICK_RADIUS;
            let d2 = pp.distance_squared(pk_t.translation.truncate());
            if d2 < r * r {
                // Skip pickups another player already claimed this tick.
                if net_entities.pickups.remove(&net_id.0).is_none() {
                    continue;
                }
                // Player-friendly: a weaker pickup never downgrades an active stronger
                // buff, but it still refreshes the timer so the strong buff lasts
                // longer. Natural expiry resets `money_mult` to 1 in `player.rs`.
                player.money_mult = pickup.factor.max(player.money_mult);
                player.money_mult_timer = MONEY_MULT_DURATION;
                commands.entity(entity).despawn_recursive();
                sfx.send(SfxEvent::Hit);
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn money_respawn(
    mut commands: Commands,
    extra_assets: Res<ExtraPickupAssets>,
    obstacles: Res<MapObstacles>,
    existing: Query<(), With<MoneyMultPickup>>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
    mut timer: ResMut<MoneyRespawnTimer>,
    time: Res<Time>,
) {
    timer.0 -= time.delta_seconds();
    if timer.0 > 0.0 {
        return;
    }
    timer.0 = MONEY_RESPAWN_INTERVAL;
    if existing.iter().count() >= TARGET_MONEY_COUNT {
        return;
    }
    let mut rng = rand::thread_rng();
    let Some(p) = find_pickup_spot(&mut rng, &obstacles) else {
        return;
    };
    let factor = if rng.gen_bool(0.5) { 2 } else { 3 };
    let net_id = ctx.alloc_pickup_id();
    let entity = spawn_money_entity(&mut commands, &extra_assets, p, factor, net_id);
    net_entities.pickups.insert(net_id, entity);
}

fn build_armor_pickup_image() -> Image {
    let outline: Rgba = [12, 14, 18, 255];
    let steel: Rgba = [120, 130, 150, 255];
    let steel_l: Rgba = [190, 200, 220, 255];
    let steel_d: Rgba = [70, 80, 100, 255];
    let buckle: Rgba = [220, 200, 90, 255];

    let w = 22;
    let h = 18;
    let mut c = Canvas::new(w, h);
    // Vest body — trapezoid
    c.fill_rect(4, 2, w - 8, h - 4, outline);
    c.fill_rect(5, 3, w - 10, h - 6, steel);
    c.fill_rect(5, 3, w - 10, 2, steel_l);
    c.fill_rect(5, h - 5, w - 10, 1, steel_d);
    // Shoulder straps
    c.fill_rect(3, 1, 4, 4, outline);
    c.fill_rect(w - 7, 1, 4, 4, outline);
    c.fill_rect(4, 2, 2, 2, steel_l);
    c.fill_rect(w - 6, 2, 2, 2, steel_l);
    // Buckle centre
    c.fill_rect(w / 2 - 2, h / 2 - 1, 4, 3, buckle);
    c.into_image()
}

fn build_money_pickup_image(factor: u8) -> Image {
    let outline: Rgba = [12, 14, 8, 255];
    let bill: Rgba = [70, 150, 80, 255];
    let bill_l: Rgba = [140, 210, 140, 255];
    let bill_d: Rgba = [40, 100, 50, 255];
    let text: Rgba = [230, 240, 210, 255];

    let w = 22;
    let h = 16;
    let mut c = Canvas::new(w, h);
    c.fill_rect(0, 1, w, h - 2, outline);
    c.fill_rect(1, 2, w - 2, h - 4, bill);
    c.fill_rect(1, 2, w - 2, 1, bill_l);
    c.fill_rect(1, h - 3, w - 2, 1, bill_d);
    // $ symbol area (3 dollar strokes)
    for &x in &[5_i32, 10, 15] {
        c.fill_rect(x, 4, 1, h - 8, text);
        c.put(x - 1, 5, text);
        c.put(x + 1, 5, text);
        c.put(x - 1, h - 6, text);
        c.put(x + 1, h - 6, text);
    }
    // "2X" or "3X" label in top-left corner
    let digit = if factor >= 3 { '3' } else { '2' };
    draw_money_char(&mut c, 1, 1, digit, text);
    draw_money_char(&mut c, 5, 1, 'X', text);
    c.into_image()
}

fn draw_money_char(c: &mut Canvas, x: i32, y: i32, ch: char, col: Rgba) {
    let bits: [u8; 5] = match ch {
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        _ => [0; 5],
    };
    for (row, &b) in bits.iter().enumerate() {
        for cx in 0..3i32 {
            if b & (1 << (2 - cx)) != 0 {
                c.put(x + cx, y + row as i32, col);
            }
        }
    }
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

fn build_assault_rifle_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let body: Rgba = [38, 40, 46, 255];
    let body_light: Rgba = [78, 82, 90, 255];
    let body_dark: Rgba = [18, 20, 24, 255];
    let barrel: Rgba = [22, 24, 28, 255];
    let mag: Rgba = [54, 36, 16, 255];
    let mag_light: Rgba = [88, 60, 26, 255];
    let stock: Rgba = [42, 24, 14, 255];
    let grip: Rgba = [34, 22, 14, 255];
    let muzzle: Rgba = [10, 10, 12, 255];
    let mut c = Canvas::new(28, 11);
    // Barrel
    c.fill_rect(15, 4, 12, 2, outline);
    c.fill_rect(16, 4, 10, 1, barrel);
    c.fill_rect(16, 5, 10, 1, body_dark);
    c.put(26, 4, muzzle);
    // Front sight
    c.fill_rect(20, 3, 1, 1, outline);
    // Receiver / body (boxy)
    c.fill_rect(7, 3, 9, 5, outline);
    c.fill_rect(8, 4, 7, 3, body);
    c.fill_rect(8, 4, 7, 1, body_light);
    // Picatinny rail dots
    c.put(10, 4, body_dark);
    c.put(12, 4, body_dark);
    // Curved magazine
    c.fill_rect(9, 7, 5, 4, outline);
    c.fill_rect(10, 8, 3, 2, mag);
    c.put(10, 8, mag_light);
    // Pistol grip
    c.fill_rect(13, 7, 3, 4, outline);
    c.fill_rect(14, 8, 1, 3, grip);
    // Stock
    c.fill_rect(1, 3, 7, 4, outline);
    c.fill_rect(2, 4, 5, 2, stock);
    c.fill_rect(2, 4, 5, 1, mag_light);
    c.into_image()
}

fn build_grenade_launcher_image() -> Image {
    let outline: Rgba = [6, 8, 6, 255];
    let body: Rgba = [60, 70, 42, 255];
    let body_light: Rgba = [102, 116, 64, 255];
    let body_dark: Rgba = [30, 36, 18, 255];
    let barrel: Rgba = [40, 46, 28, 255];
    let muzzle: Rgba = [12, 14, 8, 255];
    let stock: Rgba = [42, 28, 14, 255];
    let stock_light: Rgba = [78, 52, 22, 255];
    let grip: Rgba = [28, 18, 10, 255];
    let warhead: Rgba = [220, 170, 30, 255];
    let mut c = Canvas::new(30, 12);
    // Wide barrel — grenade launchers have a chunky muzzle
    c.fill_rect(13, 3, 16, 6, outline);
    c.fill_rect(14, 4, 14, 4, body);
    c.fill_rect(14, 4, 14, 1, body_light);
    c.fill_rect(14, 7, 14, 1, body_dark);
    // Rifling rings
    for x in (16..28).step_by(3) {
        c.fill_rect(x, 4, 1, 4, body_dark);
    }
    // Wide muzzle ring
    c.fill_rect(28, 3, 1, 6, outline);
    c.put(28, 5, warhead);
    c.put(28, 6, warhead);
    c.fill_rect(27, 4, 1, 4, muzzle);
    // Trigger / receiver
    c.fill_rect(8, 5, 6, 4, outline);
    c.fill_rect(9, 6, 4, 2, barrel);
    // Tactical grip
    c.fill_rect(10, 8, 3, 4, outline);
    c.fill_rect(11, 9, 1, 3, grip);
    // Pump-action handle on top
    c.fill_rect(15, 1, 6, 2, outline);
    c.fill_rect(16, 2, 4, 1, body_dark);
    // Folding stock
    c.fill_rect(1, 4, 7, 4, outline);
    c.fill_rect(2, 5, 5, 2, stock);
    c.fill_rect(2, 5, 5, 1, stock_light);
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

// ── New weapon sprites (AP pistol, sawed-off, revolver, auto GL, homing RPG) ──

fn build_anti_tank_pistol_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let frame: Rgba = [40, 42, 50, 255];
    let frame_light: Rgba = [78, 82, 92, 255];
    let frame_dark: Rgba = [22, 24, 28, 255];
    let slide: Rgba = [56, 58, 66, 255];
    let slide_light: Rgba = [98, 102, 112, 255];
    let grip: Rgba = [40, 28, 18, 255];
    let grip_light: Rgba = [78, 58, 32, 255];
    let mag_yellow: Rgba = [180, 140, 30, 255];
    let muzzle: Rgba = [12, 14, 16, 255];

    let mut c = Canvas::new(26, 13);
    // Long slide
    c.fill_rect(3, 2, 19, 5, outline);
    c.fill_rect(4, 3, 17, 3, slide);
    c.fill_rect(4, 3, 17, 1, slide_light);
    // Slide serrations
    c.put(8, 4, frame_dark);
    c.put(10, 4, frame_dark);
    c.put(12, 4, frame_dark);
    // Compensator / muzzle brake
    c.fill_rect(21, 3, 4, 4, outline);
    c.put(22, 4, muzzle);
    c.put(23, 4, muzzle);
    c.put(24, 4, muzzle);
    c.put(22, 5, muzzle);
    // Frame
    c.fill_rect(4, 6, 13, 2, outline);
    c.fill_rect(5, 6, 11, 1, frame);
    c.put(6, 6, frame_light);
    // Trigger guard
    c.fill_rect(9, 7, 4, 3, outline);
    c.put(10, 8, frame);
    // Extended mag (stretching below grip)
    c.fill_rect(5, 8, 5, 5, outline);
    c.fill_rect(6, 9, 3, 3, grip);
    c.put(6, 9, grip_light);
    c.put(7, 11, mag_yellow);
    c.put(8, 11, mag_yellow);
    // Optic / red dot mount
    c.fill_rect(9, 1, 4, 2, outline);
    c.put(10, 2, slide_light);
    c.put(12, 2, mag_yellow);

    c.into_image()
}

fn build_sawed_off_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let barrel: Rgba = [42, 44, 50, 255];
    let barrel_light: Rgba = [76, 80, 90, 255];
    let barrel_dark: Rgba = [16, 18, 22, 255];
    let wood: Rgba = [88, 50, 22, 255];
    let wood_light: Rgba = [128, 80, 36, 255];
    let wood_dark: Rgba = [44, 24, 10, 255];
    let muzzle: Rgba = [10, 10, 12, 255];
    let trigger: Rgba = [60, 36, 16, 255];

    let mut c = Canvas::new(20, 12);
    // Twin barrels (top + bottom — double-barrel)
    c.fill_rect(8, 3, 11, 2, outline);
    c.fill_rect(9, 3, 9, 1, barrel_light);
    c.fill_rect(9, 4, 9, 1, barrel);
    c.fill_rect(8, 5, 11, 2, outline);
    c.fill_rect(9, 5, 9, 1, barrel);
    c.fill_rect(9, 6, 9, 1, barrel_dark);
    // Sawed-off muzzles (jagged)
    c.put(18, 3, muzzle);
    c.put(18, 4, muzzle);
    c.put(18, 5, muzzle);
    c.put(18, 6, muzzle);
    c.put(19, 4, muzzle);
    // Receiver block
    c.fill_rect(6, 3, 3, 5, outline);
    c.put(7, 4, barrel);
    c.put(7, 5, barrel);
    c.put(7, 6, barrel_dark);
    // Sawed-off stock — short pistol grip
    c.fill_rect(1, 3, 6, 5, outline);
    c.fill_rect(2, 4, 4, 3, wood);
    c.fill_rect(2, 4, 4, 1, wood_light);
    c.fill_rect(2, 6, 4, 1, wood_dark);
    // Pistol-grip extension
    c.fill_rect(3, 7, 4, 4, outline);
    c.fill_rect(4, 8, 2, 3, wood);
    c.put(4, 8, wood_light);
    // Trigger
    c.fill_rect(7, 8, 2, 2, outline);
    c.put(7, 9, trigger);

    c.into_image()
}

fn build_revolver_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let barrel: Rgba = [50, 52, 60, 255];
    let barrel_light: Rgba = [88, 92, 104, 255];
    let barrel_dark: Rgba = [22, 24, 28, 255];
    let cyl: Rgba = [48, 48, 56, 255];
    let cyl_light: Rgba = [82, 84, 96, 255];
    let cyl_dark: Rgba = [18, 20, 24, 255];
    let wood: Rgba = [74, 44, 18, 255];
    let wood_light: Rgba = [110, 68, 28, 255];
    let muzzle: Rgba = [12, 14, 16, 255];

    let mut c = Canvas::new(22, 13);
    // Barrel
    c.fill_rect(11, 3, 10, 3, outline);
    c.fill_rect(12, 3, 8, 1, barrel_light);
    c.fill_rect(12, 4, 8, 1, barrel);
    c.fill_rect(12, 5, 8, 1, barrel_dark);
    c.put(20, 4, muzzle);
    // Top sight rib
    c.fill_rect(13, 2, 5, 1, barrel_dark);
    // Cylinder (round drum)
    c.fill_circle(9, 6, 4, outline);
    c.fill_circle(9, 6, 3, cyl);
    c.fill_circle(9, 6, 2, cyl_light);
    c.put(9, 6, cyl_dark);
    // Chamber holes
    c.put(7, 5, cyl_dark);
    c.put(11, 5, cyl_dark);
    c.put(8, 8, cyl_dark);
    // Hammer
    c.fill_rect(6, 3, 3, 2, outline);
    c.put(7, 4, barrel_dark);
    // Frame to grip
    c.fill_rect(7, 8, 4, 2, outline);
    // Wooden grip (rounded)
    c.fill_rect(4, 7, 5, 5, outline);
    c.fill_rect(5, 8, 3, 3, wood);
    c.fill_rect(5, 8, 3, 1, wood_light);
    c.put(7, 11, wood_light);
    // Trigger
    c.fill_rect(10, 9, 2, 2, outline);
    c.put(10, 10, barrel_dark);

    c.into_image()
}

fn build_auto_grenade_launcher_image() -> Image {
    let outline: Rgba = [6, 8, 6, 255];
    let body: Rgba = [70, 84, 50, 255];
    let body_light: Rgba = [114, 130, 74, 255];
    let body_dark: Rgba = [36, 44, 22, 255];
    let drum: Rgba = [58, 70, 38, 255];
    let drum_light: Rgba = [96, 112, 60, 255];
    let drum_dark: Rgba = [28, 36, 16, 255];
    let muzzle: Rgba = [12, 14, 8, 255];
    let warhead: Rgba = [220, 170, 30, 255];
    let stock: Rgba = [40, 26, 12, 255];
    let stock_light: Rgba = [78, 50, 22, 255];
    let grip: Rgba = [28, 18, 10, 255];

    let mut c = Canvas::new(34, 16);
    // Wide chunky barrel
    c.fill_rect(15, 4, 16, 6, outline);
    c.fill_rect(16, 5, 14, 4, body);
    c.fill_rect(16, 5, 14, 1, body_light);
    c.fill_rect(16, 8, 14, 1, body_dark);
    // Cooling rib lines
    for x in (18..30).step_by(2) {
        c.put(x, 6, body_dark);
        c.put(x, 7, body_dark);
    }
    // Muzzle ring
    c.fill_rect(30, 4, 2, 6, outline);
    c.put(30, 5, warhead);
    c.put(30, 8, warhead);
    c.put(31, 6, muzzle);
    c.put(31, 7, muzzle);
    // Big drum magazine on top — distinguishes it from single-shot GL
    c.fill_circle(11, 5, 4, outline);
    c.fill_circle(11, 5, 3, drum);
    c.fill_circle(11, 5, 2, drum_light);
    c.put(11, 5, drum_dark);
    c.put(9, 4, drum_dark);
    c.put(13, 6, drum_dark);
    // Receiver
    c.fill_rect(8, 7, 8, 5, outline);
    c.fill_rect(9, 8, 6, 3, body);
    c.put(9, 8, body_light);
    // Pistol grip
    c.fill_rect(11, 11, 3, 4, outline);
    c.fill_rect(12, 12, 1, 3, grip);
    // Trigger guard
    c.fill_rect(13, 11, 3, 2, outline);
    // Stock
    c.fill_rect(1, 7, 7, 5, outline);
    c.fill_rect(2, 8, 5, 3, stock);
    c.fill_rect(2, 8, 5, 1, stock_light);

    c.into_image()
}

fn build_homing_rpg_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let tube: Rgba = [56, 60, 66, 255];
    let tube_light: Rgba = [102, 108, 116, 255];
    let tube_dark: Rgba = [26, 28, 32, 255];
    let stripe: Rgba = [220, 60, 30, 255];
    let radar: Rgba = [40, 220, 240, 255];
    let radar_dark: Rgba = [12, 80, 120, 255];
    let grip: Rgba = [44, 28, 14, 255];
    let grip_light: Rgba = [80, 50, 22, 255];
    let warhead: Rgba = [240, 100, 40, 255];
    let muzzle: Rgba = [12, 12, 14, 255];

    let mut c = Canvas::new(34, 14);
    // Tube
    c.fill_rect(3, 4, 28, 5, outline);
    c.fill_rect(4, 5, 26, 3, tube);
    c.fill_rect(4, 5, 26, 1, tube_light);
    c.fill_rect(4, 7, 26, 1, tube_dark);
    // Twin red stripes (rocket warning)
    c.fill_rect(11, 5, 3, 3, stripe);
    c.fill_rect(20, 5, 3, 3, stripe);
    // Front (warhead exposed)
    c.fill_rect(28, 4, 3, 5, outline);
    c.put(29, 5, warhead);
    c.put(30, 5, warhead);
    c.put(29, 6, muzzle);
    c.put(30, 6, muzzle);
    // Back exhaust
    c.fill_rect(2, 4, 2, 5, outline);
    c.put(1, 6, muzzle);
    // Radar/lock-on dish on top — visual cue for homing
    c.fill_rect(12, 1, 8, 4, outline);
    c.fill_rect(13, 2, 6, 2, radar_dark);
    c.put(14, 2, radar);
    c.put(16, 2, radar);
    c.put(18, 2, radar);
    c.put(15, 3, radar);
    c.put(17, 3, radar);
    // Bracket connection
    c.fill_rect(15, 4, 2, 1, tube_light);
    // Pistol grip
    c.fill_rect(9, 9, 4, 4, outline);
    c.fill_rect(10, 10, 2, 3, grip);
    c.put(10, 10, grip_light);
    // Trigger guard
    c.fill_rect(13, 9, 3, 2, outline);
    c.put(14, 10, tube_dark);

    c.into_image()
}

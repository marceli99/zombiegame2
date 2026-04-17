use bevy::prelude::*;
use rand::Rng;

use crate::bullet::{ExplodeEvent, EXPLODER_EXPLOSION_PLAYER_DAMAGE, EXPLODER_EXPLOSION_RADIUS, EXPLODER_EXPLOSION_ZOMBIE_DAMAGE};
use crate::map::{
    bfs_distance_field, in_bounds, nav_idx, tile_center, world_to_tile, MapObstacles, NavGrid,
    BARRIER_NORTH_Y, BARRIER_SOUTH_Y, BARRIER_UNDERGROUND_Y, MAP_HEIGHT, MAP_WIDTH,
};
use crate::zones::ZoneState;
use crate::net::{is_authoritative, NetContext, NetEntities, NetId};
use crate::pixelart::{Canvas, Rgba};
use crate::player::{Player, PlayerDamagedEvent, PLAYER_RADIUS};
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
}

#[derive(Component)]
pub struct Zombie {
    pub hp: i32,
    pub speed: f32,
    pub kind: ZombieKind,
}

#[derive(Event)]
pub struct ZombieKilledEvent {
    pub kind: ZombieKind,
    pub by_explosion: bool,
}

#[derive(Event, Default)]
pub struct SpawnZombieEvent {
    pub kind: ZombieKind,
}

pub const BURN_DURATION: f32 = 10.0;
pub const BURN_DPS: f32 = 3.5;

#[derive(Component)]
pub struct BurnEffect {
    pub remaining: f32,
    pub accumulated: f32,
}

#[derive(Resource)]
pub struct ZombieAssets {
    pub normal: Handle<Image>,
    pub fast: Handle<Image>,
    pub exploder: Handle<Image>,
    pub burning: Handle<Image>,
    pub giant: Handle<Image>,
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
            .add_systems(Startup, setup_zombie_assets)
            .add_systems(OnExit(GameState::Playing), despawn_all_zombies)
            .add_systems(
                FixedUpdate,
                (
                    spawn_zombie_listener,
                    update_nav_flow,
                    zombie_movement,
                    zombie_attack,
                    burn_tick_system,
                )
                    .chain()
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

fn setup_zombie_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(ZombieAssets {
        normal: images.add(build_normal_zombie_image()),
        fast: images.add(build_fast_zombie_image()),
        exploder: images.add(build_exploder_zombie_image()),
        burning: images.add(build_burning_zombie_image()),
        giant: images.add(build_giant_zombie_image()),
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

pub fn spawn_zombie_entity(
    commands: &mut Commands,
    assets: &ZombieAssets,
    pos: Vec2,
    net_id: u32,
    hp: i32,
    speed: f32,
    kind: ZombieKind,
) -> Entity {
    commands
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
            Zombie { hp, speed, kind },
            NetId(net_id),
        ))
        .id()
}

const MAX_ALIVE_ZOMBIES: usize = 70;

fn spawn_zombie_listener(
    mut commands: Commands,
    mut events: EventReader<SpawnZombieEvent>,
    assets: Res<ZombieAssets>,
    mut ctx: ResMut<NetContext>,
    zone_state: Res<ZoneState>,
    existing: Query<(), With<Zombie>>,
) {
    let alive = existing.iter().count();
    let mut spawned = 0;
    let mut rng = rand::thread_rng();
    for ev in events.read() {
        if alive + spawned >= MAX_ALIVE_ZOMBIES {
            continue;
        }
        spawned += 1;
        let half_w = MAP_WIDTH / 2.0 - 30.0;

        let mut y_min = BARRIER_SOUTH_Y;
        let mut y_max = BARRIER_NORTH_Y;
        if zone_state.unlocked[1] {
            y_max = MAP_HEIGHT / 2.0 - 30.0;
        }
        if zone_state.unlocked[2] {
            y_min = BARRIER_UNDERGROUND_Y;
        }
        if zone_state.unlocked[3] {
            y_min = -(MAP_HEIGHT / 2.0 - 30.0);
        }

        let pos = match rng.gen_range(0..4) {
            0 => Vec2::new(rng.gen_range(-half_w..half_w), y_max),
            1 => Vec2::new(rng.gen_range(-half_w..half_w), y_min),
            2 => Vec2::new(-half_w, rng.gen_range(y_min..y_max)),
            _ => Vec2::new(half_w, rng.gen_range(y_min..y_max)),
        };
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
        let walkable = nav.walkable.clone();
        for (id, pos, tile) in rebuilds {
            let field = bfs_distance_field(&walkable, pos);
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
    if my_d == u16::MAX {
        return None;
    }
    if my_d == 0 {
        return Some((player_pos - zombie_pos).normalize_or_zero());
    }
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
        if dc != 0 && dr != 0 {
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

fn steer_around_obstacles(
    pos: Vec2,
    desired: Vec2,
    obstacles: &MapObstacles,
    radius: f32,
) -> Vec2 {
    if desired == Vec2::ZERO {
        return desired;
    }
    let near = radius + 6.0;
    let far = radius + 22.0;
    if !obstacles.hits(pos + desired * near, radius)
        && !obstacles.hits(pos + desired * far, radius * 0.85)
    {
        return desired;
    }
    let angle_steps: [f32; 5] = [
        std::f32::consts::FRAC_PI_8,
        std::f32::consts::FRAC_PI_4,
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4,
        std::f32::consts::PI * 0.85,
    ];
    let mut best: Option<(f32, Vec2)> = None;
    for &mag in &angle_steps {
        for sign in [1.0_f32, -1.0] {
            let ang = sign * mag;
            let alt = rotate_vec(desired, ang);
            if obstacles.hits(pos + alt * near, radius) {
                continue;
            }
            if obstacles.hits(pos + alt * far, radius * 0.85) {
                continue;
            }
            let score = mag;
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, alt));
            }
        }
        if best.is_some() {
            break;
        }
    }
    if let Some((_, v)) = best {
        return v;
    }
    for sign in [1.0_f32, -1.0] {
        let alt = rotate_vec(desired, sign * std::f32::consts::FRAC_PI_2);
        if !obstacles.hits(pos + alt * near, radius) {
            return alt;
        }
    }
    desired
}

fn zombie_movement(
    time: Res<Time>,
    obstacles: Res<MapObstacles>,
    nav: Res<NavGrid>,
    mut zombies: Query<(&mut Transform, &Zombie), Without<Player>>,
    players: Query<(&Transform, &Player)>,
) {
    let dt = time.delta_seconds();
    for (mut transform, zombie) in &mut zombies {
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
        let dir = steer_around_obstacles(pos, flow, &obstacles, radius);

        if dir != Vec2::ZERO {
            transform.rotation = Quat::from_rotation_z(dir.y.atan2(dir.x));
        }
        transform.translation += (dir * zombie.speed * dt).extend(0.0);

        let mut new_pos = transform.translation.truncate();
        obstacles.resolve(&mut new_pos, radius);
        transform.translation.x = new_pos.x;
        transform.translation.y = new_pos.y;
    }
}

fn zombie_attack(
    mut commands: Commands,
    mut zombies: Query<(Entity, &Transform, &mut Zombie), Without<Player>>,
    players: Query<(Entity, &Transform, &Player)>,
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
        for (p_ent, pt, player) in &players {
            if player.hp <= 0 {
                continue;
            }
            let p = pt.translation.truncate();
            if p.distance(zp) < PLAYER_RADIUS + zr {
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
                        commands.entity(p_ent).insert(BurnEffect {
                            remaining: BURN_DURATION,
                            accumulated: 0.0,
                        });
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
            killed.send(ZombieKilledEvent { kind: zombie.kind, by_explosion: false });
            commands.entity(z_ent).despawn_recursive();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn burn_tick_system(
    mut commands: Commands,
    time: Res<Time>,
    mut burn_query: Query<(Entity, &mut Player, &mut BurnEffect)>,
    other_players: Query<&Player, Without<BurnEffect>>,
    mut net_entities: ResMut<NetEntities>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let dt = time.delta_seconds();
    let mut dead_ids: Vec<u8> = Vec::new();
    for (entity, mut player, mut burn) in &mut burn_query {
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
            dead_ids.push(player.id);
            commands.entity(entity).despawn_recursive();
            net_entities.players.remove(&player.id);
        }
    }
    if !dead_ids.is_empty() {
        let alive = burn_query
            .iter()
            .filter(|(_, p, _)| p.hp > 0 && !dead_ids.contains(&p.id))
            .count()
            + other_players.iter().filter(|p| p.hp > 0).count();
        if alive == 0 {
            next_state.set(GameState::GameOver);
        }
    }
}

fn despawn_all_zombies(mut commands: Commands, q: Query<Entity, With<Zombie>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

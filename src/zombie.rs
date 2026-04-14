use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};
use rand::Rng;

use crate::net::{is_authoritative, NetContext, NetId};
use crate::player::{Player, PlayerDamagedEvent, PLAYER_RADIUS};
use crate::{gameplay_active, GameState, WINDOW_HEIGHT, WINDOW_WIDTH};

pub const ZOMBIE_RADIUS: f32 = 14.0;
pub const ZOMBIE_BASE_SPEED: f32 = 70.0;
pub const ZOMBIE_HP: i32 = 3;
pub const ZOMBIE_DAMAGE: i32 = 15;

#[derive(Component)]
pub struct Zombie {
    pub hp: i32,
    pub speed: f32,
}

#[derive(Event)]
pub struct ZombieKilledEvent {
    pub position: Vec2,
}

#[derive(Event)]
pub struct SpawnZombieEvent;

#[derive(Resource)]
pub struct ZombieAssets {
    pub body_mesh: Handle<Mesh>,
    pub head_mesh: Handle<Mesh>,
    pub arm_mesh: Handle<Mesh>,
    pub shirt_mesh: Handle<Mesh>,
    pub body_mat: Handle<ColorMaterial>,
    pub head_mat: Handle<ColorMaterial>,
    pub arm_mat: Handle<ColorMaterial>,
    pub shirt_mat: Handle<ColorMaterial>,
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
                (spawn_zombie_listener, zombie_movement, zombie_attack)
                    .chain()
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

fn setup_zombie_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.insert_resource(ZombieAssets {
        body_mesh: meshes.add(Rectangle::new(16.0, 16.0)),
        head_mesh: meshes.add(Rectangle::new(9.0, 9.0)),
        arm_mesh: meshes.add(Rectangle::new(7.0, 3.0)),
        shirt_mesh: meshes.add(Rectangle::new(12.0, 5.0)),
        body_mat: materials.add(Color::srgb(0.24, 0.38, 0.14)),
        head_mat: materials.add(Color::srgb(0.36, 0.5, 0.18)),
        arm_mat: materials.add(Color::srgb(0.20, 0.32, 0.10)),
        shirt_mat: materials.add(Color::srgb(0.30, 0.18, 0.08)),
    });
}

pub fn spawn_zombie_entity(
    commands: &mut Commands,
    assets: &ZombieAssets,
    pos: Vec2,
    net_id: u32,
    hp: i32,
    speed: f32,
) -> Entity {
    commands
        .spawn((
            SpatialBundle {
                transform: Transform::from_xyz(pos.x, pos.y, 5.0),
                ..default()
            },
            Zombie { hp, speed },
            NetId(net_id),
        ))
        .with_children(|parent| {
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.body_mesh.clone()),
                material: assets.body_mat.clone(),
                transform: Transform::from_xyz(0.0, 0.0, 0.0),
                ..default()
            });
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.shirt_mesh.clone()),
                material: assets.shirt_mat.clone(),
                transform: Transform::from_xyz(-1.0, -3.0, 0.1),
                ..default()
            });
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.head_mesh.clone()),
                material: assets.head_mat.clone(),
                transform: Transform::from_xyz(3.0, 0.0, 0.2),
                ..default()
            });
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.arm_mesh.clone()),
                material: assets.arm_mat.clone(),
                transform: Transform::from_xyz(10.0, 4.0, 0.15),
                ..default()
            });
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.arm_mesh.clone()),
                material: assets.arm_mat.clone(),
                transform: Transform::from_xyz(10.0, -4.0, 0.15),
                ..default()
            });
        })
        .id()
}

fn spawn_zombie_listener(
    mut commands: Commands,
    mut events: EventReader<SpawnZombieEvent>,
    assets: Res<ZombieAssets>,
    mut ctx: ResMut<NetContext>,
) {
    let mut rng = rand::thread_rng();
    for _ in events.read() {
        let half_w = WINDOW_WIDTH / 2.0 + 40.0;
        let half_h = WINDOW_HEIGHT / 2.0 + 40.0;
        let pos = match rng.gen_range(0..4) {
            0 => Vec2::new(rng.gen_range(-half_w..half_w), half_h),
            1 => Vec2::new(rng.gen_range(-half_w..half_w), -half_h),
            2 => Vec2::new(-half_w, rng.gen_range(-half_h..half_h)),
            _ => Vec2::new(half_w, rng.gen_range(-half_h..half_h)),
        };
        let speed = ZOMBIE_BASE_SPEED + rng.gen_range(-10.0..25.0);
        let net_id = ctx.alloc_zombie_id();
        spawn_zombie_entity(&mut commands, &assets, pos, net_id, ZOMBIE_HP, speed);
    }
}

fn zombie_movement(
    time: Res<Time>,
    mut zombies: Query<(&mut Transform, &Zombie), Without<Player>>,
    players: Query<&Transform, With<Player>>,
) {
    let dt = time.delta_seconds();
    for (mut transform, zombie) in &mut zombies {
        let pos = transform.translation.truncate();
        let mut nearest: Option<Vec2> = None;
        let mut best_d2 = f32::INFINITY;
        for p in &players {
            let pp = p.translation.truncate();
            let d2 = pp.distance_squared(pos);
            if d2 < best_d2 {
                best_d2 = d2;
                nearest = Some(pp);
            }
        }
        let Some(target) = nearest else {
            continue;
        };
        let dir = (target - pos).normalize_or_zero();
        if dir != Vec2::ZERO {
            transform.rotation = Quat::from_rotation_z(dir.y.atan2(dir.x));
        }
        transform.translation += (dir * zombie.speed * dt).extend(0.0);
    }
}

fn zombie_attack(
    zombies: Query<&Transform, (With<Zombie>, Without<Player>)>,
    players: Query<(&Transform, &Player)>,
    mut dmg: EventWriter<PlayerDamagedEvent>,
) {
    for z in &zombies {
        let zp = z.translation.truncate();
        for (pt, player) in &players {
            if player.hp <= 0 {
                continue;
            }
            let p = pt.translation.truncate();
            if p.distance(zp) < PLAYER_RADIUS + ZOMBIE_RADIUS {
                dmg.send(PlayerDamagedEvent {
                    target_id: player.id,
                    amount: ZOMBIE_DAMAGE,
                });
            }
        }
    }
}

fn despawn_all_zombies(mut commands: Commands, q: Query<Entity, With<Zombie>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

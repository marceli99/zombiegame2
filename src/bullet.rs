use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};

use crate::audio::SfxEvent;
use crate::net::{is_authoritative, NetContext, NetId};
use crate::zombie::{Zombie, ZombieKilledEvent, ZOMBIE_RADIUS};
use crate::{gameplay_active, GameState, Score};

pub const BULLET_SPEED: f32 = 720.0;
pub const BULLET_RADIUS: f32 = 4.0;
pub const BULLET_DAMAGE: i32 = 1;
pub const BULLET_LIFETIME: f32 = 1.4;

#[derive(Component)]
pub struct Bullet {
    pub velocity: Vec2,
    pub lifetime: f32,
    pub damage: i32,
}

#[derive(Event)]
pub struct ShootEvent {
    pub origin: Vec2,
    pub direction: Vec2,
}

#[derive(Resource)]
pub struct BulletAssets {
    pub mesh: Handle<Mesh>,
    pub material: Handle<ColorMaterial>,
}

pub struct BulletPlugin;

impl Plugin for BulletPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<ShootEvent>()
            .add_systems(Startup, setup_bullet_assets)
            .add_systems(OnExit(GameState::Playing), despawn_all_bullets)
            .add_systems(
                FixedUpdate,
                (shoot_listener, bullet_movement, bullet_collision)
                    .chain()
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

fn setup_bullet_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.insert_resource(BulletAssets {
        mesh: meshes.add(Rectangle::new(7.0, 3.0)),
        material: materials.add(Color::srgb(1.0, 0.92, 0.35)),
    });
}

pub fn spawn_bullet_entity(
    commands: &mut Commands,
    assets: &BulletAssets,
    origin: Vec2,
    direction: Vec2,
    net_id: u32,
) -> Entity {
    let angle = direction.y.atan2(direction.x);
    commands
        .spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.mesh.clone()),
                material: assets.material.clone(),
                transform: Transform::from_xyz(origin.x, origin.y, 8.0)
                    .with_rotation(Quat::from_rotation_z(angle)),
                ..default()
            },
            Bullet {
                velocity: direction * BULLET_SPEED,
                lifetime: BULLET_LIFETIME,
                damage: BULLET_DAMAGE,
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
) {
    for ev in events.read() {
        let net_id = ctx.alloc_bullet_id();
        spawn_bullet_entity(&mut commands, &assets, ev.origin, ev.direction, net_id);
    }
}

fn bullet_movement(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Transform, &mut Bullet)>,
) {
    let dt = time.delta_seconds();
    for (entity, mut transform, mut bullet) in &mut q {
        transform.translation += (bullet.velocity * dt).extend(0.0);
        bullet.lifetime -= dt;
        if bullet.lifetime <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

fn bullet_collision(
    mut commands: Commands,
    bullets: Query<(Entity, &Transform, &Bullet)>,
    mut zombies: Query<(Entity, &Transform, &mut Zombie)>,
    mut killed: EventWriter<ZombieKilledEvent>,
    mut sfx: EventWriter<SfxEvent>,
    mut score: ResMut<Score>,
) {
    for (b_entity, b_transform, bullet) in &bullets {
        let bp = b_transform.translation.truncate();
        for (z_entity, z_transform, mut zombie) in &mut zombies {
            if zombie.hp <= 0 {
                continue;
            }
            let zp = z_transform.translation.truncate();
            if bp.distance(zp) < BULLET_RADIUS + ZOMBIE_RADIUS {
                zombie.hp -= bullet.damage;
                commands.entity(b_entity).despawn();
                sfx.send(SfxEvent::Hit);
                if zombie.hp <= 0 {
                    commands.entity(z_entity).despawn_recursive();
                    killed.send(ZombieKilledEvent { position: zp });
                    score.0 += 10;
                    sfx.send(SfxEvent::ZombieDeath);
                }
                break;
            }
        }
    }
}

fn despawn_all_bullets(mut commands: Commands, q: Query<Entity, With<Bullet>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

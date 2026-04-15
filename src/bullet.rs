use bevy::prelude::*;

use crate::audio::SfxEvent;
use crate::map::MapObstacles;
use crate::net::{is_authoritative, NetContext, NetId};
use crate::pixelart::{Canvas, Rgba};
use crate::zombie::{Zombie, ZombieKilledEvent, ZOMBIE_RADIUS};
use crate::{gameplay_active, GameState, Score};

const BULLET_SPRITE_SIZE: Vec2 = Vec2::new(14.0, 6.0);

pub const BULLET_RADIUS: f32 = 3.0;
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
    pub damage: i32,
    pub speed: f32,
}

#[derive(Resource)]
pub struct BulletAssets {
    pub image: Handle<Image>,
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

fn setup_bullet_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(BulletAssets {
        image: images.add(build_bullet_image()),
    });
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

pub fn spawn_bullet_entity(
    commands: &mut Commands,
    assets: &BulletAssets,
    origin: Vec2,
    direction: Vec2,
    speed: f32,
    damage: i32,
    net_id: u32,
) -> Entity {
    let angle = direction.y.atan2(direction.x);
    commands
        .spawn((
            SpriteBundle {
                texture: assets.image.clone(),
                sprite: Sprite {
                    custom_size: Some(BULLET_SPRITE_SIZE),
                    ..default()
                },
                transform: Transform::from_xyz(origin.x, origin.y, 8.0)
                    .with_rotation(Quat::from_rotation_z(angle)),
                ..default()
            },
            Bullet {
                velocity: direction * speed,
                lifetime: BULLET_LIFETIME,
                damage,
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
        spawn_bullet_entity(
            &mut commands,
            &assets,
            ev.origin,
            ev.direction,
            ev.speed,
            ev.damage,
            net_id,
        );
    }
}

fn bullet_movement(
    mut commands: Commands,
    time: Res<Time>,
    obstacles: Res<MapObstacles>,
    mut q: Query<(Entity, &mut Transform, &mut Bullet)>,
) {
    let dt = time.delta_seconds();
    for (entity, mut transform, mut bullet) in &mut q {
        transform.translation += (bullet.velocity * dt).extend(0.0);
        bullet.lifetime -= dt;
        if bullet.lifetime <= 0.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        if obstacles.hits(transform.translation.truncate(), BULLET_RADIUS) {
            commands.entity(entity).despawn_recursive();
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
                commands.entity(b_entity).despawn_recursive();
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
        commands.entity(e).despawn_recursive();
    }
}

use bevy::prelude::*;

use crate::audio::SfxEvent;
use crate::map::MapObstacles;
use crate::net::{is_authoritative, NetContext, NetEntities, NetId};
use crate::pixelart::{Canvas, Rgba};
use crate::player::{Player, PlayerDamagedEvent, PLAYER_RADIUS};
use crate::weapon::ThrowableKind;
use crate::zombie::{Zombie, ZombieKilledEvent, ZombieKind};
use crate::{gameplay_active, GameState, Score};

const BULLET_SPRITE_SIZE: Vec2 = Vec2::new(14.0, 6.0);
const ROCKET_SPRITE_SIZE: Vec2 = Vec2::new(26.0, 12.0);

pub const BULLET_RADIUS: f32 = 3.0;
pub const BULLET_LIFETIME: f32 = 1.4;
pub const ROCKET_LIFETIME: f32 = 2.2;
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
pub const FIRE_RADIUS: f32 = 65.0;
pub const FIRE_DURATION: f32 = 4.0;
pub const FIRE_DAMAGE: i32 = 3;
pub const FIRE_TICK: f32 = 0.4;
pub const THROW_RANGE: f32 = 300.0;

#[derive(Component)]
pub struct ThrowIndicator;

#[derive(Resource)]
pub struct BulletAssets {
    pub bullet: Handle<Image>,
    pub rocket: Handle<Image>,
    pub explosion: Handle<Image>,
    pub thrown: Handle<Image>,
}

pub struct BulletPlugin;

impl Plugin for BulletPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<ShootEvent>()
            .add_event::<ExplodeEvent>()
            .add_event::<ThrowEvent>()
            .add_systems(Startup, setup_bullet_assets)
            .add_systems(OnExit(GameState::Playing), despawn_all_bullets)
            .add_systems(
                Update,
                update_throw_indicator.run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                (
                    shoot_listener,
                    throw_listener,
                    bullet_movement,
                    thrown_projectile_update,
                    bullet_collision,
                    explode_listener,
                    explosion_lifetime,
                    smoke_cloud_update,
                    fire_pool_update,
                )
                    .chain()
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

fn setup_bullet_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let indicator_img = images.add(build_indicator_image());
    commands.insert_resource(BulletAssets {
        bullet: images.add(build_bullet_image()),
        rocket: images.add(build_rocket_image()),
        explosion: images.add(build_explosion_image()),
        thrown: images.add(build_thrown_image()),
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
) -> Entity {
    let angle = direction.y.atan2(direction.x);
    let (texture, size, lifetime) = if is_rocket {
        (assets.rocket.clone(), ROCKET_SPRITE_SIZE, ROCKET_LIFETIME)
    } else {
        (assets.bullet.clone(), BULLET_SPRITE_SIZE, BULLET_LIFETIME)
    };
    commands
        .spawn((
            SpriteBundle {
                texture,
                sprite: Sprite {
                    custom_size: Some(size),
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
            ev.is_rocket,
        );
    }
}

fn bullet_movement(
    mut commands: Commands,
    time: Res<Time>,
    obstacles: Res<MapObstacles>,
    mut q: Query<(Entity, &mut Transform, &mut Bullet)>,
    mut explode: EventWriter<ExplodeEvent>,
) {
    let dt = time.delta_seconds();
    for (entity, mut transform, mut bullet) in &mut q {
        transform.translation += (bullet.velocity * dt).extend(0.0);
        bullet.lifetime -= dt;
        let pos = transform.translation.truncate();
        let hit_obstacle = obstacles.hits(pos, BULLET_RADIUS);
        if bullet.lifetime <= 0.0 || hit_obstacle {
            if bullet.is_rocket {
                explode.send(ExplodeEvent {
                    pos,
                    radius: ROCKET_EXPLOSION_RADIUS,
                    zombie_damage: ROCKET_EXPLOSION_ZOMBIE_DAMAGE,
                    player_damage: ROCKET_EXPLOSION_PLAYER_DAMAGE,
                });
            }
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn bullet_collision(
    mut commands: Commands,
    bullets: Query<(Entity, &Transform, &Bullet)>,
    mut zombies: Query<(Entity, &Transform, &mut Zombie)>,
    mut killed: EventWriter<ZombieKilledEvent>,
    mut explode: EventWriter<ExplodeEvent>,
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
            if bp.distance(zp) < BULLET_RADIUS + zombie.kind.radius() {
                if bullet.is_rocket {
                    explode.send(ExplodeEvent {
                        pos: bp,
                        radius: ROCKET_EXPLOSION_RADIUS,
                        zombie_damage: ROCKET_EXPLOSION_ZOMBIE_DAMAGE,
                        player_damage: ROCKET_EXPLOSION_PLAYER_DAMAGE,
                    });
                    commands.entity(b_entity).despawn_recursive();
                    break;
                }
                zombie.hp -= bullet.damage;
                commands.entity(b_entity).despawn_recursive();
                sfx.send(SfxEvent::Hit);
                if zombie.hp <= 0 {
                    let z_kind = zombie.kind;
                    let was_exploder = matches!(z_kind, ZombieKind::Exploder);
                    commands.entity(z_entity).despawn_recursive();
                    killed.send(ZombieKilledEvent { kind: z_kind, by_explosion: false });
                    score.0 += 20;
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
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn explode_listener(
    mut commands: Commands,
    assets: Res<BulletAssets>,
    mut events: EventReader<ExplodeEvent>,
    mut zombies: Query<(Entity, &Transform, &mut Zombie)>,
    players: Query<(&Transform, &Player)>,
    mut damage_evw: EventWriter<PlayerDamagedEvent>,
    mut killed_evw: EventWriter<ZombieKilledEvent>,
    mut sfx: EventWriter<SfxEvent>,
    mut score: ResMut<Score>,
    mut ctx: ResMut<NetContext>,
    mut net_entities: ResMut<NetEntities>,
) {
    let mut queue: Vec<ExplodeEvent> = events.read().copied().collect();
    while let Some(ev) = queue.pop() {
        for (z_ent, z_t, mut zombie) in &mut zombies {
            if zombie.hp <= 0 {
                continue;
            }
            let zp = z_t.translation.truncate();
            if zp.distance(ev.pos) < ev.radius + zombie.kind.radius() {
                zombie.hp -= ev.zombie_damage;
                if zombie.hp <= 0 {
                    let z_kind = zombie.kind;
                    let was_exploder = matches!(z_kind, ZombieKind::Exploder);
                    commands.entity(z_ent).despawn_recursive();
                    killed_evw.send(ZombieKilledEvent { kind: z_kind, by_explosion: true });
                    score.0 += 20;
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
        for (p_t, player) in &players {
            if player.hp <= 0 {
                continue;
            }
            let pp = p_t.translation.truncate();
            if pp.distance(ev.pos) < ev.radius + PLAYER_RADIUS {
                damage_evw.send(PlayerDamagedEvent {
                    target_id: player.id,
                    amount: ev.player_damage,
                });
            }
        }
        let net_id = ctx.alloc_explosion_id();
        let ent = spawn_explosion_entity(&mut commands, &assets, ev.pos, ev.radius, net_id);
        net_entities.explosions.insert(net_id, ent);
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
    mut q: Query<(Entity, &mut Transform, &mut ThrownProjectile)>,
    mut explode: EventWriter<ExplodeEvent>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
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
                spawn_smoke_cloud(&mut commands, &mut meshes, &mut materials, pos);
            }
            ThrowableKind::Molotov => {
                spawn_fire_pool(&mut commands, &mut meshes, &mut materials, pos);
            }
        }
    }
}

fn spawn_smoke_cloud(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    pos: Vec2,
) {
    use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};
    let mesh = meshes.add(Circle::new(SMOKE_RADIUS));
    let mat = materials.add(Color::srgba(0.7, 0.72, 0.75, 0.35));
    commands.spawn((
        MaterialMesh2dBundle {
            mesh: Mesh2dHandle(mesh),
            material: mat,
            transform: Transform::from_xyz(pos.x, pos.y, 9.0),
            ..default()
        },
        SmokeCloud {
            lifetime: SMOKE_DURATION,
            radius: SMOKE_RADIUS,
        },
    ));
}

fn spawn_fire_pool(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    pos: Vec2,
) {
    use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};
    let mesh = meshes.add(Circle::new(FIRE_RADIUS));
    let mat = materials.add(Color::srgba(0.9, 0.35, 0.05, 0.4));
    commands.spawn((
        MaterialMesh2dBundle {
            mesh: Mesh2dHandle(mesh),
            material: mat,
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
    mut zombies: Query<(&Transform, &mut Zombie), Without<SmokeCloud>>,
) {
    let dt = time.delta_seconds();
    for (entity, mut cloud, cloud_t) in &mut clouds {
        cloud.lifetime -= dt;
        if cloud.lifetime <= 0.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        // Slow zombies inside the cloud
        let cp = cloud_t.translation.truncate();
        for (zt, mut zombie) in &mut zombies {
            let zp = zt.translation.truncate();
            if zp.distance(cp) < cloud.radius + zombie.kind.radius() {
                zombie.speed = zombie.kind.base_speed() * 0.3;
            }
        }
    }
}

fn fire_pool_update(
    mut commands: Commands,
    time: Res<Time>,
    mut fires: Query<(Entity, &mut FirePool, &Transform)>,
    mut zombies: Query<(Entity, &Transform, &mut Zombie), Without<FirePool>>,
    mut killed: EventWriter<ZombieKilledEvent>,
    mut score: ResMut<Score>,
) {
    let dt = time.delta_seconds();
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
            if zp.distance(fp) < fire.radius + zombie.kind.radius() {
                zombie.hp -= FIRE_DAMAGE;
                if zombie.hp <= 0 {
                    let z_kind = zombie.kind;
                    commands.entity(z_ent).despawn_recursive();
                    killed.send(ZombieKilledEvent {
                        kind: z_kind,
                        by_explosion: true,
                    });
                    score.0 += 20;
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
        )>,
    >,
    mut net_entities: ResMut<NetEntities>,
) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
    net_entities.explosions.clear();
}

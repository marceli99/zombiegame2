use bevy::prelude::*;
use rand::Rng;

use crate::map::{
    bfs_distance_field, in_bounds, nav_idx, tile_center, world_to_tile, MapObstacles, NavGrid,
    MAP_HEIGHT, MAP_WIDTH,
};
use crate::net::{is_authoritative, NetContext, NetId};
use crate::pixelart::{Canvas, Rgba};
use crate::player::{Player, PlayerDamagedEvent, PLAYER_RADIUS};
use crate::{gameplay_active, GameState};

const ZOMBIE_SPRITE_SIZE: Vec2 = Vec2::new(32.0, 32.0);

pub const ZOMBIE_RADIUS: f32 = 10.0;
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
    pub image: Handle<Image>,
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
                )
                    .chain()
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

fn setup_zombie_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(ZombieAssets {
        image: images.add(build_zombie_image()),
    });
}

fn build_zombie_image() -> Image {
    let outline: Rgba = [10, 18, 6, 255];
    let body_main: Rgba = [100, 150, 55, 255];
    let body_light: Rgba = [140, 185, 80, 255];
    let body_dark: Rgba = [62, 98, 32, 255];
    let head_main: Rgba = [130, 172, 70, 255];
    let head_light: Rgba = [170, 205, 95, 255];
    let shirt: Rgba = [98, 56, 30, 255];
    let shirt_dark: Rgba = [56, 28, 12, 255];
    let eye: Rgba = [245, 50, 35, 255];
    let wound: Rgba = [140, 18, 18, 255];
    let wound_light: Rgba = [200, 30, 30, 255];
    let arm: Rgba = [118, 162, 60, 255];
    let arm_dark: Rgba = [70, 105, 35, 255];

    let mut c = Canvas::new(25, 25);

    c.fill_circle(11, 12, 9, outline);
    c.fill_circle(11, 12, 8, body_dark);
    c.fill_circle(11, 12, 7, body_main);
    c.fill_circle(9, 10, 3, body_light);

    c.fill_rect(9, 9, 4, 8, shirt);
    c.put(9, 10, shirt_dark);
    c.put(12, 12, shirt_dark);
    c.put(10, 15, shirt_dark);
    c.put(11, 16, shirt_dark);

    c.fill_rect(13, 7, 3, 2, wound);
    c.put(13, 7, wound_light);
    c.put(10, 16, wound);

    c.fill_rect(11, 4, 8, 3, outline);
    c.fill_rect(11, 5, 7, 1, arm);
    c.fill_rect(11, 6, 7, 1, arm_dark);
    c.fill_rect(17, 3, 3, 3, outline);
    c.put(18, 4, head_light);

    c.fill_rect(11, 18, 8, 3, outline);
    c.fill_rect(11, 19, 7, 1, arm);
    c.fill_rect(11, 20, 7, 1, arm_dark);
    c.fill_rect(17, 19, 3, 3, outline);
    c.put(18, 20, head_light);

    c.fill_circle(15, 12, 4, outline);
    c.fill_circle(15, 12, 3, head_main);
    c.put(14, 10, head_light);

    c.put(17, 11, eye);
    c.put(17, 13, eye);
    c.put(16, 12, outline);

    c.into_image()
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
            SpriteBundle {
                texture: assets.image.clone(),
                sprite: Sprite {
                    custom_size: Some(ZOMBIE_SPRITE_SIZE),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 5.0),
                ..default()
            },
            Zombie { hp, speed },
            NetId(net_id),
        ))
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
        let half_w = MAP_WIDTH / 2.0 - 30.0;
        let half_h = MAP_HEIGHT / 2.0 - 30.0;
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

fn update_nav_flow(mut nav: ResMut<NavGrid>, players: Query<(&Transform, &Player)>) {
    nav.player_flow.clear();
    let walkable = nav.walkable.clone();
    for (t, p) in &players {
        if p.hp <= 0 {
            continue;
        }
        let field = bfs_distance_field(&walkable, t.translation.truncate());
        nav.player_flow.insert(p.id, field);
    }
}

fn zombie_flow_direction(nav: &NavGrid, zombie_pos: Vec2, player_pos: Vec2) -> Option<Vec2> {
    let flow = nav.player_flow.values().min_by_key(|field| {
        let (c, r) = world_to_tile(zombie_pos);
        if !in_bounds(c, r) {
            return u16::MAX;
        }
        field[nav_idx(c, r)]
    })?;
    let (zc, zr) = world_to_tile(zombie_pos);
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
        if best.map(|(bd, _)| d < bd).unwrap_or(true) {
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
) -> Vec2 {
    if desired == Vec2::ZERO {
        return desired;
    }
    let look = ZOMBIE_RADIUS + 10.0;
    if !obstacles.hits(pos + desired * look, ZOMBIE_RADIUS) {
        return desired;
    }
    const OFFSETS: [f32; 6] = [
        std::f32::consts::FRAC_PI_6,
        -std::f32::consts::FRAC_PI_6,
        std::f32::consts::FRAC_PI_4,
        -std::f32::consts::FRAC_PI_4,
        std::f32::consts::FRAC_PI_2,
        -std::f32::consts::FRAC_PI_2,
    ];
    for &ang in &OFFSETS {
        let alt = rotate_vec(desired, ang);
        if !obstacles.hits(pos + alt * look, ZOMBIE_RADIUS) {
            return alt;
        }
    }
    let back_offsets: [f32; 2] = [
        std::f32::consts::PI * 0.75,
        -std::f32::consts::PI * 0.75,
    ];
    for &ang in &back_offsets {
        let alt = rotate_vec(desired, ang);
        if !obstacles.hits(pos + alt * look, ZOMBIE_RADIUS) {
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
        let dir = steer_around_obstacles(pos, flow, &obstacles);

        if dir != Vec2::ZERO {
            transform.rotation = Quat::from_rotation_z(dir.y.atan2(dir.x));
        }
        transform.translation += (dir * zombie.speed * dt).extend(0.0);

        let mut new_pos = transform.translation.truncate();
        obstacles.resolve(&mut new_pos, ZOMBIE_RADIUS);
        transform.translation.x = new_pos.x;
        transform.translation.y = new_pos.y;
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

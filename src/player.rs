use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rand::Rng;

use std::collections::HashSet;

use crate::audio::SfxEvent;
use crate::bullet::ShootEvent;
use crate::map::{MapObstacles, MAP_HEIGHT, MAP_WIDTH};
use crate::net::{
    is_authoritative, LocalInput, NetContext, NetEntities, NetMode, RemoteInputs,
};
use crate::pixelart::{Canvas, Rgba};
use crate::weapon::Weapon;
use crate::{gameplay_active, GameState, Score};

const PLAYER_SPRITE_SIZE: Vec2 = Vec2::new(30.0, 25.0);

pub const PLAYER_RADIUS: f32 = 10.0;
pub const PLAYER_SPEED: f32 = 260.0;
pub const PLAYER_MAX_HP: i32 = 100;
pub const PLAYER_INVULN: f32 = 0.5;

#[derive(Component)]
pub struct Player {
    pub id: u8,
    pub hp: i32,
    pub fire_cooldown: f32,
    pub invuln_timer: f32,
    pub aim: Vec2,
    pub weapon: Weapon,
}

#[derive(Event)]
pub struct PlayerDamagedEvent {
    pub target_id: u8,
    pub amount: i32,
}

#[derive(Resource)]
pub struct PlayerAssets {
    pub images: [Handle<Image>; 4],
}

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<PlayerDamagedEvent>()
            .add_systems(Startup, setup_player_assets)
            .add_systems(OnEnter(GameState::Playing), spawn_players)
            .add_systems(OnExit(GameState::Playing), despawn_players)
            .add_systems(
                Update,
                gather_local_input.run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                (server_player_tick, player_damage_handler)
                    .chain()
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

fn setup_player_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let imgs: [Handle<Image>; 4] = [
        images.add(build_player_image(0)),
        images.add(build_player_image(1)),
        images.add(build_player_image(2)),
        images.add(build_player_image(3)),
    ];
    commands.insert_resource(PlayerAssets { images: imgs });
}

fn player_palette(id: u8) -> (Rgba, Rgba, Rgba) {
    match id % 4 {
        0 => (
            [100, 145, 230, 255],
            [60, 95, 175, 255],
            [32, 55, 110, 255],
        ),
        1 => (
            [225, 90, 75, 255],
            [170, 55, 50, 255],
            [95, 24, 24, 255],
        ),
        2 => (
            [110, 190, 90, 255],
            [65, 135, 60, 255],
            [35, 80, 32, 255],
        ),
        _ => (
            [235, 195, 60, 255],
            [180, 140, 35, 255],
            [105, 78, 18, 255],
        ),
    }
}

fn build_player_image(id: u8) -> Image {
    let (body_light, body_main, body_dark) = player_palette(id);
    let outline: Rgba = [14, 12, 8, 255];
    let skin: Rgba = [232, 196, 148, 255];
    let skin_light: Rgba = [252, 222, 184, 255];
    let hair: Rgba = [36, 22, 12, 255];
    let gun: Rgba = [46, 46, 56, 255];
    let gun_light: Rgba = [92, 92, 104, 255];
    let stock: Rgba = [118, 72, 30, 255];
    let stock_dark: Rgba = [72, 42, 15, 255];

    let mut c = Canvas::new(25, 21);

    c.fill_circle(12, 10, 8, outline);
    c.fill_circle(12, 10, 7, body_dark);
    c.fill_circle(12, 10, 6, body_main);
    c.fill_circle(10, 8, 3, body_light);

    c.fill_rect(10, 8, 3, 5, body_dark);
    c.put(11, 10, body_main);

    c.fill_rect(17, 9, 4, 3, stock_dark);
    c.fill_rect(17, 9, 4, 1, stock);
    c.put(17, 10, stock);

    c.fill_rect(19, 10, 6, 2, gun);
    c.fill_rect(19, 10, 6, 1, gun_light);
    c.put(24, 10, outline);
    c.put(24, 11, outline);

    c.fill_circle(15, 10, 3, outline);
    c.fill_circle(15, 10, 2, skin);
    c.put(16, 9, skin_light);

    c.fill_rect(12, 8, 2, 5, hair);
    c.put(13, 8, outline);
    c.put(13, 12, outline);

    c.into_image()
}

pub fn spawn_player_entity(
    commands: &mut Commands,
    assets: &PlayerAssets,
    id: u8,
    pos: Vec2,
) -> Entity {
    commands
        .spawn((
            SpriteBundle {
                texture: assets.images[(id as usize) % 4].clone(),
                sprite: Sprite {
                    custom_size: Some(PLAYER_SPRITE_SIZE),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 10.0),
                ..default()
            },
            Player {
                id,
                hp: PLAYER_MAX_HP,
                fire_cooldown: 0.0,
                invuln_timer: 0.0,
                aim: Vec2::X,
                weapon: Weapon::Pistol,
            },
        ))
        .id()
}

fn spawn_players(
    mut commands: Commands,
    assets: Res<PlayerAssets>,
    mut score: ResMut<Score>,
    net: Res<NetMode>,
    ctx: Res<NetContext>,
    mut net_entities: ResMut<NetEntities>,
) {
    score.0 = 0;
    net_entities.clear();

    let ids: Vec<u8> = match *net {
        NetMode::SinglePlayer => vec![0],
        NetMode::Host => {
            if ctx.lobby_players.is_empty() {
                vec![0]
            } else {
                ctx.lobby_players.clone()
            }
        }
        NetMode::Client => return,
    };

    for (idx, id) in ids.iter().enumerate() {
        let pos = Vec2::new(0.0, -100.0 + idx as f32 * 70.0);
        let ent = spawn_player_entity(&mut commands, &assets, *id, pos);
        net_entities.players.insert(*id, ent);
    }
}

fn despawn_players(
    mut commands: Commands,
    q: Query<Entity, With<Player>>,
    mut net_entities: ResMut<NetEntities>,
) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
    net_entities.clear();
}

fn gather_local_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    players: Query<(&Transform, &Player)>,
    ctx: Res<NetContext>,
    mut local: ResMut<LocalInput>,
) {
    let mut mv = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        mv.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        mv.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        mv.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        mv.x += 1.0;
    }
    mv = mv.normalize_or_zero();
    local.0.move_x = mv.x;
    local.0.move_y = mv.y;
    local.0.shoot = mouse.pressed(MouseButton::Left);

    let Ok(window) = windows.get_single() else {
        return;
    };
    let Ok((camera, cam_transform)) = cameras.get_single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Some(world) = camera.viewport_to_world_2d(cam_transform, cursor) else {
        return;
    };

    let my_pos = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .map(|(t, _)| t.translation.truncate());

    if let Some(pos) = my_pos {
        let dir = (world - pos).normalize_or_zero();
        if dir != Vec2::ZERO {
            local.0.aim_x = dir.x;
            local.0.aim_y = dir.y;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn server_player_tick(
    time: Res<Time>,
    local: Res<LocalInput>,
    remote: Res<RemoteInputs>,
    ctx: Res<NetContext>,
    obstacles: Res<MapObstacles>,
    mut players: Query<(&mut Transform, &mut Player)>,
    mut shoot_events: EventWriter<ShootEvent>,
    mut sfx: EventWriter<SfxEvent>,
) {
    let dt = time.delta_seconds();
    for (mut transform, mut player) in &mut players {
        let input = if player.id == ctx.my_id {
            local.0
        } else {
            remote.0.get(&player.id).copied().unwrap_or_default()
        };

        let mv = Vec2::new(input.move_x, input.move_y);
        let mv = if mv.length_squared() > 1.0 {
            mv.normalize()
        } else {
            mv
        };
        if mv != Vec2::ZERO {
            transform.translation += (mv * PLAYER_SPEED * dt).extend(0.0);
        }
        let half_w = MAP_WIDTH / 2.0 - PLAYER_RADIUS;
        let half_h = MAP_HEIGHT / 2.0 - PLAYER_RADIUS;
        transform.translation.x = transform.translation.x.clamp(-half_w, half_w);
        transform.translation.y = transform.translation.y.clamp(-half_h, half_h);

        let mut pos = transform.translation.truncate();
        obstacles.resolve(&mut pos, PLAYER_RADIUS);
        transform.translation.x = pos.x.clamp(-half_w, half_w);
        transform.translation.y = pos.y.clamp(-half_h, half_h);

        let aim = Vec2::new(input.aim_x, input.aim_y);
        let aim = if aim.length_squared() > 0.0001 {
            aim.normalize()
        } else {
            player.aim
        };
        player.aim = aim;
        transform.rotation = Quat::from_rotation_z(aim.y.atan2(aim.x));

        if player.fire_cooldown > 0.0 {
            player.fire_cooldown -= dt;
        }
        if player.invuln_timer > 0.0 {
            player.invuln_timer -= dt;
        }

        if input.shoot && player.fire_cooldown <= 0.0 && player.hp > 0 {
            let weapon = player.weapon;
            player.fire_cooldown = weapon.fire_cooldown();
            let origin = transform.translation.truncate() + player.aim * (PLAYER_RADIUS + 8.0);
            let count = weapon.bullet_count();
            let spread = weapon.spread();
            let damage = weapon.bullet_damage();
            let speed = weapon.bullet_speed();
            let mut rng = rand::thread_rng();
            for _ in 0..count {
                let angle = if spread > 0.0 {
                    rng.gen_range(-spread..spread)
                } else {
                    0.0
                };
                let (sin, cos) = angle.sin_cos();
                let dir = Vec2::new(
                    player.aim.x * cos - player.aim.y * sin,
                    player.aim.x * sin + player.aim.y * cos,
                );
                shoot_events.send(ShootEvent {
                    origin,
                    direction: dir,
                    damage,
                    speed,
                });
            }
            sfx.send(SfxEvent::Shot);
        }
    }
}

fn player_damage_handler(
    mut commands: Commands,
    mut events: EventReader<PlayerDamagedEvent>,
    mut players: Query<(Entity, &mut Player)>,
    mut net_entities: ResMut<NetEntities>,
    mut sfx: EventWriter<SfxEvent>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let mut newly_dead: HashSet<u8> = HashSet::new();
    for ev in events.read() {
        for (_, mut player) in &mut players {
            if player.id != ev.target_id {
                continue;
            }
            if player.invuln_timer > 0.0 || player.hp <= 0 {
                continue;
            }
            player.hp -= ev.amount;
            player.invuln_timer = PLAYER_INVULN;
            sfx.send(SfxEvent::PlayerHit);
            if player.hp <= 0 {
                newly_dead.insert(player.id);
            }
        }
    }
    if newly_dead.is_empty() {
        return;
    }

    for (entity, player) in &players {
        if newly_dead.contains(&player.id) {
            commands.entity(entity).despawn_recursive();
            net_entities.players.remove(&player.id);
        }
    }

    let survivors = players
        .iter()
        .filter(|(_, p)| p.hp > 0 && !newly_dead.contains(&p.id))
        .count();
    if survivors == 0 {
        next_state.set(GameState::GameOver);
    }
}

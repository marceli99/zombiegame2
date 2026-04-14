use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};
use bevy::window::PrimaryWindow;

use crate::audio::SfxEvent;
use crate::bullet::ShootEvent;
use crate::net::{
    is_authoritative, LocalInput, NetContext, NetEntities, NetMode, RemoteInputs,
};
use crate::{gameplay_active, GameState, Score, WINDOW_HEIGHT, WINDOW_WIDTH};

pub const PLAYER_RADIUS: f32 = 16.0;
pub const PLAYER_SPEED: f32 = 280.0;
pub const PLAYER_MAX_HP: i32 = 100;
pub const PLAYER_FIRE_COOLDOWN: f32 = 0.15;
pub const PLAYER_INVULN: f32 = 0.5;

#[derive(Component)]
pub struct Player {
    pub id: u8,
    pub hp: i32,
    pub fire_cooldown: f32,
    pub invuln_timer: f32,
    pub aim: Vec2,
}

#[derive(Event)]
pub struct PlayerDamagedEvent {
    pub target_id: u8,
    pub amount: i32,
}

#[derive(Resource)]
pub struct PlayerAssets {
    pub body_mesh: Handle<Mesh>,
    pub jacket_mesh: Handle<Mesh>,
    pub head_mesh: Handle<Mesh>,
    pub hair_mesh: Handle<Mesh>,
    pub gun_mesh: Handle<Mesh>,
    pub muzzle_mesh: Handle<Mesh>,
    pub head_mat: Handle<ColorMaterial>,
    pub hair_mat: Handle<ColorMaterial>,
    pub gun_mat: Handle<ColorMaterial>,
    pub muzzle_mat: Handle<ColorMaterial>,
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

fn setup_player_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.insert_resource(PlayerAssets {
        body_mesh: meshes.add(Rectangle::new(18.0, 18.0)),
        jacket_mesh: meshes.add(Rectangle::new(14.0, 6.0)),
        head_mesh: meshes.add(Rectangle::new(11.0, 11.0)),
        hair_mesh: meshes.add(Rectangle::new(11.0, 3.0)),
        gun_mesh: meshes.add(Rectangle::new(16.0, 4.0)),
        muzzle_mesh: meshes.add(Rectangle::new(3.0, 3.0)),
        head_mat: materials.add(Color::srgb(0.85, 0.68, 0.52)),
        hair_mat: materials.add(Color::srgb(0.22, 0.12, 0.06)),
        gun_mat: materials.add(Color::srgb(0.18, 0.18, 0.22)),
        muzzle_mat: materials.add(Color::srgb(0.68, 0.68, 0.72)),
    });
}

fn player_body_colors(id: u8) -> (Color, Color) {
    match id % 4 {
        0 => (
            Color::srgb(0.18, 0.26, 0.48),
            Color::srgb(0.10, 0.16, 0.32),
        ),
        1 => (
            Color::srgb(0.52, 0.18, 0.18),
            Color::srgb(0.34, 0.10, 0.10),
        ),
        2 => (
            Color::srgb(0.20, 0.46, 0.22),
            Color::srgb(0.10, 0.30, 0.12),
        ),
        _ => (
            Color::srgb(0.58, 0.52, 0.14),
            Color::srgb(0.38, 0.32, 0.08),
        ),
    }
}

pub fn spawn_player_entity(
    commands: &mut Commands,
    assets: &PlayerAssets,
    materials: &mut Assets<ColorMaterial>,
    id: u8,
    pos: Vec2,
) -> Entity {
    let (body_col, jacket_col) = player_body_colors(id);
    let body_mat = materials.add(body_col);
    let jacket_mat = materials.add(jacket_col);
    commands
        .spawn((
            SpatialBundle {
                transform: Transform::from_xyz(pos.x, pos.y, 10.0),
                ..default()
            },
            Player {
                id,
                hp: PLAYER_MAX_HP,
                fire_cooldown: 0.0,
                invuln_timer: 0.0,
                aim: Vec2::X,
            },
        ))
        .with_children(|parent| {
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.body_mesh.clone()),
                material: body_mat,
                transform: Transform::from_xyz(0.0, 0.0, 0.0),
                ..default()
            });
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.jacket_mesh.clone()),
                material: jacket_mat,
                transform: Transform::from_xyz(-1.0, -5.0, 0.1),
                ..default()
            });
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.head_mesh.clone()),
                material: assets.head_mat.clone(),
                transform: Transform::from_xyz(2.0, 0.0, 1.0),
                ..default()
            });
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.hair_mesh.clone()),
                material: assets.hair_mat.clone(),
                transform: Transform::from_xyz(-1.0, 0.0, 1.1),
                ..default()
            });
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.gun_mesh.clone()),
                material: assets.gun_mat.clone(),
                transform: Transform::from_xyz(14.0, 0.0, 0.5),
                ..default()
            });
            parent.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(assets.muzzle_mesh.clone()),
                material: assets.muzzle_mat.clone(),
                transform: Transform::from_xyz(22.0, 0.0, 0.6),
                ..default()
            });
        })
        .id()
}

fn spawn_players(
    mut commands: Commands,
    assets: Res<PlayerAssets>,
    mut materials: ResMut<Assets<ColorMaterial>>,
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
        let pos = Vec2::new(-220.0 + idx as f32 * 120.0, 0.0);
        let ent = spawn_player_entity(&mut commands, &assets, &mut materials, *id, pos);
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
        let half_w = WINDOW_WIDTH / 2.0 - PLAYER_RADIUS;
        let half_h = WINDOW_HEIGHT / 2.0 - PLAYER_RADIUS;
        transform.translation.x = transform.translation.x.clamp(-half_w, half_w);
        transform.translation.y = transform.translation.y.clamp(-half_h, half_h);

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
            player.fire_cooldown = PLAYER_FIRE_COOLDOWN;
            let origin = transform.translation.truncate() + player.aim * (PLAYER_RADIUS + 8.0);
            shoot_events.send(ShootEvent {
                origin,
                direction: player.aim,
            });
            sfx.send(SfxEvent::Shot);
        }
    }
}

fn player_damage_handler(
    mut events: EventReader<PlayerDamagedEvent>,
    mut players: Query<&mut Player>,
    mut sfx: EventWriter<SfxEvent>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let mut any_dead = false;
    for ev in events.read() {
        for mut player in &mut players {
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
                any_dead = true;
            }
        }
    }
    if any_dead {
        let all_dead = players.iter().all(|p| p.hp <= 0);
        if all_dead {
            next_state.set(GameState::GameOver);
        }
    }
}

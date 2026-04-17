mod achievements;
mod audio;
mod bullet;
mod lobby;
mod map;
mod menu;
mod net;
mod pause;
mod pixelart;
mod player;
mod settings;
mod sync;
mod ui;
mod wave;
mod weapon;
mod zombie;
mod zones;

use bevy::prelude::*;
use bevy::render::camera::ScalingMode;
use bevy::time::Fixed;
use bevy::window::{PrimaryWindow, WindowMode, WindowResizeConstraints};

use crate::map::{MAP_HEIGHT, MAP_WIDTH};
use crate::net::{NetContext, NetMode};
use crate::player::Player;

pub const WINDOW_WIDTH: f32 = 1280.0;
pub const WINDOW_HEIGHT: f32 = 720.0;
pub const FIXED_VIEW_H: f32 = 760.0;
pub const TICK_HZ: f64 = 60.0;

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GameState {
    #[default]
    Menu,
    Settings,
    JoinPrompt,
    Lobby,
    Playing,
    GameOver,
    Achievements,
    Guide,
}

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum PauseState {
    #[default]
    Running,
    Paused,
}

#[derive(Resource)]
pub struct UiAssets {
    pub font: Handle<Font>,
}

#[derive(Resource, Default)]
pub struct Score(pub u32);

pub fn gameplay_active(
    game: Res<State<GameState>>,
    pause: Res<State<PauseState>>,
    net: Res<NetMode>,
) -> bool {
    if *game.get() != GameState::Playing {
        return false;
    }
    if *net != NetMode::SinglePlayer {
        return true;
    }
    *pause.get() == PauseState::Running
}

fn main() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Zombies - Waves of Survival".into(),
                    resolution: (WINDOW_WIDTH, WINDOW_HEIGHT).into(),
                    mode: WindowMode::BorderlessFullscreen,
                    resizable: true,
                    resize_constraints: WindowResizeConstraints {
                        min_width: WINDOW_WIDTH,
                        min_height: WINDOW_HEIGHT,
                        ..default()
                    },
                    ..default()
                }),
                ..default()
            })
            .set(ImagePlugin::default_nearest()),
    );

    let font: Handle<Font> = app
        .world()
        .resource::<AssetServer>()
        .load("fonts/PressStart2P.ttf");
    app.insert_resource(UiAssets { font });

    app.init_state::<GameState>()
        .init_state::<PauseState>()
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .init_resource::<Score>()
        .add_plugins((
            settings::SettingsPlugin,
            net::NetPlugin,
            sync::NetSyncPlugin,
            map::MapPlugin,
            menu::MenuPlugin,
            lobby::LobbyPlugin,
            pause::PausePlugin,
            player::PlayerPlugin,
        ))
        .add_plugins((
            zombie::ZombiePlugin,
            bullet::BulletPlugin,
            weapon::WeaponPlugin,
            wave::WavePlugin,
            zones::ZonesPlugin,
            achievements::AchievementsPlugin,
            audio::AudioFxPlugin,
            ui::UiPlugin,
        ))
        .add_systems(Startup, setup_camera)
        .add_systems(
            Update,
            camera_follow.run_if(in_state(GameState::Playing)),
        )
        .run();
}

fn setup_camera(mut commands: Commands) {
    let mut camera = Camera2dBundle::default();
    camera.projection.scaling_mode = ScalingMode::FixedVertical(FIXED_VIEW_H);
    commands.spawn(camera);
}

fn camera_follow(
    windows: Query<&Window, With<PrimaryWindow>>,
    ctx: Res<NetContext>,
    players: Query<(&Transform, &Player), Without<Camera>>,
    mut camera: Query<&mut Transform, With<Camera>>,
) {
    let Ok(window) = windows.get_single() else {
        return;
    };
    let Ok(mut cam_transform) = camera.get_single_mut() else {
        return;
    };

    let target = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .or_else(|| players.iter().next())
        .map(|(t, _)| t.translation.truncate());
    let Some(target) = target else {
        return;
    };

    let aspect = if window.height() > 0.0 {
        window.width() / window.height()
    } else {
        WINDOW_WIDTH / WINDOW_HEIGHT
    };
    let view_w = FIXED_VIEW_H * aspect;

    let max_x = (MAP_WIDTH / 2.0 - view_w / 2.0).max(0.0);
    let max_y = (MAP_HEIGHT / 2.0 - FIXED_VIEW_H / 2.0).max(0.0);

    cam_transform.translation.x = target.x.clamp(-max_x, max_x);
    cam_transform.translation.y = target.y.clamp(-max_y, max_y);
}

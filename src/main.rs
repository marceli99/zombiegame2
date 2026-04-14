mod audio;
mod bullet;
mod lobby;
mod map;
mod menu;
mod net;
mod pause;
mod player;
mod sync;
mod ui;
mod wave;
mod zombie;

use bevy::prelude::*;
use bevy::render::camera::ScalingMode;
use bevy::time::Fixed;
use bevy::window::WindowResizeConstraints;

use crate::net::NetMode;

pub const WINDOW_WIDTH: f32 = 1280.0;
pub const WINDOW_HEIGHT: f32 = 720.0;
pub const TICK_HZ: f64 = 60.0;

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GameState {
    #[default]
    Menu,
    JoinPrompt,
    Lobby,
    Playing,
    GameOver,
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
                    title: "Zombiaki - Fale Przetrwania".into(),
                    resolution: (WINDOW_WIDTH, WINDOW_HEIGHT).into(),
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
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.08)))
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .init_resource::<Score>()
        .add_plugins((
            net::NetPlugin,
            sync::NetSyncPlugin,
            map::MapPlugin,
            menu::MenuPlugin,
            lobby::LobbyPlugin,
            pause::PausePlugin,
            player::PlayerPlugin,
            zombie::ZombiePlugin,
            bullet::BulletPlugin,
            wave::WavePlugin,
            audio::AudioFxPlugin,
            ui::UiPlugin,
        ))
        .add_systems(Startup, setup_camera)
        .run();
}

fn setup_camera(mut commands: Commands) {
    let mut camera = Camera2dBundle::default();
    camera.projection.scaling_mode = ScalingMode::FixedVertical(WINDOW_HEIGHT);
    commands.spawn(camera);
}

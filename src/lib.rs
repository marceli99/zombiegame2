mod achievements;
mod audio;
mod bullet;
mod chat;
mod lobby;
mod map;
mod map_data;
mod map_nav;
mod map_obstacles;
mod menu;
mod mobile;
mod net;
mod pause;
mod pixelart;
mod player;
mod settings;
mod sync;
mod ui;
mod underground;
mod wave;
mod weapon;
mod world_consts;
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
/// Vertical world height the 2D camera frames on phones.  Desktop's 760 reads
/// as "too far away" on a small screen held close, so phones render fewer world
/// units vertically (≈1.6× zoom) to make the action read bigger.  Tuned on a
/// 20:9 landscape phone; smaller = more zoom.  Shared by `setup_camera` and
/// `camera_follow` via `view_height()` so the projection and the clamp math
/// never disagree.
pub const MOBILE_VIEW_H: f32 = 360.0;
pub const TICK_HZ: f64 = 60.0;

/// True when this run uses the mobile/touch profile — a real phone (Android /
/// iOS) or the `ZG_FORCE_TOUCH` desktop preview.  Mirrors the condition that
/// lights up the on-screen touch controls so the zoom and the controls always
/// turn on together.
pub fn mobile_profile() -> bool {
    cfg!(any(target_os = "android", target_os = "ios")) || std::env::var("ZG_FORCE_TOUCH").is_ok()
}

/// Effective vertical view height for the 2D camera — the single source of
/// truth shared by `setup_camera` (projection) and `camera_follow` (clamp).
pub fn view_height() -> f32 {
    if mobile_profile() {
        MOBILE_VIEW_H
    } else {
        FIXED_VIEW_H
    }
}

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

/// Decaying screen shake.  Bumped by `accumulate_camera_shake` whenever an
/// explosion fires near the camera; `camera_follow` then jitters the
/// camera translation by `intensity` pixels each frame and decays it.
#[derive(Resource, Default)]
pub struct CameraShake {
    pub intensity: f32,
}

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

/// Builds and runs the game.  Called by the desktop binary (`src/main.rs`) and
/// by the Android entry point (`#[bevy_main]`, below).
pub fn run() {
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

    // MSAA crashes some Android GPUs/drivers; disable it there.  (Bevy's own
    // mobile example does the same.)  Desktop/iOS keep the default 4× MSAA.
    #[cfg(target_os = "android")]
    app.insert_resource(Msaa::Off);

    app.init_state::<GameState>()
        .init_state::<PauseState>()
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .init_resource::<Score>()
        .init_resource::<CameraShake>()
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
            chat::ChatPlugin,
            underground::UndergroundPlugin,
            mobile::MobileControlsPlugin,
        ))
        .add_systems(Startup, setup_camera)
        .add_systems(
            Update,
            // Camera reads Transform after `interpolate_logical_pos` has
            // lerped it between FixedUpdate ticks, so the world scrolls
            // smoothly at any render FPS instead of stepping at 60 Hz.
            (accumulate_camera_shake, camera_follow)
                .chain()
                .after(player::interpolate_logical_pos)
                .run_if(in_state(GameState::Playing)),
        )
        .run();
}

fn setup_camera(mut commands: Commands) {
    let mut camera = Camera2dBundle::default();
    camera.projection.scaling_mode = ScalingMode::FixedVertical(view_height());

    // Mobile GPUs are tiled + bandwidth-bound and choke on HDR rendering, a
    // multi-pass bloom, and a LUT-based tonemapper — and those features also
    // raise the bar for obtaining a working render adapter at all.  So on
    // Android/iOS we run a lean SDR profile: no HDR, no bloom, cheap (LUT-free)
    // tonemapping; desktop keeps the full glow look.
    //
    // Using a runtime `cfg!()` flag (not `#[cfg]`) keeps BOTH paths compiled on
    // every target, so the desktop build type-checks the mobile path too — and
    // `ZG_LOW_GFX=1` lets you preview the lean look on desktop.
    let lean_gfx = cfg!(any(target_os = "android", target_os = "ios"))
        || std::env::var("ZG_LOW_GFX").is_ok();

    if lean_gfx {
        camera.camera.hdr = false;
        camera.tonemapping = bevy::core_pipeline::tonemapping::Tonemapping::Reinhard;
        commands.spawn(camera);
    } else {
        // HDR is required for bloom: bright pixels overshoot 1.0 and the bloom
        // pass picks them up.  Tonemapping then compresses the HDR back into
        // SDR range so the image stays readable.
        camera.camera.hdr = true;
        camera.tonemapping = bevy::core_pipeline::tonemapping::Tonemapping::TonyMcMapface;
        commands.spawn((
            camera,
            // Soft glow on muzzle flashes, explosions, lamps, tracers etc.
            // The default settings are tuned for 3D, so we lean intensity down
            // a touch and threshold up to avoid bloom on every bright pixel.
            bevy::core_pipeline::bloom::BloomSettings {
                intensity: 0.18,
                low_frequency_boost: 0.5,
                low_frequency_boost_curvature: 0.95,
                high_pass_frequency: 1.0,
                prefilter_settings: bevy::core_pipeline::bloom::BloomPrefilterSettings {
                    threshold: 0.6,
                    threshold_softness: 0.3,
                },
                composite_mode: bevy::core_pipeline::bloom::BloomCompositeMode::Additive,
            },
        ));
    }
}

fn camera_follow(
    windows: Query<&Window, With<PrimaryWindow>>,
    ctx: Res<NetContext>,
    players: Query<(&Transform, &Player), Without<Camera>>,
    mut camera: Query<&mut Transform, With<Camera>>,
    mut shake: ResMut<CameraShake>,
    time: Res<Time>,
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
    let view_h = view_height();
    let view_w = view_h * aspect;

    let half_view_w = view_w * 0.5;
    let half_view_h = view_h / 2.0;

    // Camera clamp is context-dependent: while the player is on the
    // surface we clamp to the surface map rect; once they descend below
    // the surface south boundary we switch to the underground rect so
    // neither the void above the metro nor the empty space east/west of
    // its bounding box ever enters frame.
    let (min_x, max_x, min_y, max_y) = if target.y < -crate::map::MAP_HEIGHT * 0.5 {
        // Underground bounds (metro platform + tracks).
        let lo_x = crate::underground::UNDER_LEFT + half_view_w;
        let hi_x = crate::underground::UNDER_RIGHT - half_view_w;
        let lo_y = crate::underground::UNDER_BOTTOM + half_view_h;
        let hi_y = crate::underground::UNDER_TOP - half_view_h;
        // If the metro is narrower than the viewport, lock the camera to
        // its centre instead of producing an inverted clamp range.
        let (mn_x, mx_x) = if hi_x >= lo_x {
            (lo_x, hi_x)
        } else {
            let cx = crate::underground::UNDER_CX;
            (cx, cx)
        };
        let (mn_y, mx_y) = if hi_y >= lo_y {
            (lo_y, hi_y)
        } else {
            let cy = crate::underground::UNDER_CY;
            (cy, cy)
        };
        (mn_x, mx_x, mn_y, mx_y)
    } else {
        let surf_x = (MAP_WIDTH / 2.0 - half_view_w).max(0.0);
        let surf_y = (MAP_HEIGHT / 2.0 - half_view_h).max(0.0);
        (-surf_x, surf_x, -surf_y, surf_y)
    };

    let base_x = target.x.clamp(min_x, max_x);
    let base_y = target.y.clamp(min_y, max_y);

    // Apply screen shake — random offset proportional to intensity.  Decay
    // exponentially so big hits punch hard and fade smoothly.
    let shake_amount = shake.intensity;
    if shake_amount > 0.05 {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let ox = rng.gen_range(-shake_amount..shake_amount);
        let oy = rng.gen_range(-shake_amount..shake_amount);
        cam_transform.translation.x = base_x + ox;
        cam_transform.translation.y = base_y + oy;
    } else {
        cam_transform.translation.x = base_x;
        cam_transform.translation.y = base_y;
    }
    // Decay at ~6 units/sec exponential — feels snappy without lingering.
    shake.intensity -= shake.intensity * 6.0 * time.delta_seconds();
    shake.intensity = shake.intensity.max(0.0);
}

/// Reads explosion events and bumps the screen-shake intensity scaled by
/// distance to the local player — close-range explosions punch the camera
/// noticeably while far-away ones barely register.  Capped to keep things
/// readable.
fn accumulate_camera_shake(
    mut shake: ResMut<CameraShake>,
    mut events: EventReader<bullet::ExplodeEvent>,
    ctx: Res<NetContext>,
    players: Query<(&Transform, &Player)>,
) {
    let local_pos = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .or_else(|| players.iter().next())
        .map(|(t, _)| t.translation.truncate());
    let Some(p) = local_pos else {
        events.clear();
        return;
    };
    for ev in events.read() {
        let dist = ev.pos.distance(p);
        // Audible-shake range: closer than 600 px gives meaningful kick.
        let proximity = (1.0 - (dist / 600.0)).clamp(0.0, 1.0);
        let bump = ev.radius * 0.12 * proximity;
        shake.intensity = (shake.intensity + bump).min(28.0);
    }
}

/// Mobile entry point.  `#[bevy_main]` generates the platform glue the OS calls
/// into — `android_main` (NDK NativeActivity) on Android and the C-callable
/// `main_rs` (invoked from `ios/main.m`) on iOS.  Compiled only for those
/// targets; on desktop the binary (`src/main.rs`) drives `run()` directly.
#[cfg(any(target_os = "android", target_os = "ios"))]
#[bevy_main]
fn main() {
    run();
}

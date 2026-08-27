use bevy::audio::{PlaybackMode, Volume};
use bevy::prelude::*;
use std::time::Duration;

use crate::GameState;

#[derive(Event, Clone, Copy)]
pub enum SfxEvent {
    Shot,
    Hit,
    ZombieDeath,
    PlayerHit,
    Explosion,
    MenuMove,
    MenuSelect,
    MenuCancel,
    Heal,
}

/// A looping ambience layer.  `base` is its volume at 100% master, so
/// `apply_ambience_volume` can rescale the live sink when the master changes.
#[derive(Component)]
struct MenuAmbience {
    base: f32,
}

pub struct AudioFxPlugin;

impl Plugin for AudioFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<SfxEvent>()
            .add_systems(Update, (play_sfx, apply_ambience_volume))
            .add_systems(
                OnEnter(GameState::Menu),
                ensure_menu_ambience,
            )
            .add_systems(
                OnEnter(GameState::Settings),
                ensure_menu_ambience,
            )
            .add_systems(
                OnEnter(GameState::JoinPrompt),
                ensure_menu_ambience,
            )
            .add_systems(
                OnEnter(GameState::Lobby),
                ensure_menu_ambience,
            )
            .add_systems(OnEnter(GameState::Playing), stop_menu_ambience)
            .add_systems(OnEnter(GameState::GameOver), stop_menu_ambience);
    }
}

fn play_sfx(
    mut commands: Commands,
    mut events: EventReader<SfxEvent>,
    mut pitches: ResMut<Assets<Pitch>>,
    settings: Res<crate::settings::GraphicsSettings>,
) {
    for ev in events.read() {
        let (freq, ms, base_vol) = match ev {
            SfxEvent::Shot => (880.0, 45, 0.12),
            SfxEvent::Hit => (520.0, 35, 0.12),
            SfxEvent::ZombieDeath => (160.0, 180, 0.20),
            SfxEvent::PlayerHit => (110.0, 260, 0.30),
            SfxEvent::Explosion => (70.0, 320, 0.36),
            SfxEvent::MenuMove => (220.0, 35, 0.10),
            SfxEvent::MenuSelect => (146.0, 110, 0.18),
            SfxEvent::MenuCancel => (98.0, 140, 0.15),
            SfxEvent::Heal => (660.0, 100, 0.18),
        };
        let vol = base_vol * settings.volume;
        commands.spawn(PitchBundle {
            source: pitches.add(Pitch {
                frequency: freq,
                duration: Duration::from_millis(ms),
            }),
            settings: PlaybackSettings::DESPAWN.with_volume(Volume::new(vol)),
        });
    }
}

fn ensure_menu_ambience(
    mut commands: Commands,
    existing: Query<Entity, With<MenuAmbience>>,
    mut pitches: ResMut<Assets<Pitch>>,
    settings: Res<crate::settings::GraphicsSettings>,
) {
    if !existing.is_empty() {
        return;
    }
    let master = settings.volume;
    let loop_settings = |base: f32| PlaybackSettings {
        mode: PlaybackMode::Loop,
        volume: Volume::new(base * master),
        ..default()
    };
    // (frequency, base volume at 100% master)
    for (freq, base) in [(55.0, 0.12), (82.4, 0.07), (138.6, 0.04)] {
        commands.spawn((
            PitchBundle {
                source: pitches.add(Pitch {
                    frequency: freq,
                    duration: Duration::from_secs(3),
                }),
                settings: loop_settings(base),
            },
            MenuAmbience { base },
        ));
    }
}

/// Rescale the playing ambience when the master volume changes, so the slider
/// in Settings is audible immediately rather than only after re-entering a menu.
fn apply_ambience_volume(
    settings: Res<crate::settings::GraphicsSettings>,
    sinks: Query<(&MenuAmbience, &AudioSink)>,
) {
    if !settings.is_changed() {
        return;
    }
    for (layer, sink) in &sinks {
        sink.set_volume(layer.base * settings.volume);
    }
}

fn stop_menu_ambience(
    mut commands: Commands,
    q: Query<Entity, With<MenuAmbience>>,
) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

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

#[derive(Component)]
struct MenuAmbience;

pub struct AudioFxPlugin;

impl Plugin for AudioFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<SfxEvent>()
            .add_systems(Update, play_sfx)
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
) {
    for ev in events.read() {
        let (freq, ms, vol) = match ev {
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
) {
    if !existing.is_empty() {
        return;
    }
    let loop_settings = |vol: f32| PlaybackSettings {
        mode: PlaybackMode::Loop,
        volume: Volume::new(vol),
        ..default()
    };
    commands.spawn((
        PitchBundle {
            source: pitches.add(Pitch {
                frequency: 55.0,
                duration: Duration::from_secs(3),
            }),
            settings: loop_settings(0.12),
        },
        MenuAmbience,
    ));
    commands.spawn((
        PitchBundle {
            source: pitches.add(Pitch {
                frequency: 82.4,
                duration: Duration::from_secs(3),
            }),
            settings: loop_settings(0.07),
        },
        MenuAmbience,
    ));
    commands.spawn((
        PitchBundle {
            source: pitches.add(Pitch {
                frequency: 138.6,
                duration: Duration::from_secs(3),
            }),
            settings: loop_settings(0.04),
        },
        MenuAmbience,
    ));
}

fn stop_menu_ambience(
    mut commands: Commands,
    q: Query<Entity, With<MenuAmbience>>,
) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

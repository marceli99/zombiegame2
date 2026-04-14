use bevy::audio::Volume;
use bevy::prelude::*;
use std::time::Duration;

#[derive(Event, Clone, Copy)]
pub enum SfxEvent {
    Shot,
    Hit,
    ZombieDeath,
    PlayerHit,
}

pub struct AudioFxPlugin;

impl Plugin for AudioFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<SfxEvent>().add_systems(Update, play_sfx);
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

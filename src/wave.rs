use bevy::prelude::*;

use crate::net::is_authoritative;
use crate::zombie::{SpawnZombieEvent, Zombie};
use crate::{gameplay_active, GameState};

#[derive(Resource)]
pub struct WaveState {
    pub current_wave: u32,
    pub zombies_to_spawn: u32,
    pub spawn_timer: Timer,
    pub break_timer: Timer,
    pub in_break: bool,
}

impl Default for WaveState {
    fn default() -> Self {
        Self {
            current_wave: 0,
            zombies_to_spawn: 0,
            spawn_timer: Timer::from_seconds(0.4, TimerMode::Repeating),
            break_timer: Timer::from_seconds(3.0, TimerMode::Once),
            in_break: true,
        }
    }
}

pub struct WavePlugin;

impl Plugin for WavePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WaveState>()
            .add_systems(OnEnter(GameState::Playing), reset_waves)
            .add_systems(
                FixedUpdate,
                wave_system
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

fn reset_waves(mut state: ResMut<WaveState>) {
    *state = WaveState::default();
}

fn wave_system(
    time: Res<Time>,
    mut state: ResMut<WaveState>,
    mut spawn_events: EventWriter<SpawnZombieEvent>,
    zombies: Query<(), With<Zombie>>,
) {
    if state.in_break {
        state.break_timer.tick(time.delta());
        if state.break_timer.finished() {
            state.current_wave += 1;
            state.zombies_to_spawn = 4 + state.current_wave * 3;
            state.spawn_timer.reset();
            state.in_break = false;
        }
        return;
    }
    if state.zombies_to_spawn > 0 {
        state.spawn_timer.tick(time.delta());
        if state.spawn_timer.just_finished() {
            spawn_events.send(SpawnZombieEvent);
            state.zombies_to_spawn -= 1;
        }
    } else if zombies.is_empty() {
        state.in_break = true;
        state.break_timer.reset();
    }
}

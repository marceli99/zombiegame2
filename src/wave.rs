use bevy::prelude::*;
use rand::seq::SliceRandom;
use std::collections::VecDeque;

use crate::net::is_authoritative;
use crate::player::Player;
use crate::zombie::{SpawnZombieEvent, Zombie, ZombieKind};
use crate::{gameplay_active, GameState};

#[derive(Resource)]
pub struct WaveState {
    pub current_wave: u32,
    pub spawn_queue: VecDeque<ZombieKind>,
    pub zombies_to_spawn: u32,
    pub spawn_timer: Timer,
    pub break_timer: Timer,
    pub in_break: bool,
}

impl Default for WaveState {
    fn default() -> Self {
        Self {
            current_wave: 0,
            spawn_queue: VecDeque::new(),
            zombies_to_spawn: 0,
            spawn_timer: Timer::from_seconds(0.25, TimerMode::Repeating),
            break_timer: Timer::from_seconds(2.5, TimerMode::Once),
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

fn player_scale(player_count: usize) -> f32 {
    match player_count {
        2 => 1.2,
        3 => 1.4,
        4.. => 1.5,
        _ => 1.0,
    }
}

fn build_wave_queue(wave: u32, player_count: usize) -> VecDeque<ZombieKind> {
    let s = player_scale(player_count);
    let normal = ((6 + wave * 5) as f32 * s) as u32;
    let fast = if wave >= 2 {
        (wave.min(12) as f32 * s) as u32
    } else {
        0
    };
    let exploder = if wave >= 3 {
        (((wave - 2) / 2 + 1).min(8) as f32 * s) as u32
    } else {
        0
    };
    let burning = if wave >= 3 {
        (((wave - 2) / 2 + 1).min(6) as f32 * s) as u32
    } else {
        0
    };
    let giant = if wave >= 5 {
        (((wave - 4) / 2 + 1).min(3) as f32 * s) as u32
    } else {
        0
    };
    let total = (normal + fast + exploder + burning + giant) as usize;
    let mut v: Vec<ZombieKind> = Vec::with_capacity(total);
    for _ in 0..normal {
        v.push(ZombieKind::Normal);
    }
    for _ in 0..fast {
        v.push(ZombieKind::Fast);
    }
    for _ in 0..exploder {
        v.push(ZombieKind::Exploder);
    }
    for _ in 0..burning {
        v.push(ZombieKind::Burning);
    }
    for _ in 0..giant {
        v.push(ZombieKind::Giant);
    }
    let mut rng = rand::thread_rng();
    v.shuffle(&mut rng);
    VecDeque::from(v)
}

fn wave_system(
    time: Res<Time>,
    mut state: ResMut<WaveState>,
    mut spawn_events: EventWriter<SpawnZombieEvent>,
    zombies: Query<(), With<Zombie>>,
    players: Query<(), With<Player>>,
) {
    if state.in_break {
        state.break_timer.tick(time.delta());
        if state.break_timer.finished() {
            state.current_wave += 1;
            state.spawn_queue = build_wave_queue(state.current_wave, players.iter().count());
            state.zombies_to_spawn = state.spawn_queue.len() as u32;
            state.spawn_timer.reset();
            state.in_break = false;
        }
        return;
    }
    if !state.spawn_queue.is_empty() {
        state.spawn_timer.tick(time.delta());
        if state.spawn_timer.just_finished() {
            if let Some(kind) = state.spawn_queue.pop_front() {
                spawn_events.send(SpawnZombieEvent { kind });
                state.zombies_to_spawn = state.spawn_queue.len() as u32;
            }
        }
    } else if zombies.is_empty() {
        state.in_break = true;
        state.break_timer.reset();
    }
}

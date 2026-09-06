use bevy::prelude::*;
use rand::seq::SliceRandom;
use std::collections::VecDeque;

use crate::net::{is_authoritative, NetEntities};
use crate::map::{PLAYER_SPAWN_X, PLAYER_SPAWN_Y};
use crate::player::{spawn_player_entity, DeadPlayers, LogicalPos, Player, PlayerAssets};
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
    /// Consecutive ticks the clear condition (queue empty + no live zombies)
    /// has held.  The final `SpawnZombieEvent` may still be in flight —
    /// `spawn_zombie_listener` (ZombiePlugin, no ordering edge to
    /// `wave_system`) can consume it a tick after the send and materializes
    /// the zombie via deferred commands — so one empty observation isn't
    /// proof the wave is actually done.
    pub clear_streak: u8,
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
            clear_streak: 0,
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

#[allow(clippy::too_many_arguments)]
fn wave_system(
    time: Res<Time>,
    mut commands: Commands,
    mut state: ResMut<WaveState>,
    mut spawn_events: EventWriter<SpawnZombieEvent>,
    zombies: Query<(), With<Zombie>>,
    players: Query<&Transform, With<Player>>,
    mut dead_players: ResMut<DeadPlayers>,
    player_assets: Res<PlayerAssets>,
    mut net_entities: ResMut<NetEntities>,
) {
    if state.in_break {
        state.break_timer.tick(time.delta());
        if state.break_timer.finished() {
            state.current_wave += 1;
            // `dead_players` is drained at wave-clear (below) before the break,
            // so it's empty here in normal flow — counting only the live query
            // is both correct and cheaper.
            let alive = players.iter().count();
            state.spawn_queue = build_wave_queue(state.current_wave, alive);
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
        // Require two consecutive empty ticks: a zombie sent on the queue's
        // last pop can take until the next tick to reach the query (see
        // `clear_streak` docs), and clearing early would start the break
        // with a straggler still incoming.
        state.clear_streak += 1;
        if state.clear_streak < 2 {
            return;
        }
        state.clear_streak = 0;
        // Wave cleared — respawn dead players before the break
        for (i, id) in dead_players.0.drain(..).enumerate() {
            let col = i % 4;
            let row = i / 4;
            let pos = Vec2::new(
                PLAYER_SPAWN_X + col as f32 * 64.0,
                PLAYER_SPAWN_Y - row as f32 * 64.0,
            );
            let ent = spawn_player_entity(&mut commands, &player_assets, id, pos);
            // Wave respawns happen on the authoritative side (host/SP) so
            // every respawned player needs the interp buffer too.
            commands.entity(ent).insert(LogicalPos::at(pos));
            net_entities.players.insert(id, ent);
        }
        state.in_break = true;
        state.break_timer.reset();
    } else {
        state.clear_streak = 0;
    }
}

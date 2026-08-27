use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::collections::{HashSet, VecDeque};

use crate::bullet::{
    spawn_bullet_entity, spawn_explosion_entity, Bullet, BulletAssets, Explosion,
    EXPLOSION_LIFETIME,
};
use crate::chat::ChatLog;
use crate::map::MapSegmentUnlockState;
use crate::net::{
    broadcast, broadcast_frame, dq_pos, dq_radius, dq_rot, encode_frame, is_host, is_net_client,
    q_pos, q_radius, q_rot,
    ClientInEvent, ClientMsg, LocalInput, NetBulletState, NetContext, NetEntities,
    NetExplosionState, NetMode, NetPickupState, NetPlayerState, NetSnapshot, NetZombieState,
    PlayerNicknames, RemoteInputs, ServerEvent, ServerMsg,
};
use crate::player::{
    apply_input_to_local, spawn_player_entity, InputHistory, LogicalPos, Player, PlayerAssets,
    PlayerDiedEvent,
};
use crate::map::MapObstacles;
use crate::wave::WaveState;
use crate::weapon::{
    spawn_armor_entity, spawn_health_entity, spawn_money_entity, spawn_pickup_entity, ArmorPickup,
    ExtraPickupAssets, HealthPickup, MoneyMultPickup, Weapon, WeaponAssets, WeaponPickup,
    ARMOR_PICKUP_KIND, HEALTH_PICKUP_KIND, MONEY2X_PICKUP_KIND, MONEY3X_PICKUP_KIND,
};
use crate::zombie::{spawn_zombie_entity, Zombie, ZombieAssets, ZombieKind};
use crate::{GameState, Score};

// ─── Snapshot rate / interpolation tuning ───────────────────────────────
//
// Server broadcasts every Nth `FixedUpdate` tick (60 Hz).  At N=1 snapshots
// ship at the full 60 Hz simulation rate.  Inputs flow independently at
// 60 Hz too (they're cheap and latency-sensitive).
//
// 30 Hz (N=2) was visibly choppy on remote players even with interpolation —
// alpha tracking at 60 Hz halves the worst-case stale-data window from
// ~33 ms to ~17 ms, which on a 144 Hz monitor moves remote movement from
// "stair-stepped" to "fluid".  Bandwidth doubles vs. 30 Hz but a typical
// snapshot is well under 2 KB, so we're nowhere near the 256 KB cap on LAN.
const SNAPSHOT_INTERVAL_TICKS: u64 = 1;
/// Approximate gap between snapshots in seconds; used to size the interp
/// buffer and the render-time delay.
const SNAPSHOT_INTERVAL_SECS: f32 = SNAPSHOT_INTERVAL_TICKS as f32 / 60.0;
/// Render remote entities this far behind the newest snapshot — large
/// enough to ride out a couple of dropped/late snapshots without falling
/// off the edge of the buffer (which would freeze remote pose at the
/// newest sample for a frame).  ~3 snapshot intervals = ~50 ms of jitter
/// budget at 60 Hz — barely perceptible on movement, very forgiving on
/// transient TCP stalls.
const INTERP_DELAY_SECS: f32 = SNAPSHOT_INTERVAL_SECS * 3.0;
/// How long we keep snapshots in memory for interpolation lookups.
const HISTORY_RETAIN_SECS: f32 = 0.5;

/// Buffered snapshots for interpolation.  Latest is the back; oldest is the
/// front.  `last_applied_tick` is bumped each time we run lifecycle apply.
#[derive(Resource, Default)]
pub struct SnapshotHistory {
    pub entries: VecDeque<BufferedSnapshot>,
    pub last_applied_tick: u64,
}

pub struct BufferedSnapshot {
    pub received: f64,
    pub snap: Box<NetSnapshot>,
}

impl SnapshotHistory {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last_applied_tick = 0;
    }
}

pub struct NetSyncPlugin;

impl Plugin for NetSyncPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SnapshotHistory>()
            .init_resource::<ApplyScratch>()
            .add_systems(
                FixedUpdate,
                (server_receive_inputs, server_broadcast_snapshot)
                    .run_if(in_state(GameState::Playing))
                    .run_if(is_host),
            )
            .add_systems(
                FixedUpdate,
                client_send_input
                    .run_if(in_state(GameState::Playing))
                    .run_if(is_net_client),
            )
            .add_systems(
                Update,
                client_apply_snapshots
                    .run_if(in_state(GameState::Playing))
                    .run_if(is_net_client),
            )
            .add_systems(OnExit(GameState::Playing), reset_snapshot_history);
    }
}

fn reset_snapshot_history(
    mut history: ResMut<SnapshotHistory>,
    mut input_history: ResMut<InputHistory>,
) {
    history.clear();
    // Drop unacknowledged inputs from the finished round.  A fresh round-2
    // player spawns with `last_processed_seq: 0`, so leftover entries (whose
    // seqs are large — `next_seq` is monotonic across rounds) would never be
    // trimmed by ack, and the reconciliation replay would drag the local
    // player off spawn for the first frames of a rematch.  `next_seq` itself
    // stays monotonic so a stale round-1 ack can never trim round-2 inputs.
    input_history.buffer.clear();
}

#[allow(clippy::too_many_arguments)]
fn server_receive_inputs(
    mut commands: Commands,
    mut ctx: ResMut<NetContext>,
    mut remote: ResMut<RemoteInputs>,
    mut nicknames: ResMut<PlayerNicknames>,
    mut chat_log: ResMut<ChatLog>,
    mut net_entities: ResMut<crate::net::NetEntities>,
    players: Query<(Entity, &Player)>,
) {
    let Some(host) = ctx.host.as_ref() else {
        return;
    };
    let events_arc = host.events.clone();
    let mut events = Vec::new();
    if let Ok(rx) = events_arc.lock() {
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
    }
    // One client can have shipped multiple input packets into a single drain
    // (host frame hitch, clock jitter) — track which ids we've already seen
    // this drain so one-shot flags can be OR-merged *within* the drain only.
    let mut updated_this_drain: HashSet<u8> = HashSet::new();
    for e in events {
        match e {
            ServerEvent::Input { id, input } => {
                // Sanitise BEFORE merge — a malicious / buggy client could
                // have sent NaN/Inf or out-of-range slot ids that we must
                // not feed to the simulation.
                let mut merged = input;
                merged.sanitize();
                if let Some(prev) = remote.0.get(&id) {
                    // Merge one-shot switch_slot to prevent lost inputs —
                    // sticky across ticks until the simulation consumes it.
                    if merged.switch_slot == 0 {
                        merged.switch_slot = prev.switch_slot;
                    }
                    // Other one-shots (throw/reload/interact): merge ONLY
                    // against packets from this same drain — the previous
                    // tick's entry was already consumed by the simulation,
                    // so OR-ing across ticks would double-fire grenade
                    // throws, reloads and purchases.
                    if updated_this_drain.contains(&id) {
                        merged.throw |= prev.throw;
                        merged.reload |= prev.reload;
                        merged.interact |= prev.interact;
                    }
                }
                remote.0.insert(id, merged);
                updated_this_drain.insert(id);
            }
            ServerEvent::Connected { id } => {
                info!("Client {} connected mid-game (not spawning)", id);
            }
            ServerEvent::Hello { id, nickname } => {
                nicknames.0.insert(id, nickname);
            }
            ServerEvent::Disconnected { id } => {
                remote.0.remove(&id);
                nicknames.0.remove(&id);
                ctx.lobby_players.retain(|p| *p != id);
                // Despawn the host-side Player entity for the gone client —
                // otherwise it lingers as a stationary body forever, gets
                // shipped in every snapshot, and still soaks up zombie
                // collisions without being reachable by anyone.
                if let Some(ent) = net_entities.players.remove(&id) {
                    if players.get(ent).is_ok() {
                        commands.entity(ent).despawn_recursive();
                    }
                }
            }
            ServerEvent::ChatRelay { id, text } => {
                let author = nicknames
                    .0
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| format!("P{id}"));
                chat_log.push(author.clone(), text.clone());
                if let Some(host) = ctx.host.as_ref() {
                    broadcast(host, &ServerMsg::Chat { author, text });
                }
            }
        }
    }
}

/// Tracks state needed to skip unchanged sub-fields between snapshots.
#[derive(Default)]
struct BroadcastState {
    /// Sorted ids of last broadcast pickups — same set ⇒ skip the field.
    last_pickup_ids: Vec<u32>,
    /// Tick when nicknames last changed — we re-send when our local hash
    /// of (id, nickname) pairs differs.  Cheap on a 4-player cap.
    last_nicknames: Vec<(u8, String)>,
    tick: u64,
}

/// Low-churn scalar world state bundled into one `SystemParam` so
/// `server_broadcast_snapshot` keeps room under Bevy's 16-arg limit after
/// gaining the explodables query.
#[derive(SystemParam)]
struct SnapshotMeta<'w> {
    score: Res<'w, Score>,
    segments: Res<'w, MapSegmentUnlockState>,
    destroyed: Res<'w, crate::map::DestroyedExplodables>,
    game_state: Res<'w, State<GameState>>,
}

#[allow(clippy::too_many_arguments)]
fn server_broadcast_snapshot(
    ctx: Res<NetContext>,
    players: Query<(&Transform, &Player)>,
    zombies: Query<(&Transform, &crate::net::NetId, &Zombie)>,
    bullets: Query<(&Transform, &crate::net::NetId, &Bullet)>,
    pickups: Query<(&Transform, &crate::net::NetId, &WeaponPickup)>,
    health_pickups: Query<(&Transform, &crate::net::NetId), With<HealthPickup>>,
    armor_pickups: Query<(&Transform, &crate::net::NetId), With<ArmorPickup>>,
    money_pickups: Query<(&Transform, &crate::net::NetId, &MoneyMultPickup)>,
    explosions: Query<(&Transform, &crate::net::NetId, &Explosion)>,
    explodables: Query<(&crate::map::Explodable, &crate::map::ExplodableObstacleIdx)>,
    wave: Res<WaveState>,
    nicknames: Res<PlayerNicknames>,
    meta: SnapshotMeta,
    mut bcast: Local<BroadcastState>,
) {
    let Some(host) = ctx.host.as_ref() else {
        return;
    };
    bcast.tick += 1;
    // Nobody connected (solo-hosted lobby, or every client disconnected
    // mid-match) — skip building + serializing a snapshot no one receives.
    // Tick keeps counting so it stays monotonic.  Reset the delta-tracking
    // state so the first snapshot once a client *is* connected (next round —
    // mid-round joins are rejected) re-sends the full pickup/nickname fields.
    if host.senders.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
        bcast.last_pickup_ids.clear();
        bcast.last_nicknames.clear();
        return;
    }
    let tick = bcast.tick;
    // Skip ticks per `SNAPSHOT_INTERVAL_TICKS` (currently 1 = 60 Hz), but
    // always send tick 1 so a client sees world state immediately on join.
    if tick > 1 && !tick.is_multiple_of(SNAPSHOT_INTERVAL_TICKS) {
        return;
    }

    let player_states: Vec<NetPlayerState> = players
        .iter()
        .map(|(t, p)| NetPlayerState {
            id: p.id,
            x: q_pos(t.translation.x),
            y: q_pos(t.translation.y),
            rot: q_rot(t.rotation.to_euler(EulerRot::ZYX).0),
            // HP/armor zawsze w 0..=PLAYER_MAX_HP (100), więc i16 to overkill
            // ale spójność z resztą snapshot fields.
            hp: p.hp.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            armor: p.armor.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            active_slot: p.active_slot,
            slot1_weapon: p.slots[1].map(|w| w.as_u8()).unwrap_or(255),
            last_processed_seq: p.last_processed_seq,
        })
        .collect();

    let zombie_states: Vec<NetZombieState> = zombies
        .iter()
        .map(|(t, id, z)| NetZombieState {
            id: id.0,
            x: q_pos(t.translation.x),
            y: q_pos(t.translation.y),
            rot: q_rot(t.rotation.to_euler(EulerRot::ZYX).0),
            kind: z.kind.as_u8(),
            hp: z.hp.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        })
        .collect();

    let bullet_states: Vec<NetBulletState> = bullets
        .iter()
        .map(|(t, id, b)| NetBulletState {
            id: id.0,
            x: q_pos(t.translation.x),
            y: q_pos(t.translation.y),
            rot: q_rot(t.rotation.to_euler(EulerRot::ZYX).0),
            is_rocket: b.is_rocket,
        })
        .collect();

    let mut pickup_states: Vec<NetPickupState> = pickups
        .iter()
        .map(|(t, id, pk)| NetPickupState {
            id: id.0,
            x: q_pos(t.translation.x),
            y: q_pos(t.translation.y),
            kind: pk.kind.as_u8(),
        })
        .collect();
    for (t, id) in &health_pickups {
        pickup_states.push(NetPickupState {
            id: id.0,
            x: q_pos(t.translation.x),
            y: q_pos(t.translation.y),
            kind: HEALTH_PICKUP_KIND,
        });
    }
    for (t, id) in &armor_pickups {
        pickup_states.push(NetPickupState {
            id: id.0,
            x: q_pos(t.translation.x),
            y: q_pos(t.translation.y),
            kind: ARMOR_PICKUP_KIND,
        });
    }
    for (t, id, mp) in &money_pickups {
        pickup_states.push(NetPickupState {
            id: id.0,
            x: q_pos(t.translation.x),
            y: q_pos(t.translation.y),
            kind: if mp.factor >= 3 {
                MONEY3X_PICKUP_KIND
            } else {
                MONEY2X_PICKUP_KIND
            },
        });
    }
    // Decide whether to skip the pickups field — the set is "stable" if the
    // sorted id list matches the previous tick's broadcast.  Pickups are
    // static (no per-tick movement), so id-set equality ⇒ snapshot equality.
    let mut current_pickup_ids: Vec<u32> = pickup_states.iter().map(|p| p.id).collect();
    current_pickup_ids.sort_unstable();
    let pickups_field = if current_pickup_ids == bcast.last_pickup_ids {
        None
    } else {
        bcast.last_pickup_ids = current_pickup_ids;
        Some(pickup_states)
    };

    let explosion_states: Vec<NetExplosionState> = explosions
        .iter()
        .map(|(t, id, exp)| NetExplosionState {
            id: id.0,
            x: q_pos(t.translation.x),
            y: q_pos(t.translation.y),
            radius: q_radius(exp.radius),
            remaining_ms: (exp.lifetime * 1000.0).clamp(0.0, u16::MAX as f32) as u16,
        })
        .collect();

    // Same trick for nicknames — only re-send the table when (id, name)
    // pairs differ from last tick.  We sort by id so HashMap order doesn't
    // create false-positive diffs.
    let mut current_nicks: Vec<(u8, String)> =
        nicknames.0.iter().map(|(&id, n)| (id, n.clone())).collect();
    current_nicks.sort_by_key(|&(id, _)| id);
    let nicknames_field = if current_nicks == bcast.last_nicknames {
        None
    } else {
        bcast.last_nicknames = current_nicks.clone();
        Some(current_nicks)
    };

    // Explodables that are hurt-but-alive: ship their visual damage stage so
    // clients can smoke/burn the same wreck the host is.  Destroyed ones are
    // already gone from this query (despawned on detonation) and travel in
    // `destroyed_explodables` instead, so the two sets never overlap.
    let damaged_explodables: Vec<(u32, u8)> = explodables
        .iter()
        .filter_map(|(expl, idx)| {
            let stage = expl.damage_stage();
            (stage >= 1).then_some((idx.0 as u32, stage))
        })
        .collect();

    let snap = NetSnapshot {
        tick,
        players: player_states,
        zombies: zombie_states,
        bullets: bullet_states,
        pickups: pickups_field,
        explosions: explosion_states,
        score: meta.score.0,
        wave: wave.current_wave,
        in_break: wave.in_break,
        break_ms: (wave.break_timer.remaining_secs() * 1000.0).clamp(0.0, u16::MAX as f32) as u16,
        zombies_to_spawn: wave.zombies_to_spawn,
        game_over: *meta.game_state.get() == GameState::GameOver,
        unlocked_segments_mask: meta.segments.as_mask(),
        player_nicknames: nicknames_field,
        destroyed_explodables: meta.destroyed.indices.iter().copied().collect(),
        damaged_explodables,
    };

    // Snapshots are identical for every client, so serialize once into a
    // shared frame and hand each writer a cheap Arc clone — instead of deep
    // cloning + re-serializing the whole snapshot per connection on the main
    // thread.  (If serialization ever fails we just skip this tick.)
    if let Some(frame) = encode_frame(&ServerMsg::Snapshot(Box::new(snap))) {
        broadcast_frame(host, &frame);
    }
}

fn client_send_input(
    ctx: Res<NetContext>,
    mut local: ResMut<LocalInput>,
    mut history: ResMut<crate::player::InputHistory>,
) {
    if let Some(client) = ctx.client.as_ref() {
        // Stamp a fresh sequence number — client_send_input fires once per
        // FixedUpdate, so seq increments at exactly the simulation rate.
        // Server echoes the highest seq it processed back in NetPlayerState
        // so we can ack-and-trim our local history below.
        history.next_seq = history.next_seq.wrapping_add(1);
        local.0.seq = history.next_seq;
        history.push(&local.0);
        let _ = client.sender.send(ClientMsg::Input(local.0));
    }
}

/// Bundled asset handles — the snapshot apply system already pushes Bevy's
/// 16-param-per-system limit, so we group these into one `SystemParam`.
#[derive(SystemParam)]
struct SnapshotAssets<'w> {
    player: Res<'w, PlayerAssets>,
    zombie: Res<'w, ZombieAssets>,
    bullet: Res<'w, BulletAssets>,
    weapon: Res<'w, WeaponAssets>,
    extra: Res<'w, ExtraPickupAssets>,
}

/// Same trick: groups all the mutable game-state resources `client_apply_snapshots`
/// touches into a single `SystemParam`, so the 16-arg limit isn't blown when
/// we add scratch buffers / future state.
#[derive(SystemParam)]
struct SnapshotApplyCtx<'w> {
    score: ResMut<'w, Score>,
    wave: ResMut<'w, WaveState>,
    segments: ResMut<'w, MapSegmentUnlockState>,
    nicknames: ResMut<'w, PlayerNicknames>,
    scratch: ResMut<'w, ApplyScratch>,
    input_history: ResMut<'w, InputHistory>,
    obstacles: Res<'w, MapObstacles>,
    chat_log: ResMut<'w, ChatLog>,
    died_evw: EventWriter<'w, PlayerDiedEvent>,
    boss_spawn_evw: EventWriter<'w, crate::zombie::SpawnZombieEvent>,
    destroyed: ResMut<'w, crate::map::DestroyedExplodables>,
}

/// Find the (older, newer) snapshot pair whose received-times bracket
/// `render_time`, plus the [0, 1] alpha for lerping between them.
///
/// Edge cases:
/// - Single snapshot in buffer → `(Some(it), Some(it), 0.0)` (no interp).
/// - render_time before everything → use the two oldest, alpha 0.
/// - render_time after newest → use the two newest, alpha 1 (frozen at
///   newest; we deliberately don't extrapolate).
fn find_interp_pair(
    history: &VecDeque<BufferedSnapshot>,
    render_time: f64,
) -> Option<(usize, usize, f32)> {
    if history.is_empty() {
        return None;
    }
    if history.len() == 1 {
        return Some((0, 0, 0.0));
    }
    // Walk forward looking for the first entry with received >= render_time.
    let newer_idx = history
        .iter()
        .position(|e| e.received >= render_time)
        .unwrap_or(history.len() - 1);
    let older_idx = newer_idx.saturating_sub(1);
    if older_idx == newer_idx {
        return Some((older_idx, newer_idx, 0.0));
    }
    let older = &history[older_idx];
    let newer = &history[newer_idx];
    let span = (newer.received - older.received).max(1e-6);
    let alpha = ((render_time - older.received) / span).clamp(0.0, 1.0) as f32;
    Some((older_idx, newer_idx, alpha))
}

#[inline]
fn lerp_pos(o: (f32, f32), n: (f32, f32), a: f32) -> Vec2 {
    Vec2::new(o.0 + (n.0 - o.0) * a, o.1 + (n.1 - o.1) * a)
}

/// Picks the interpolated position from `(older, newer)` snapshot lookups,
/// falling back to whichever side has the entity (or `fallback` if neither
/// does — usually a freshly-spawned entity present only in `lifecycle`).
#[inline]
fn interp_or_fallback(
    older: Option<&(f32, f32)>,
    newer: Option<&(f32, f32)>,
    alpha: f32,
    fallback: (f32, f32),
) -> Vec2 {
    match (older, newer) {
        (Some(&o), Some(&n)) => lerp_pos(o, n, alpha),
        (_, Some(&n)) => Vec2::new(n.0, n.1),
        (Some(&o), None) => Vec2::new(o.0, o.1),
        (None, None) => Vec2::new(fallback.0, fallback.1),
    }
}

/// Despawns every entity whose key is not in `seen`, leaving the surviving
/// keys in `map`.  Used to drop replicated entities that the latest
/// snapshot no longer references.
fn despawn_stale<K>(
    commands: &mut Commands,
    map: &mut std::collections::HashMap<K, Entity>,
    seen: &HashSet<K>,
) where
    K: Eq + std::hash::Hash + Copy,
{
    let stale: Vec<K> = map.keys().filter(|k| !seen.contains(k)).copied().collect();
    for k in stale {
        if let Some(ent) = map.remove(&k) {
            commands.entity(ent).despawn_recursive();
        }
    }
}

/// Re-usable scratch buffers for snapshot application.  Held as a Resource
/// (instead of `Local`) so we don't blow Bevy's 16-param-per-system limit on
/// `client_apply_snapshots`, which already pulls a lot of state.  The values
/// are reset each tick — only the underlying allocations persist.
#[derive(Resource, Default)]
struct ApplyScratch {
    older_players: std::collections::HashMap<u8, (f32, f32)>,
    newer_players: std::collections::HashMap<u8, (f32, f32)>,
    older_zombies: std::collections::HashMap<u32, (f32, f32)>,
    newer_zombies: std::collections::HashMap<u32, (f32, f32)>,
    older_bullets: std::collections::HashMap<u32, (f32, f32)>,
    newer_bullets: std::collections::HashMap<u32, (f32, f32)>,
    seen_players: HashSet<u8>,
    seen_zombies: HashSet<u32>,
    seen_bullets: HashSet<u32>,
    seen_pickups: HashSet<u32>,
    seen_explosions: HashSet<u32>,
    new_snaps: Vec<Box<NetSnapshot>>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn client_apply_snapshots(
    time: Res<Time>,
    mut history: ResMut<SnapshotHistory>,
    mut commands: Commands,
    ctx: ResMut<NetContext>,
    mut mode: ResMut<NetMode>,
    mut net_entities: ResMut<NetEntities>,
    assets: SnapshotAssets,
    mut players: Query<
        (&mut Transform, &mut Player, Option<&mut LogicalPos>),
        (Without<Zombie>, Without<Bullet>, Without<Explosion>),
    >,
    mut zombies: Query<
        (&mut Transform, &mut Zombie),
        (Without<Player>, Without<Bullet>, Without<Explosion>),
    >,
    mut bullets: Query<
        &mut Transform,
        (With<Bullet>, Without<Player>, Without<Zombie>, Without<Explosion>),
    >,
    mut explosions: Query<
        (&mut Explosion, &mut Sprite),
        (Without<Player>, Without<Zombie>, Without<Bullet>),
    >,
    // Smolder mirror — disjoint from every query above (touches neither
    // Transform nor the marker components), so no archetype conflict.
    mut explodable_vis: Query<(
        &crate::map::ExplodableObstacleIdx,
        &mut crate::map::ExplodableSmolder,
    )>,
    mut next_state: ResMut<NextState<GameState>>,
    mut apply_ctx: SnapshotApplyCtx,
) {
    let score = &mut apply_ctx.score;
    let wave = &mut apply_ctx.wave;
    let segments = &mut apply_ctx.segments;
    let nicknames = &mut apply_ctx.nicknames;
    let scratch = &mut apply_ctx.scratch;
    let input_history = &mut apply_ctx.input_history;
    let obstacles = &apply_ctx.obstacles;
    let chat_log = &mut apply_ctx.chat_log;
    let died_evw = &mut apply_ctx.died_evw;
    let boss_spawn_evw = &mut apply_ctx.boss_spawn_evw;
    let destroyed = &mut apply_ctx.destroyed;
    let Some(client) = ctx.client.as_ref() else {
        return;
    };
    let events_arc = client.events.clone();

    // ── 1. Drain incoming events ──────────────────────────────────────────
    scratch.new_snaps.clear();
    let mut disconnect = false;
    {
        let Ok(rx) = events_arc.lock() else {
            return;
        };
        while let Ok(e) = rx.try_recv() {
            match e {
                ClientInEvent::Snapshot(s) => {
                    // Drop stale snapshots — TCP guarantees order, but be
                    // defensive in case we ever swap the transport.
                    if s.tick > history.last_applied_tick {
                        scratch.new_snaps.push(s);
                    }
                }
                ClientInEvent::Disconnected
                | ClientInEvent::FullLobby
                | ClientInEvent::GameInProgress
                | ClientInEvent::ProtocolMismatch { .. } => {
                    disconnect = true;
                }
                ClientInEvent::Chat { author, text } => {
                    chat_log.push(author, text);
                }
                _ => {}
            }
        }
    }

    if disconnect {
        let mut ctx = ctx;
        ctx.disconnect();
        *mode = NetMode::SinglePlayer;
        net_entities.clear();
        history.clear();
        next_state.set(GameState::Menu);
        return;
    }

    // ── 2. Push new snapshots into the history buffer ─────────────────────
    let now = time.elapsed_seconds_f64();
    for s in scratch.new_snaps.drain(..) {
        history.entries.push_back(BufferedSnapshot {
            received: now,
            snap: s,
        });
    }

    // Trim history that's outside our retain window — but always keep at
    // least two so interpolation can still find a pair.
    let cutoff = now - HISTORY_RETAIN_SECS as f64;
    while history.entries.len() > 2 {
        let drop = history
            .entries
            .front()
            .map(|e| e.received < cutoff)
            .unwrap_or(false);
        if !drop {
            break;
        }
        history.entries.pop_front();
    }

    // Need at least one snapshot before we can do anything.
    let Some(latest_entry) = history.entries.back() else {
        return;
    };
    let latest_tick = latest_entry.snap.tick;
    // Snapshots arrive at 60 Hz but this system runs once per render frame
    // (120 Hz displays are common on the mobile targets).  Everything derived
    // purely from the latest snapshot's *content* — scalar state, the
    // reconciliation replay, spawns/despawns, lifecycle mirrors — is
    // identical until a new snapshot lands, so it's gated on `has_new`; only
    // the render-time interpolation writes stay on the per-frame path.
    let has_new = latest_tick > history.last_applied_tick;

    // ── 3. Apply scalar wave/score state from latest ──────────────────────
    let lifecycle = latest_entry.snap.as_ref();
    if has_new {
        score.0 = lifecycle.score;
        wave.current_wave = lifecycle.wave;
        wave.in_break = lifecycle.in_break;
        wave.zombies_to_spawn = lifecycle.zombies_to_spawn;
        wave.break_timer = Timer::from_seconds(
            ((lifecycle.break_ms as f32) / 1000.0).max(0.01),
            TimerMode::Once,
        );
        segments.apply_mask(lifecycle.unlocked_segments_mask);
        // Nicknames: None ⇒ unchanged, keep what we have.  Some(list) ⇒ rebuild.
        if let Some(nicks) = lifecycle.player_nicknames.as_ref() {
            nicknames.0.clear();
            for (id, n) in nicks {
                nicknames.0.insert(*id, n.clone());
            }
        }
        // Destroyed explodables: full set on every snapshot, so a join-in-progress
        // client catches up instantly.  We only write when the wire set differs
        // from local — `is_changed` then triggers the cleanup system in map.rs
        // to despawn matching entities + zero their obstacle shape.
        let mut wire_set: HashSet<u32> =
            lifecycle.destroyed_explodables.iter().copied().collect();
        if wire_set != destroyed.indices {
            std::mem::swap(&mut destroyed.indices, &mut wire_set);
        }
    }

    // ── 4. Find interpolation pair for remote-entity poses ────────────────
    let render_time = now - INTERP_DELAY_SECS as f64;
    let pair = find_interp_pair(&history.entries, render_time);
    let (older_snap, newer_snap, alpha) = match pair {
        Some((oi, ni, a)) => (
            history.entries.get(oi).map(|e| e.snap.as_ref()),
            history.entries.get(ni).map(|e| e.snap.as_ref()),
            a,
        ),
        None => (None, None, 0.0),
    };

    // Pre-build id→pos lookups so per-entity interpolation is O(1).
    // Re-use scratch HashMaps from `Local` state — saves 6 alloc/free
    // cycles per frame.
    scratch.older_players.clear();
    scratch.newer_players.clear();
    scratch.older_zombies.clear();
    scratch.newer_zombies.clear();
    scratch.older_bullets.clear();
    scratch.newer_bullets.clear();
    if let Some(o) = older_snap {
        for p in &o.players {
            scratch.older_players.insert(p.id, (dq_pos(p.x), dq_pos(p.y)));
        }
        for z in &o.zombies {
            scratch.older_zombies.insert(z.id, (dq_pos(z.x), dq_pos(z.y)));
        }
        for b in &o.bullets {
            scratch.older_bullets.insert(b.id, (dq_pos(b.x), dq_pos(b.y)));
        }
    }
    if let Some(n) = newer_snap {
        for p in &n.players {
            scratch.newer_players.insert(p.id, (dq_pos(p.x), dq_pos(p.y)));
        }
        for z in &n.zombies {
            scratch.newer_zombies.insert(z.id, (dq_pos(z.x), dq_pos(z.y)));
        }
        for b in &n.bullets {
            scratch.newer_bullets.insert(b.id, (dq_pos(b.x), dq_pos(b.y)));
        }
    }

    // ── 5. Players: lifecycle + interpolated remote positions ─────────────
    let my_id = ctx.my_id;
    if has_new {
        scratch.seen_players.clear();
    }
    for np in &lifecycle.players {
        if has_new {
            scratch.seen_players.insert(np.id);
        }
        match net_entities.players.get(&np.id).copied() {
            Some(ent) => {
                if let Ok((mut t, mut p, mut lp)) = players.get_mut(ent) {
                    if has_new {
                        p.hp = np.hp as i32;
                        p.armor = np.armor as i32;
                        p.active_slot = np.active_slot;
                        if np.slot1_weapon == 255 {
                            p.slots[1] = None;
                        } else {
                            p.slots[1] = Some(Weapon::from_u8(np.slot1_weapon));
                        }
                    }
                    let nx = dq_pos(np.x);
                    let ny = dq_pos(np.y);

                    if np.id == my_id {
                        // Between snapshots the local player is advanced by
                        // `client_local_predict` (FixedUpdate) + the LogicalPos
                        // render interpolation — reconciliation only has new
                        // information to act on when a snapshot arrived.
                        if !has_new {
                            continue;
                        }
                        // Local player: ack-based reconciliation.  Drop
                        // already-processed inputs from history, snap to the
                        // server's authoritative position, then replay the
                        // unacknowledged tail.  This keeps the local player
                        // perfectly responsive (every input is felt
                        // immediately) while staying server-authoritative —
                        // any divergence is corrected within one snapshot
                        // round-trip instead of accumulating.
                        input_history.ack(np.last_processed_seq);

                        // Snap transform & component state to server truth.
                        t.translation.x = nx;
                        t.translation.y = ny;
                        if let Some(lp) = lp.as_mut() {
                            lp.prev = Vec2::new(nx, ny);
                            lp.curr = Vec2::new(nx, ny);
                        }

                        // Replay each pending input as one fixed-step tick.
                        // We use the fixed dt (60 Hz) rather than render dt
                        // so the replay matches the host's simulation rate
                        // exactly — otherwise predicted pos drifts.
                        let fixed_dt = 1.0 / crate::TICK_HZ as f32;
                        // Take a snapshot of the queue so the borrow on
                        // input_history doesn't conflict with mutating
                        // player/transform inside the loop.  Cheap because
                        // history is bounded to 240 entries (≤4 s).
                        let pending: Vec<crate::net::NetInput> = input_history
                            .buffer
                            .iter()
                            .map(|(_, inp)| *inp)
                            .collect();
                        for inp in &pending {
                            apply_input_to_local(&mut t, &mut p, inp, obstacles, fixed_dt);
                        }
                        if let Some(mut lp) = lp {
                            // Make the FixedFirst restore land on the
                            // post-replay position so interpolation in
                            // Update doesn't snap the avatar back.
                            lp.curr.x = t.translation.x;
                            lp.curr.y = t.translation.y;
                            lp.prev = lp.curr;
                        }
                    } else {
                        // Remote player: render-time interpolation between
                        // the two history snapshots bracketing render_time.
                        // Remote players don't carry LogicalPos.
                        let target = interp_or_fallback(
                            scratch.older_players.get(&np.id),
                            scratch.newer_players.get(&np.id),
                            alpha,
                            (nx, ny),
                        );
                        t.translation.x = target.x;
                        t.translation.y = target.y;
                        t.rotation = Quat::from_rotation_z(dq_rot(np.rot));
                    }
                }
            }
            None => {
                if has_new {
                    let pos = Vec2::new(dq_pos(np.x), dq_pos(np.y));
                    let ent = spawn_player_entity(&mut commands, &assets.player, np.id, pos);
                    // Only the local player gets a LogicalPos — remote players
                    // are interpolated solely via the snapshot history buffer.
                    if np.id == my_id {
                        commands.entity(ent).insert(LogicalPos::at(pos));
                    }
                    net_entities.players.insert(np.id, ent);
                }
            }
        }
    }
    if has_new {
        // Before tearing down stale player entities, capture their last-known
        // pose so we can fire `PlayerDiedEvent` — the death-animation listener
        // turns it into a corpse sprite.  Without this, MP clients would just
        // see other players pop out of existence with no visual cue.
        for (&id, ent) in net_entities.players.iter() {
            if scratch.seen_players.contains(&id) {
                continue;
            }
            if let Ok((t, p, _)) = players.get_mut(*ent) {
                died_evw.send(PlayerDiedEvent {
                    player_id: id,
                    pos: t.translation.truncate(),
                    aim_rot: p.aim.y.atan2(p.aim.x),
                });
            }
        }
        despawn_stale(&mut commands, &mut net_entities.players, &scratch.seen_players);
    }

    // ── 6. Zombies: interpolated positions, snapped rotation ──────────────
    if has_new {
        scratch.seen_zombies.clear();
    }
    for nz in &lifecycle.zombies {
        if has_new {
            scratch.seen_zombies.insert(nz.id);
        }
        match net_entities.zombies.get(&nz.id).copied() {
            Some(ent) => {
                if let Ok((mut t, mut zomb)) = zombies.get_mut(ent) {
                    let target = interp_or_fallback(
                        scratch.older_zombies.get(&nz.id),
                        scratch.newer_zombies.get(&nz.id),
                        alpha,
                        (dq_pos(nz.x), dq_pos(nz.y)),
                    );
                    t.translation.x = target.x;
                    t.translation.y = target.y;
                    t.rotation = Quat::from_rotation_z(dq_rot(nz.rot));
                    // Mirror authoritative HP — drives the giant HP bar
                    // and any future per-zombie health overlays.
                    if has_new {
                        zomb.hp = nz.hp as i32;
                    }
                }
            }
            None => {
                if has_new {
                    let kind = ZombieKind::from_u8(nz.kind);
                    let ent = spawn_zombie_entity(
                        &mut commands,
                        &assets.zombie,
                        Vec2::new(dq_pos(nz.x), dq_pos(nz.y)),
                        nz.id,
                        nz.hp as i32,
                        kind.base_speed(),
                        kind,
                    );
                    net_entities.zombies.insert(nz.id, ent);
                    // Newly-appearing Giant on the wire: fire the boss-alert
                    // event locally so the UI flash plays on every client, not
                    // just the host where the spawn listener ran.
                    if matches!(kind, ZombieKind::Giant) {
                        boss_spawn_evw.send(crate::zombie::SpawnZombieEvent { kind });
                    }
                }
            }
        }
    }
    if has_new {
        despawn_stale(&mut commands, &mut net_entities.zombies, &scratch.seen_zombies);
    }

    // ── 7. Bullets: interpolated positions ────────────────────────────────
    if has_new {
        scratch.seen_bullets.clear();
    }
    for nb in &lifecycle.bullets {
        if has_new {
            scratch.seen_bullets.insert(nb.id);
        }
        match net_entities.bullets.get(&nb.id).copied() {
            Some(ent) => {
                if let Ok(mut t) = bullets.get_mut(ent) {
                    let target = interp_or_fallback(
                        scratch.older_bullets.get(&nb.id),
                        scratch.newer_bullets.get(&nb.id),
                        alpha,
                        (dq_pos(nb.x), dq_pos(nb.y)),
                    );
                    t.translation.x = target.x;
                    t.translation.y = target.y;
                    t.rotation = Quat::from_rotation_z(dq_rot(nb.rot));
                }
            }
            None => {
                if has_new {
                    let rot = dq_rot(nb.rot);
                    let ent = spawn_bullet_entity(
                        &mut commands,
                        &assets.bullet,
                        Vec2::new(dq_pos(nb.x), dq_pos(nb.y)),
                        Vec2::new(rot.cos(), rot.sin()),
                        0.0,
                        0,
                        nb.id,
                        nb.is_rocket,
                        false,
                        None,
                        false,
                        0, // shooter_id — irrelevant on the client side, no auth hit-test runs here.
                    );
                    net_entities.bullets.insert(nb.id, ent);
                }
            }
        }
    }
    if has_new {
        despawn_stale(&mut commands, &mut net_entities.bullets, &scratch.seen_bullets);
    }

    // ── 8. Pickups: spawn-only (static positions) ─────────────────────────
    // Skip the whole section when host signalled "no change" (None).  Without
    // this guard `despawn_stale` would clear every pickup the moment we get
    // a delta snapshot — they aren't in `lifecycle.pickups` because the host
    // omitted the field.  Also skipped when no new snapshot arrived this
    // frame — the field's content can't have changed since the last apply.
    if let Some(snap_pickups) = lifecycle.pickups.as_ref().filter(|_| has_new) {
    scratch.seen_pickups.clear();
    for np in snap_pickups {
        scratch.seen_pickups.insert(np.id);
        use std::collections::hash_map::Entry;
        if let Entry::Vacant(entry) = net_entities.pickups.entry(np.id) {
            let ent = match np.kind {
                HEALTH_PICKUP_KIND => spawn_health_entity(
                    &mut commands,
                    &assets.extra,
                    Vec2::new(dq_pos(np.x), dq_pos(np.y)),
                    np.id,
                ),
                ARMOR_PICKUP_KIND => spawn_armor_entity(
                    &mut commands,
                    &assets.extra,
                    Vec2::new(dq_pos(np.x), dq_pos(np.y)),
                    np.id,
                ),
                MONEY2X_PICKUP_KIND => spawn_money_entity(
                    &mut commands,
                    &assets.extra,
                    Vec2::new(dq_pos(np.x), dq_pos(np.y)),
                    2,
                    np.id,
                ),
                MONEY3X_PICKUP_KIND => spawn_money_entity(
                    &mut commands,
                    &assets.extra,
                    Vec2::new(dq_pos(np.x), dq_pos(np.y)),
                    3,
                    np.id,
                ),
                _ => {
                    let kind = Weapon::from_u8(np.kind);
                    spawn_pickup_entity(
                        &mut commands,
                        &assets.weapon,
                        Vec2::new(dq_pos(np.x), dq_pos(np.y)),
                        kind,
                        np.id,
                    )
                }
            };
            entry.insert(ent);
        }
    }
    despawn_stale(&mut commands, &mut net_entities.pickups, &scratch.seen_pickups);
    } // end "if let Some(snap_pickups)"

    // Everything below only reacts to snapshot *content* — skip it entirely
    // on frames where no new snapshot arrived.
    if !has_new {
        return;
    }

    // ── 9. Explosions: anim state from latest ─────────────────────────────
    scratch.seen_explosions.clear();
    for ne in &lifecycle.explosions {
        scratch.seen_explosions.insert(ne.id);
        match net_entities.explosions.get(&ne.id).copied() {
            Some(ent) => {
                if let Ok((mut exp, mut sprite)) = explosions.get_mut(ent) {
                    let remaining = (ne.remaining_ms as f32) / 1000.0;
                    let radius = dq_radius(ne.radius);
                    exp.lifetime = remaining;
                    exp.radius = radius;
                    let t = (remaining / EXPLOSION_LIFETIME).clamp(0.0, 1.0);
                    let phase = 1.0 - t;
                    let scale = 1.1 + phase * 1.0;
                    sprite.custom_size = Some(Vec2::splat(radius * scale));
                }
            }
            None => {
                let ent = spawn_explosion_entity(
                    &mut commands,
                    &assets.bullet,
                    Vec2::new(dq_pos(ne.x), dq_pos(ne.y)),
                    dq_radius(ne.radius),
                    ne.id,
                );
                net_entities.explosions.insert(ne.id, ent);
            }
        }
    }
    despawn_stale(&mut commands, &mut net_entities.explosions, &scratch.seen_explosions);

    // ── Explodable smolder: mirror the host's per-wreck damage stage so a
    // car the host shows smoking/burning looks the same on our screen.  Any
    // explodable absent from the wire list is intact (stage 0); destroyed
    // ones are handled separately via `destroyed_explodables`.
    let damaged: std::collections::HashMap<u32, u8> =
        lifecycle.damaged_explodables.iter().copied().collect();
    for (idx, mut sm) in explodable_vis.iter_mut() {
        let stage = damaged.get(&(idx.0 as u32)).copied().unwrap_or(0);
        if sm.stage != stage {
            sm.stage = stage;
        }
    }

    if lifecycle.game_over {
        next_state.set(GameState::GameOver);
    }

    history.last_applied_tick = latest_tick;
}

//! Multiplayer protocol and the game-facing connection handles.
//!
//! Everything here is transport-agnostic: `HostConn` / `ClientConn` are just
//! mpsc channels, so `sync.rs`, `lobby.rs` and `chat.rs` never see a socket.
//! The actual transport is WebRTC data channels brokered by a small
//! signaling server — see `net_web.rs` (browser only).  Native builds keep
//! single player and compile the same code; `start_host` / `start_client`
//! simply refuse there.

// The protocol is only spoken by the browser transport, so on native every
// message variant and limit is "never constructed" — that's expected, not rot.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const MAX_PLAYERS: u8 = 4;
/// Snapshot bandwidth ceiling — server→client only.  Big snapshots can hit
/// ~10-20 KB legitimately; 256 KB is comfortably above the worst case while
/// still preventing a malformed length field from triggering a multi-MB alloc.
pub const MAX_MSG_SIZE: usize = 256 * 1024;
/// Tighter cap for client→server messages (just inputs + Hello/Leave) — these
/// don't legitimately exceed a few hundred bytes, so we can be aggressive.
pub const MAX_CLIENT_MSG_SIZE: usize = 4 * 1024;
/// Drop the connection if a client exceeds this many messages per second.
/// Inputs flow at 60 Hz, so 120 leaves comfortable headroom for bursts.
pub const CLIENT_MSG_RATE_LIMIT: u32 = 120;
/// Hard timeout for completing the initial `Hello` handshake.  Peers that
/// open a data channel and never introduce themselves are dropped.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// Room codes as issued by the signaling server (`CODE_LEN` in
/// signaling/src/main.rs — keep in sync).
pub const ROOM_CODE_LEN: usize = 4;
/// Input field cap — a little slack over `ROOM_CODE_LEN` for future formats.
pub const ROOM_CODE_MAX_LEN: usize = 6;
/// Network protocol version — bumped on any wire-format change.  Clients with
/// a mismatched version are rejected at connect time so they don't trigger
/// `bincode::deserialize` panics on a wrong-shape struct.
pub const PROTOCOL_VERSION: u16 = 8;

/// Hard limit on a single chat line.  80 chars is wide enough to be useful
/// without enabling spam.  Enforced on both the client send path and the
/// server relay path so a bad actor can't bypass it.
pub const CHAT_MAX_LEN: usize = 80;

/// Trim, restrict to printable ASCII, cap at `CHAT_MAX_LEN`.  Returns `None`
/// if nothing usable remains so callers can drop empty messages.
pub fn sanitize_chat(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(CHAT_MAX_LEN);
    for c in trimmed.chars() {
        if out.chars().count() >= CHAT_MAX_LEN {
            break;
        }
        // Keep printable ASCII (space + visible glyphs).  Strips control
        // chars / non-ASCII so the renderer (PressStart2P) doesn't draw
        // tofu boxes.
        if c == ' ' || (c.is_ascii_graphic()) {
            out.push(c);
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ClientMsg {
    /// Handshake: client announces itself + the protocol version it speaks.
    /// Server hangs up if the version doesn't match `PROTOCOL_VERSION`.
    Hello {
        nickname: String,
        protocol_version: u16,
    },
    Input(NetInput),
    Chat {
        text: String,
    },
    Leave,
}

pub const NICKNAME_MAX_LEN: usize = 10;
/// Sanitises a free-form nickname: trims whitespace, uppercases, restricts
/// to printable ASCII, caps to NICKNAME_MAX_LEN.  Empty input → "GRACZ".
pub fn sanitize_nickname(input: &str) -> String {
    let mut out = String::with_capacity(NICKNAME_MAX_LEN);
    for c in input.chars() {
        if out.chars().count() >= NICKNAME_MAX_LEN {
            break;
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        }
    }
    if out.is_empty() {
        out.push_str("GRACZ");
    }
    out
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ServerMsg {
    Welcome { your_id: u8, protocol_version: u16 },
    LobbyState { players: Vec<u8> },
    StartGame,
    /// Pre-start countdown — host broadcasts this when the start key is
    /// pressed.  Clients display the remaining seconds and can prepare
    /// themselves; `StartGame` follows when the host's local timer hits 0.
    /// `CountdownCancel` aborts a running countdown.
    CountdownStart { seconds: u8 },
    CountdownCancel,
    Snapshot(Box<NetSnapshot>),
    FullLobby,
    /// Sent when a client's protocol version doesn't match the server.
    ProtocolMismatch { server_version: u16 },
    /// Sent when a client tries to join while a round is already running.
    /// Closes the connection — client should fall back to the menu.
    GameInProgress,
    /// Server-relayed chat line.  Author resolved to a display name on the
    /// host (from `PlayerNicknames` / `LocalNickname`) so receivers don't
    /// need a nickname-table lookup.
    Chat { author: String, text: String },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct NetInput {
    pub move_x: f32,
    pub move_y: f32,
    pub aim_x: f32,
    pub aim_y: f32,
    pub shoot: bool,
    pub throw: bool,
    pub reload: bool,
    pub switch_slot: u8,
    /// True for the single tick when the player presses the interact key (E)
    /// — used by the segment-unlock system to detect a manual purchase.
    pub interact: bool,
    /// True for as long as the interact key is held down — used by the
    /// revive system, which needs hold-progress rather than a one-shot.
    pub interact_held: bool,
    /// Monotonic per-client input sequence number.  The server echoes the
    /// last sequence it applied for that client back in `NetPlayerState`,
    /// which lets the client drop already-processed inputs from its
    /// history buffer and replay only the unacknowledged ones after each
    /// authoritative snapshot.  Wraps after ~135 years at 60 Hz.
    pub seq: u32,
}

impl NetInput {
    /// Strip NaN / infinite floats and clamp magnitudes so a malicious or
    /// buggy client cannot poison the server simulation (`pos += NaN`
    /// permanently breaks the player's transform).  Called on every input
    /// the host receives.  Movement is normalised post-clamp by the
    /// existing per-tick `mv.normalize()` path; aim must stay non-zero so
    /// we leave the previous value untouched if the new one is unusable.
    pub fn sanitize(&mut self) {
        let san = |f: f32| if f.is_finite() { f.clamp(-1.5, 1.5) } else { 0.0 };
        self.move_x = san(self.move_x);
        self.move_y = san(self.move_y);
        // Aim: only overwrite with sanitised values if magnitude is reasonable;
        // otherwise zero it so the server-side fallback (`player.aim` previous
        // value) kicks in.
        let ax = san(self.aim_x);
        let ay = san(self.aim_y);
        if (ax * ax + ay * ay) > 0.0001 {
            self.aim_x = ax;
            self.aim_y = ay;
        } else {
            self.aim_x = 0.0;
            self.aim_y = 0.0;
        }
        // Slot: only 0..=3 are meaningful.
        if self.switch_slot > 3 {
            self.switch_slot = 0;
        }
    }
}

/// Position quantisation factor.  1/8 px precision, range ±4096 px (mapa
/// max half-extent 3840 px ≤ 4096, więc full mapa się mieści w i16 bez
/// utraty informacji nieosiągalnej dla gracza).
const POS_Q: f32 = 8.0;
/// Rotation quantisation: i16 reprezentuje radiany * 10000 (≈0.0001 rad
/// precyzji = ~0.006°).  Wystarczy dla aimu / sprite rotation.
const ROT_Q: f32 = 10000.0;

#[inline] pub fn q_pos(v: f32) -> i16 { (v * POS_Q).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16 }
#[inline] pub fn dq_pos(q: i16) -> f32 { q as f32 / POS_Q }
#[inline] pub fn q_rot(r: f32) -> i16 {
    if !r.is_finite() { return 0; }
    (r * ROT_Q).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
}
#[inline] pub fn dq_rot(q: i16) -> f32 { q as f32 / ROT_Q }
/// Radii (eksplozje, bullety) — quantyzacja taka sama jak pozycji ale unsigned
/// (max 8191 px ≈ więcej niż największa eksplozja w grze).
#[inline] pub fn q_radius(v: f32) -> u16 { (v * POS_Q).round().clamp(0.0, u16::MAX as f32) as u16 }
#[inline] pub fn dq_radius(q: u16) -> f32 { q as f32 / POS_Q }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NetSnapshot {
    pub tick: u64,
    pub players: Vec<NetPlayerState>,
    pub zombies: Vec<NetZombieState>,
    pub bullets: Vec<NetBulletState>,
    /// `None` = pickups didn't change since the previous snapshot — client
    /// keeps its existing entities.  Pickups are static (no movement, just
    /// spawn/despawn) so the host can reliably detect "no change" by hashing
    /// the id-set.  Saves ~14 B × 28 pickups ≈ 400 B per snapshot when stable.
    pub pickups: Option<Vec<NetPickupState>>,
    pub explosions: Vec<NetExplosionState>,
    pub score: u32,
    pub wave: u32,
    pub in_break: bool,
    /// Pozostały czas przerwy w milisekundach — max 65 s (przerwa to ~2.5 s,
    /// więc bardzo dużo zapasu).  Klient odbudowuje z `break_ms / 1000.0`.
    pub break_ms: u16,
    pub zombies_to_spawn: u32,
    pub game_over: bool,
    /// Bitmask: bit `i` set ⇒ map segment with idx `i` is unlocked.
    /// Bit 0 (starting area) is always 1.
    pub unlocked_segments_mask: u8,
    /// `None` = nicknames didn't change.  Population only changes on
    /// connect/disconnect, so empty most of the game.  Saves ~12-40 B per
    /// snapshot once players have introduced themselves.
    pub player_nicknames: Option<Vec<(u8, String)>>,
    /// Map-obstacle indices for explodables that have been destroyed on
    /// the host.  Sent every snapshot (full set, not delta) so a client
    /// joining mid-game catches up immediately.  Tiny on the wire — at
    /// most ~30 entries × 4 B in a long match.
    pub destroyed_explodables: Vec<u32>,
    /// `(obstacle_idx, stage)` for explodables that are damaged but not yet
    /// destroyed: `stage` 1 = smoking, 2 = burning (close to detonation).
    /// Lets remote clients show a wreck smoking then catching fire as it
    /// nears its blast.  Empty until something actually takes a hit, so it
    /// costs nothing on the wire most of the match.
    pub damaged_explodables: Vec<(u32, u8)>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct NetPlayerState {
    pub id: u8,
    pub x: i16,
    pub y: i16,
    pub rot: i16,
    pub hp: i16,
    pub armor: i16,
    pub active_slot: u8,
    pub slot1_weapon: u8, // 255 = None
    /// Last input sequence number the server applied for this client.
    /// Used for input-replay reconciliation on the owning client; remote
    /// clients ignore this field.
    pub last_processed_seq: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct NetZombieState {
    pub id: u32,
    pub x: i16,
    pub y: i16,
    pub rot: i16,
    pub kind: u8,
    /// Current HP — synced so the giant's life bar (and any future
    /// per-zombie HP overlays) read the same value on every client.
    /// Quantised i16 fits the existing per-kind base HP comfortably
    /// (Giant tops out at 1500 in current tuning).
    pub hp: i16,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct NetBulletState {
    pub id: u32,
    pub x: i16,
    pub y: i16,
    pub rot: i16,
    pub is_rocket: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct NetPickupState {
    pub id: u32,
    pub x: i16,
    pub y: i16,
    pub kind: u8,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct NetExplosionState {
    pub id: u32,
    pub x: i16,
    pub y: i16,
    pub radius: u16,
    /// Pozostały czas eksplozji w milisekundach (max 65s — eksplozje żyją
    /// ~0.4 s, więc tylko niewielki ułamek zakresu).
    pub remaining_ms: u16,
}

#[derive(Resource, Default, PartialEq, Eq, Clone, Copy, Debug)]
pub enum NetMode {
    #[default]
    SinglePlayer,
    Host,
    Client,
}

pub enum ServerEvent {
    Connected { id: u8 },
    Hello { id: u8, nickname: String },
    Disconnected { id: u8 },
    Input { id: u8, input: NetInput },
    /// Raw chat submission from a connected client — host resolves the
    /// author nickname before broadcasting `ServerMsg::Chat`.
    ChatRelay { id: u8, text: String },
    /// The signaling server assigned this host a room code — the lobby shows
    /// it so the host can pass it on.  Arrives once, shortly after
    /// `start_host`.
    RoomCode { code: String },
    /// Hosting fell through before the room was usable (signaling server
    /// unreachable, rejected the request, ...).  The lobby bails to the menu.
    HostError { reason: String },
}

pub enum ClientInEvent {
    Welcomed { your_id: u8 },
    LobbyState { players: Vec<u8> },
    Started,
    /// Host started the pre-game countdown.  Client should mirror it for
    /// visual feedback; the actual game start arrives as `Started`.
    CountdownStart { seconds: u8 },
    CountdownCancel,
    Snapshot(Box<NetSnapshot>),
    Disconnected,
    FullLobby,
    /// Host rejected the connection because a round is already running.
    GameInProgress,
    ProtocolMismatch {
        #[allow(dead_code)] // Surfaced for future UI display of mismatch
        server_version: u16,
    },
    /// Chat line broadcast from the host.
    Chat { author: String, text: String },
    /// The connection attempt never reached `Welcomed` — bad room code, room
    /// full, host gone, WebRTC failed.  Carries a short player-facing reason.
    ConnectFailed { reason: String },
}

/// What the host queues on a client's outgoing channel.  `Msg` is a normal
/// message serialized per-client when the transport drains the queue
/// (lobby/chat/control — low frequency).  `Frame` is pre-serialized bytes
/// shared (via `Arc`) across all clients: the 60 Hz snapshot is identical for
/// everyone, so it is encoded once and every client's queue gets a cheap
/// `Arc` clone instead of a deep clone + re-serialization.
pub enum OutMsg {
    Msg(ServerMsg),
    Frame(Arc<[u8]>),
}

pub struct HostConn {
    pub events: Arc<Mutex<Receiver<ServerEvent>>>,
    pub senders: Arc<Mutex<HashMap<u8, Sender<OutMsg>>>>,
    /// Set when the round has actually started (GameState::Playing).  The
    /// transport reads this on each handshake to reject mid-game joiners
    /// with `ServerMsg::GameInProgress` instead of dropping them into a live
    /// simulation that won't spawn a player entity for them.
    pub in_game: Arc<AtomicBool>,
    /// Identifies the transport session backing this handle, so dropping a
    /// stale handle can't tear down a newer session (see `net_web::teardown`).
    pub session: u64,
}

impl Drop for HostConn {
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        crate::net_web::teardown(self.session);
    }
}

pub struct ClientConn {
    pub events: Arc<Mutex<Receiver<ClientInEvent>>>,
    pub sender: Sender<ClientMsg>,
    pub session: u64,
}

impl Drop for ClientConn {
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        crate::net_web::teardown(self.session);
    }
}

#[derive(Resource, Default)]
pub struct NetContext {
    pub host: Option<HostConn>,
    pub client: Option<ClientConn>,
    pub my_id: u8,
    pub lobby_players: Vec<u8>,
    /// Room code of the current session (host: assigned by signaling;
    /// client: the one typed in).  Empty until known.
    pub room_code: String,
    pub next_zombie_net_id: u32,
    pub next_bullet_net_id: u32,
    pub next_pickup_net_id: u32,
    pub next_explosion_net_id: u32,
}

impl NetContext {
    pub fn alloc_zombie_id(&mut self) -> u32 {
        self.next_zombie_net_id = self.next_zombie_net_id.wrapping_add(1);
        self.next_zombie_net_id
    }
    pub fn alloc_bullet_id(&mut self) -> u32 {
        self.next_bullet_net_id = self.next_bullet_net_id.wrapping_add(1);
        self.next_bullet_net_id
    }
    pub fn alloc_pickup_id(&mut self) -> u32 {
        self.next_pickup_net_id = self.next_pickup_net_id.wrapping_add(1);
        self.next_pickup_net_id
    }
    pub fn alloc_explosion_id(&mut self) -> u32 {
        self.next_explosion_net_id = self.next_explosion_net_id.wrapping_add(1);
        self.next_explosion_net_id
    }
    pub fn reset_alloc(&mut self) {
        self.next_zombie_net_id = 0;
        self.next_bullet_net_id = 0;
        self.next_pickup_net_id = 0;
        self.next_explosion_net_id = 0;
    }
    pub fn disconnect(&mut self) {
        self.host = None;
        self.client = None;
        self.lobby_players.clear();
        self.my_id = 0;
        self.room_code.clear();
        self.reset_alloc();
    }
}

#[derive(Resource, Default)]
pub struct LocalInput(pub NetInput);

#[derive(Resource, Default)]
pub struct RemoteInputs(pub HashMap<u8, NetInput>);

#[derive(Resource)]
pub struct LocalNickname(pub String);

impl Default for LocalNickname {
    fn default() -> Self {
        Self("GRACZ".to_string())
    }
}

/// Map of `player_id → nickname`.  Server populates from `Hello` messages
/// (and writes its own from `LocalNickname`); client populates from
/// `NetSnapshot.player_nicknames`.
#[derive(Resource, Default)]
pub struct PlayerNicknames(pub HashMap<u8, String>);

#[derive(Resource, Default)]
pub struct NetEntities {
    pub players: HashMap<u8, Entity>,
    pub zombies: HashMap<u32, Entity>,
    pub bullets: HashMap<u32, Entity>,
    pub pickups: HashMap<u32, Entity>,
    pub explosions: HashMap<u32, Entity>,
}

impl NetEntities {
    pub fn clear(&mut self) {
        self.players.clear();
        self.zombies.clear();
        self.bullets.clear();
        self.pickups.clear();
        self.explosions.clear();
    }
}

#[derive(Component)]
pub struct NetId(pub u32);

/// Open a room and start accepting joiners.  Returns immediately; the room
/// code arrives as `ServerEvent::RoomCode` (or `HostError` if hosting fell
/// through).  Browser only — see `net_web::start_host`.
pub fn start_host() -> Result<HostConn, String> {
    #[cfg(target_arch = "wasm32")]
    {
        crate::net_web::start_host()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err("MULTIPLAYER IS WEB-ONLY".to_string())
    }
}

/// Join the room `code`.  Returns immediately; progress arrives as
/// `ClientInEvent::Welcomed` / `ConnectFailed`.  Browser only — see
/// `net_web::start_client`.
pub fn start_client(code: &str, nickname: &str) -> Result<ClientConn, String> {
    #[cfg(target_arch = "wasm32")]
    {
        crate::net_web::start_client(code, nickname)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (code, nickname);
        Err("MULTIPLAYER IS WEB-ONLY".to_string())
    }
}

/// Decode one wire message, refusing anything over `limit` bytes before
/// touching bincode — a hostile peer must not be able to make the host
/// allocate a multi-MB buffer or feed garbage into a deserializer panic.
/// Data channels are message-framed, so no length prefix is involved.
pub fn decode_limited<T: for<'de> Deserialize<'de>>(bytes: &[u8], limit: usize) -> Result<T, String> {
    if bytes.is_empty() {
        return Err("empty message".into());
    }
    if bytes.len() > limit {
        return Err(format!("message of {} bytes exceeds limit {limit}", bytes.len()));
    }
    bincode::deserialize(bytes).map_err(|e| e.to_string())
}

pub fn broadcast(host: &HostConn, msg: &ServerMsg) {
    // `StartGame` on the wire means the round is live *now* — flip the
    // in-game flag before iterating senders so a joiner whose handshake
    // completes between this broadcast and `OnEnter(Playing)` (which runs
    // `set_host_in_game` a frame-plus later) is rejected with
    // `GameInProgress` instead of being welcomed into a lobby that will
    // never send it `StartGame` again.
    if matches!(msg, ServerMsg::StartGame) {
        host.in_game.store(true, Ordering::Relaxed);
    }
    let senders = host.senders.lock().unwrap_or_else(|e| e.into_inner());
    for tx in senders.values() {
        let _ = tx.send(OutMsg::Msg(msg.clone()));
    }
}

/// Send a message to a single client's writer queue — the targeted
/// counterpart of `broadcast`.  No-ops if the client already disconnected
/// (its sender was removed from the map, or the writer thread hung up).
pub fn send_to(host: &HostConn, id: u8, msg: &ServerMsg) {
    let senders = host.senders.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tx) = senders.get(&id) {
        let _ = tx.send(OutMsg::Msg(msg.clone()));
    }
}

/// Serialize a message once into its on-wire bytes (plain bincode — data
/// channels frame messages themselves) so it can be broadcast to every
/// client without re-serializing per connection.  Returns `None` if
/// serialization fails (the caller simply skips the tick).
pub fn encode_frame<T: Serialize>(msg: &T) -> Option<Arc<[u8]>> {
    bincode::serialize(msg).ok().map(Into::into)
}

/// Broadcast a pre-encoded frame (see `encode_frame`) to all clients.  Each
/// client's writer gets a cheap `Arc` clone — no deep clone, no re-serialize.
pub fn broadcast_frame(host: &HostConn, frame: &Arc<[u8]>) {
    let senders = host.senders.lock().unwrap_or_else(|e| e.into_inner());
    for tx in senders.values() {
        let _ = tx.send(OutMsg::Frame(frame.clone()));
    }
}

pub fn is_authoritative(net: Res<NetMode>) -> bool {
    !matches!(*net, NetMode::Client)
}

pub fn is_host(net: Res<NetMode>) -> bool {
    matches!(*net, NetMode::Host)
}

pub fn is_net_client(net: Res<NetMode>) -> bool {
    matches!(*net, NetMode::Client)
}

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetMode>()
            .init_resource::<NetContext>()
            .init_resource::<LocalInput>()
            .init_resource::<RemoteInputs>()
            .init_resource::<NetEntities>()
            .init_resource::<LocalNickname>()
            .init_resource::<PlayerNicknames>()
            .add_systems(OnEnter(crate::GameState::Playing), set_host_in_game)
            .add_systems(OnExit(crate::GameState::Playing), clear_host_in_game);
        // Flush everything queued on the connection handles this frame into
        // the browser's data channels — runs last so the FixedUpdate snapshot
        // broadcast leaves in the same frame it was produced.
        #[cfg(target_arch = "wasm32")]
        app.add_systems(Last, crate::net_web::pump);
    }
}

/// Mark the host session as "in game" so further joiners are rejected with
/// `ServerMsg::GameInProgress`.
fn set_host_in_game(ctx: Res<NetContext>) {
    if let Some(host) = ctx.host.as_ref() {
        host.in_game.store(true, Ordering::Relaxed);
    }
}

/// Clear the in-game flag when the round ends so the lobby can accept new
/// joiners again on the next session.
fn clear_host_in_game(ctx: Res<NetContext>) {
    if let Some(host) = ctx.host.as_ref() {
        host.in_game.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_nickname_clamps_and_uppercases() {
        assert_eq!(sanitize_nickname("alice"), "ALICE");
        assert_eq!(sanitize_nickname("  bob "), "BOB");
        assert_eq!(sanitize_nickname(""), "GRACZ");
        // Strips non-alphanumeric.
        assert_eq!(sanitize_nickname("hi!@#"), "HI");
        // Truncates to NICKNAME_MAX_LEN (10) characters.
        assert_eq!(sanitize_nickname("abcdefghijklmnop"), "ABCDEFGHIJ");
        assert_eq!(sanitize_nickname("abcdefghijklmnop").chars().count(), NICKNAME_MAX_LEN);
    }

    #[test]
    fn netinput_sanitize_rejects_nan_and_inf() {
        // Movement: NaN/Inf → 0.  Aim: NaN component zeroed; if the
        // remaining magnitude is non-trivial we keep the partial vector.
        let mut i = NetInput {
            move_x: f32::NAN,
            move_y: f32::INFINITY,
            aim_x: f32::NEG_INFINITY,
            aim_y: 0.5,
            switch_slot: 99,
            ..Default::default()
        };
        i.sanitize();
        assert_eq!(i.move_x, 0.0);
        assert_eq!(i.move_y, 0.0);
        // aim_x sanitised to 0; aim_y survives at 0.5 (magnitude 0.25 > eps).
        assert_eq!(i.aim_x, 0.0);
        assert_eq!(i.aim_y, 0.5);
        assert_eq!(i.switch_slot, 0); // out-of-range slot reset.
    }

    #[test]
    fn netinput_sanitize_zeros_aim_when_both_axes_invalid() {
        // Both aim axes garbage ⇒ magnitude < eps ⇒ both zeroed.
        let mut i = NetInput {
            aim_x: f32::NAN,
            aim_y: f32::INFINITY,
            ..Default::default()
        };
        i.sanitize();
        assert_eq!(i.aim_x, 0.0);
        assert_eq!(i.aim_y, 0.0);
    }

    #[test]
    fn netinput_sanitize_clamps_oversized_movement() {
        let mut i = NetInput {
            move_x: 5.0,
            move_y: -10.0,
            aim_x: 0.7,
            aim_y: 0.7,
            switch_slot: 2,
            ..Default::default()
        };
        i.sanitize();
        assert!((i.move_x - 1.5).abs() < 1e-6, "move_x should clamp to 1.5");
        assert!((i.move_y - -1.5).abs() < 1e-6, "move_y should clamp to -1.5");
        assert!((i.aim_x - 0.7).abs() < 1e-6);
        assert!((i.aim_y - 0.7).abs() < 1e-6);
        assert_eq!(i.switch_slot, 2);
    }

    #[test]
    fn quantization_round_trips_to_within_one_eighth_pixel() {
        for &v in &[-3840.0, -1234.5, 0.0, 0.4, 1234.5, 3840.0] {
            let q = q_pos(v);
            let dq = dq_pos(q);
            assert!((dq - v).abs() <= 1.0 / POS_Q + 1e-3,
                "value {v} round-tripped to {dq}");
        }
    }

    #[test]
    fn rotation_quantization_handles_full_circle() {
        use std::f32::consts::PI;
        for &v in &[-PI, -1.0, 0.0, 1.0, PI] {
            let q = q_rot(v);
            let dq = dq_rot(q);
            assert!((dq - v).abs() < 1e-3, "rot {v} round-tripped to {dq}");
        }
        // NaN should be silently zeroed (not panic, not propagate).
        assert_eq!(q_rot(f32::NAN), 0);
    }

    #[test]
    fn radius_quantization_clamps_negatives_to_zero() {
        assert_eq!(q_radius(-5.0), 0);
        assert_eq!(dq_radius(q_radius(50.0)), 50.0);
    }

    // ── sanitize_chat ─────────────────────────────────────────────────
    // The chat trust boundary: enforced on the local send path (chat.rs)
    // and on the host relay path for remote clients, so these invariants
    // are what stands between a hostile LAN client and every player's
    // chat overlay.

    #[test]
    fn sanitize_chat_rejects_empty_and_unprintable_input() {
        assert_eq!(sanitize_chat(""), None);
        assert_eq!(sanitize_chat("   "), None);
        assert_eq!(sanitize_chat("\n\t"), None);
        // Control chars are stripped; the surviving interior space alone
        // must still be dropped by the second `out.trim()` check.
        assert_eq!(sanitize_chat("\x01\x02 \x03"), None);
        assert_eq!(sanitize_chat("\t\x01 \x02"), None);
    }

    #[test]
    fn sanitize_chat_strips_non_ascii_and_control_chars() {
        // Non-ASCII (accents, emoji) would render as tofu boxes in
        // PressStart2P — stripped, while interior spaces survive.
        assert_eq!(sanitize_chat("héllo wörld").as_deref(), Some("hllo wrld"));
        assert_eq!(sanitize_chat("abc\u{1F44D}def").as_deref(), Some("abcdef"));
        assert_eq!(sanitize_chat("a\x01b\nc").as_deref(), Some("abc"));
    }

    #[test]
    fn sanitize_chat_trims_whitespace_and_caps_length() {
        assert_eq!(sanitize_chat("  hi  ").as_deref(), Some("hi"));
        let long = "a".repeat(200);
        assert_eq!(sanitize_chat(&long).unwrap().chars().count(), CHAT_MAX_LEN);
        // Output invariant regardless of input: at most CHAT_MAX_LEN chars,
        // every one of them space or printable ASCII.
        let hostile = format!("x{}y\x07z", "\u{1F9DF}".repeat(100));
        let out = sanitize_chat(&hostile).unwrap();
        assert!(out.chars().count() <= CHAT_MAX_LEN);
        assert!(out.chars().all(|c| c == ' ' || c.is_ascii_graphic()));
    }

    // ── wire format ───────────────────────────────────────────────────

    /// Serialize → deserialize → re-serialize and require byte equality.
    /// Bincode is positional, so this proves full field equality without
    /// needing PartialEq on the wire structs: any dropped, reordered or
    /// defaulted field changes the second serialization.
    fn assert_bincode_roundtrip<T: Serialize + for<'de> Deserialize<'de>>(msg: &T) {
        let bytes = bincode::serialize(msg).expect("serialize");
        let back: T = bincode::deserialize(&bytes).expect("deserialize");
        let bytes2 = bincode::serialize(&back).expect("re-serialize");
        assert_eq!(bytes, bytes2, "roundtrip changed the wire bytes");
    }

    fn sample_input() -> NetInput {
        NetInput {
            move_x: -0.5,
            move_y: 1.0,
            aim_x: 0.25,
            aim_y: -0.75,
            shoot: true,
            throw: false,
            reload: true,
            switch_slot: 2,
            interact: true,
            interact_held: false,
            seq: 123_456,
        }
    }

    /// Snapshot with every field populated (incl. both `Some` states of the
    /// optional delta fields) so the roundtrip exercises the whole struct.
    fn sample_snapshot() -> NetSnapshot {
        NetSnapshot {
            tick: 987_654,
            players: vec![NetPlayerState {
                id: 1,
                x: q_pos(-1234.5),
                y: q_pos(321.0),
                rot: q_rot(1.5),
                hp: 87,
                armor: 25,
                active_slot: 1,
                slot1_weapon: 255,
                last_processed_seq: 42,
            }],
            zombies: vec![NetZombieState {
                id: 7,
                x: q_pos(100.0),
                y: q_pos(-3000.0),
                rot: q_rot(-0.5),
                kind: 3,
                hp: 1500,
            }],
            bullets: vec![NetBulletState {
                id: 9,
                x: 5,
                y: -5,
                rot: 0,
                is_rocket: true,
            }],
            pickups: Some(vec![NetPickupState { id: 4, x: 1, y: 2, kind: 6 }]),
            explosions: vec![NetExplosionState {
                id: 11,
                x: 0,
                y: 0,
                radius: q_radius(96.0),
                remaining_ms: 400,
            }],
            score: 31_337,
            wave: 12,
            in_break: true,
            break_ms: 2500,
            zombies_to_spawn: 44,
            game_over: false,
            unlocked_segments_mask: 0b101,
            player_nicknames: Some(vec![(0, "GRACZ".into()), (1, "ALICE".into())]),
            destroyed_explodables: vec![3, 17],
            damaged_explodables: vec![(5, 1), (6, 2)],
        }
    }

    #[test]
    fn every_client_msg_variant_survives_bincode_roundtrip() {
        let msgs = [
            ClientMsg::Hello {
                nickname: "ALICE".into(),
                protocol_version: PROTOCOL_VERSION,
            },
            ClientMsg::Input(sample_input()),
            ClientMsg::Chat { text: "hello there".into() },
            ClientMsg::Leave,
        ];
        for msg in &msgs {
            assert_bincode_roundtrip(msg);
        }
        // Spot-check field-level equality on the payload-carrying variants.
        let bytes = bincode::serialize(&msgs[0]).unwrap();
        match bincode::deserialize::<ClientMsg>(&bytes).unwrap() {
            ClientMsg::Hello { nickname, protocol_version } => {
                assert_eq!(nickname, "ALICE");
                assert_eq!(protocol_version, PROTOCOL_VERSION);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        let bytes = bincode::serialize(&msgs[1]).unwrap();
        match bincode::deserialize::<ClientMsg>(&bytes).unwrap() {
            ClientMsg::Input(i) => {
                assert_eq!(i.move_x, -0.5);
                assert_eq!(i.aim_y, -0.75);
                assert!(i.shoot && i.reload && i.interact);
                assert!(!i.throw && !i.interact_held);
                assert_eq!(i.switch_slot, 2);
                assert_eq!(i.seq, 123_456);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn every_server_msg_variant_survives_bincode_roundtrip() {
        let msgs = [
            ServerMsg::Welcome { your_id: 2, protocol_version: PROTOCOL_VERSION },
            ServerMsg::LobbyState { players: vec![0, 1, 2] },
            ServerMsg::StartGame,
            ServerMsg::CountdownStart { seconds: 3 },
            ServerMsg::CountdownCancel,
            ServerMsg::Snapshot(Box::new(sample_snapshot())),
            ServerMsg::FullLobby,
            ServerMsg::ProtocolMismatch { server_version: PROTOCOL_VERSION },
            ServerMsg::GameInProgress,
            ServerMsg::Chat { author: "BOB".into(), text: "hi".into() },
        ];
        for msg in &msgs {
            assert_bincode_roundtrip(msg);
        }
        // Field-level equality on the populated snapshot.
        let bytes = bincode::serialize(&msgs[5]).unwrap();
        match bincode::deserialize::<ServerMsg>(&bytes).unwrap() {
            ServerMsg::Snapshot(s) => {
                assert_eq!(s.tick, 987_654);
                assert_eq!(s.players.len(), 1);
                assert_eq!(s.players[0].x, q_pos(-1234.5));
                assert_eq!(s.players[0].last_processed_seq, 42);
                assert_eq!(s.zombies[0].hp, 1500);
                assert!(s.bullets[0].is_rocket);
                assert_eq!(s.pickups.as_ref().unwrap()[0].kind, 6);
                assert_eq!(s.explosions[0].radius, q_radius(96.0));
                assert_eq!(s.unlocked_segments_mask, 0b101);
                assert_eq!(
                    s.player_nicknames.as_ref().unwrap()[1],
                    (1, "ALICE".to_string())
                );
                assert_eq!(s.destroyed_explodables, vec![3, 17]);
                assert_eq!(s.damaged_explodables, vec![(5, 1), (6, 2)]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    /// Protocol-drift canary tied to PROTOCOL_VERSION (currently 8): the
    /// bincode fixint encoding of NetInput is 4×f32 + 5×bool + u8 + u32 =
    /// 26 bytes.  If this assert fires you changed the wire format — bump
    /// PROTOCOL_VERSION and update this constant in the same commit.
    #[test]
    fn netinput_wire_size_is_a_protocol_canary() {
        let bytes = bincode::serialize(&NetInput::default()).unwrap();
        assert_eq!(
            bytes.len(),
            26,
            "NetInput wire size changed — bump PROTOCOL_VERSION"
        );
    }

    // ── wire framing (encode_frame / decode_limited) ──────────────────

    #[test]
    fn encode_frame_is_plain_bincode_and_decodes_back() {
        let msg = ServerMsg::Snapshot(Box::new(sample_snapshot()));
        let frame = encode_frame(&msg).expect("encode");
        assert_eq!(&frame[..], &bincode::serialize(&msg).unwrap()[..]);
        let back: ServerMsg = decode_limited(&frame, MAX_MSG_SIZE).expect("decode");
        match back {
            ServerMsg::Snapshot(s) => assert_eq!(s.tick, 987_654),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn decode_limited_rejects_empty_oversized_and_garbage() {
        assert!(decode_limited::<ClientMsg>(&[], MAX_CLIENT_MSG_SIZE).is_err());
        let big = vec![0u8; MAX_CLIENT_MSG_SIZE + 1];
        assert!(decode_limited::<ClientMsg>(&big, MAX_CLIENT_MSG_SIZE).is_err());
        // Garbage that fits the size cap must surface as a decode error, not
        // a panic — the host feeds untrusted peer bytes straight in here.
        assert!(decode_limited::<ClientMsg>(&[0xff; 32], MAX_CLIENT_MSG_SIZE).is_err());
        // An input frame comfortably fits the client cap.
        let ok = bincode::serialize(&ClientMsg::Input(sample_input())).unwrap();
        assert!(ok.len() < MAX_CLIENT_MSG_SIZE);
        assert!(decode_limited::<ClientMsg>(&ok, MAX_CLIENT_MSG_SIZE).is_ok());
    }
}

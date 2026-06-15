use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const NET_PORT: u16 = 7777;
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
/// Hard timeout for completing the initial `Hello` handshake.  Connections
/// that just hold the socket open without sending anything are dropped.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// Network protocol version — bumped on any wire-format change.  Clients with
/// a mismatched version are rejected at connect time so they don't trigger
/// `bincode::deserialize` panics on a wrong-shape struct.
pub const PROTOCOL_VERSION: u16 = 7;

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
}

/// What the host queues onto a client's writer channel.  `Msg` is a normal
/// message the writer thread serializes per-client (lobby/chat/control —
/// low frequency).  `Frame` is a pre-serialized, length-prefixed byte buffer
/// shared (via `Arc`) across all clients: the 60 Hz snapshot is identical for
/// everyone, so we serialize it once and hand each writer a cheap `Arc` clone
/// instead of deep-cloning + re-serializing the whole snapshot per client.
pub enum OutMsg {
    Msg(ServerMsg),
    Frame(Arc<[u8]>),
}

pub struct HostConn {
    pub events: Arc<Mutex<Receiver<ServerEvent>>>,
    pub senders: Arc<Mutex<HashMap<u8, Sender<OutMsg>>>>,
    pub shutdown: Arc<AtomicBool>,
    /// Set when the round has actually started (GameState::Playing).  The
    /// listener thread reads this on each accept to reject mid-game joiners
    /// with `ServerMsg::GameInProgress` instead of dropping them into a live
    /// simulation that won't spawn a player entity for them.
    pub in_game: Arc<AtomicBool>,
}

impl Drop for HostConn {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

pub struct ClientConn {
    pub events: Arc<Mutex<Receiver<ClientInEvent>>>,
    pub sender: Sender<ClientMsg>,
}

#[derive(Resource, Default)]
pub struct NetContext {
    pub host: Option<HostConn>,
    pub client: Option<ClientConn>,
    pub my_id: u8,
    pub lobby_players: Vec<u8>,
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

fn write_msg<T: Serialize>(stream: &mut TcpStream, msg: &T) -> std::io::Result<()> {
    let bytes = bincode::serialize(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}

fn read_msg<T: for<'de> Deserialize<'de>>(stream: &mut TcpStream) -> std::io::Result<T> {
    read_msg_with_limit(stream, MAX_MSG_SIZE)
}

fn read_msg_with_limit<T: for<'de> Deserialize<'de>>(
    stream: &mut TcpStream,
    max_len: usize,
) -> std::io::Result<T> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > max_len {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad length",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    bincode::deserialize::<T>(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

pub fn start_host() -> std::io::Result<HostConn> {
    let listener = TcpListener::bind(("0.0.0.0", NET_PORT))?;
    listener.set_nonblocking(true)?;

    let (event_tx, event_rx) = channel::<ServerEvent>();
    let senders: Arc<Mutex<HashMap<u8, Sender<OutMsg>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let shutdown = Arc::new(AtomicBool::new(false));
    let in_game = Arc::new(AtomicBool::new(false));

    let senders_clone = senders.clone();
    let event_tx_clone = event_tx.clone();
    let shutdown_clone = shutdown.clone();
    let in_game_clone = in_game.clone();
    thread::spawn(move || {
        loop {
            if shutdown_clone.load(Ordering::Relaxed) {
                break;
            }
            let (stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(_) => break,
            };
            if stream.set_nonblocking(false).is_err() {
                continue;
            }
            let _ = stream.set_nodelay(true);

            let current_count = senders_clone.lock().unwrap_or_else(|e| e.into_inner()).len();
            if current_count >= (MAX_PLAYERS - 1) as usize {
                let mut s = stream;
                let _ = write_msg(&mut s, &ServerMsg::FullLobby);
                let _ = s.shutdown(std::net::Shutdown::Both);
                continue;
            }

            // Handshake: require Hello with matching protocol version
            // before allocating an id / spawning reader+writer threads.
            // Bound by HELLO_TIMEOUT so a client that opens the socket and
            // never sends anything can't squat resources.
            let mut handshake_stream = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            };
            let _ = handshake_stream.set_read_timeout(Some(HELLO_TIMEOUT));
            let hello = match read_msg_with_limit::<ClientMsg>(
                &mut handshake_stream,
                MAX_CLIENT_MSG_SIZE,
            ) {
                Ok(ClientMsg::Hello { nickname, protocol_version }) => {
                    if protocol_version != PROTOCOL_VERSION {
                        let _ = write_msg(
                            &mut handshake_stream,
                            &ServerMsg::ProtocolMismatch {
                                server_version: PROTOCOL_VERSION,
                            },
                        );
                        let _ = handshake_stream.shutdown(std::net::Shutdown::Both);
                        continue;
                    }
                    sanitize_nickname(&nickname)
                }
                _ => {
                    // Wrong message kind, malformed payload, or timeout —
                    // drop without leaking an id.
                    let _ = handshake_stream.shutdown(std::net::Shutdown::Both);
                    continue;
                }
            };
            // Restore blocking, no-timeout reads for the steady-state stream.
            let _ = handshake_stream.set_read_timeout(None);

            // Reject mid-round joiners cleanly.  Without this the host would
            // accept the connection but never spawn a Player entity for them
            // (sync.rs::server_receive_inputs only logs `Connected` mid-game),
            // leaving the client wedged in a broken state.
            if in_game_clone.load(Ordering::Relaxed) {
                let _ = write_msg(&mut handshake_stream, &ServerMsg::GameInProgress);
                let _ = handshake_stream.shutdown(std::net::Shutdown::Both);
                continue;
            }

            // Allocate the lowest free id in 1..u8::MAX that isn't already
            // in `senders`.  Skipping in-use ids removes the wraparound
            // collision risk of a monotonic counter (after 255 rejoins it
            // would land on 0 = host, then start fighting with live clients).
            // Host id is always 0, never appears in `senders`.
            let id = {
                let lock = senders_clone.lock().unwrap_or_else(|e| e.into_inner());
                let mut candidate: u8 = 1;
                while lock.contains_key(&candidate) {
                    if candidate == u8::MAX {
                        break;
                    }
                    candidate += 1;
                }
                candidate
            };

            let mut welcome_stream = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            };
            if write_msg(
                &mut welcome_stream,
                &ServerMsg::Welcome {
                    your_id: id,
                    protocol_version: PROTOCOL_VERSION,
                },
            )
            .is_err()
            {
                continue;
            }

            let (out_tx, out_rx) = channel::<OutMsg>();
            senders_clone.lock().unwrap_or_else(|e| e.into_inner()).insert(id, out_tx);
            let _ = event_tx_clone.send(ServerEvent::Connected { id });
            let _ = event_tx_clone.send(ServerEvent::Hello {
                id,
                nickname: hello,
            });

            let mut writer_stream = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            };
            thread::spawn(move || {
                while let Ok(out) = out_rx.recv() {
                    let res = match out {
                        // Pre-framed snapshot bytes already carry the 4-byte
                        // big-endian length prefix, so write them verbatim.
                        OutMsg::Frame(frame) => writer_stream.write_all(&frame),
                        OutMsg::Msg(msg) => write_msg(&mut writer_stream, &msg),
                    };
                    if res.is_err() {
                        break;
                    }
                }
                let _ = writer_stream.shutdown(std::net::Shutdown::Both);
            });

            let reader_event_tx = event_tx_clone.clone();
            let reader_senders = senders_clone.clone();
            let mut reader_stream = stream;
            thread::spawn(move || {
                use std::time::Instant;
                let mut window_start = Instant::now();
                let mut window_count: u32 = 0;
                loop {
                    // Rate-limit: if a client sends > CLIENT_MSG_RATE_LIMIT
                    // messages in any rolling 1-second window, drop them.
                    let now = Instant::now();
                    if now.duration_since(window_start) >= Duration::from_secs(1) {
                        window_start = now;
                        window_count = 0;
                    }
                    match read_msg_with_limit::<ClientMsg>(
                        &mut reader_stream,
                        MAX_CLIENT_MSG_SIZE,
                    ) {
                        Ok(msg) => {
                            window_count += 1;
                            if window_count > CLIENT_MSG_RATE_LIMIT {
                                // Spammy client — terminate.
                                reader_senders
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&id);
                                let _ = reader_event_tx
                                    .send(ServerEvent::Disconnected { id });
                                break;
                            }
                            match msg {
                                ClientMsg::Input(input) => {
                                    if reader_event_tx
                                        .send(ServerEvent::Input { id, input })
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                                ClientMsg::Hello { nickname, .. } => {
                                    // Late re-Hello (client renamed itself) —
                                    // accept the new nickname; protocol version
                                    // is already validated.
                                    let clean = sanitize_nickname(&nickname);
                                    let _ = reader_event_tx
                                        .send(ServerEvent::Hello { id, nickname: clean });
                                }
                                ClientMsg::Chat { text } => {
                                    if let Some(clean) = sanitize_chat(&text) {
                                        if reader_event_tx
                                            .send(ServerEvent::ChatRelay { id, text: clean })
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
                                ClientMsg::Leave => {
                                    reader_senders
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .remove(&id);
                                    let _ = reader_event_tx
                                        .send(ServerEvent::Disconnected { id });
                                    break;
                                }
                            }
                        }
                        Err(_) => {
                            reader_senders
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&id);
                            let _ = reader_event_tx.send(ServerEvent::Disconnected { id });
                            break;
                        }
                    }
                }
            });
        }
    });

    Ok(HostConn {
        events: Arc::new(Mutex::new(event_rx)),
        senders,
        shutdown,
        in_game,
    })
}

pub fn start_client(addr: SocketAddr, nickname: &str) -> std::io::Result<ClientConn> {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_nodelay(true)?;

    let (event_tx, event_rx) = channel::<ClientInEvent>();
    let (send_tx, send_rx) = channel::<ClientMsg>();

    let mut hello_stream = stream.try_clone()?;
    let clean_nick = sanitize_nickname(nickname);
    write_msg(
        &mut hello_stream,
        &ClientMsg::Hello {
            nickname: clean_nick,
            protocol_version: PROTOCOL_VERSION,
        },
    )?;

    let mut writer_stream = stream.try_clone()?;
    thread::spawn(move || {
        while let Ok(msg) = send_rx.recv() {
            if write_msg(&mut writer_stream, &msg).is_err() {
                break;
            }
        }
        let _ = writer_stream.shutdown(std::net::Shutdown::Both);
    });

    let reader_event_tx = event_tx.clone();
    let mut reader_stream = stream;
    thread::spawn(move || loop {
        match read_msg::<ServerMsg>(&mut reader_stream) {
            Ok(ServerMsg::Welcome { your_id, protocol_version }) => {
                if protocol_version != PROTOCOL_VERSION {
                    // Server speaks a different protocol than we expected.
                    let _ = reader_event_tx.send(ClientInEvent::ProtocolMismatch {
                        server_version: protocol_version,
                    });
                    break;
                }
                if reader_event_tx
                    .send(ClientInEvent::Welcomed { your_id })
                    .is_err()
                {
                    break;
                }
            }
            Ok(ServerMsg::LobbyState { players }) => {
                if reader_event_tx
                    .send(ClientInEvent::LobbyState { players })
                    .is_err()
                {
                    break;
                }
            }
            Ok(ServerMsg::StartGame) => {
                if reader_event_tx.send(ClientInEvent::Started).is_err() {
                    break;
                }
            }
            Ok(ServerMsg::CountdownStart { seconds }) => {
                if reader_event_tx
                    .send(ClientInEvent::CountdownStart { seconds })
                    .is_err()
                {
                    break;
                }
            }
            Ok(ServerMsg::CountdownCancel) => {
                if reader_event_tx
                    .send(ClientInEvent::CountdownCancel)
                    .is_err()
                {
                    break;
                }
            }
            Ok(ServerMsg::Snapshot(snap)) => {
                if reader_event_tx.send(ClientInEvent::Snapshot(snap)).is_err() {
                    break;
                }
            }
            Ok(ServerMsg::FullLobby) => {
                let _ = reader_event_tx.send(ClientInEvent::FullLobby);
                break;
            }
            Ok(ServerMsg::GameInProgress) => {
                let _ = reader_event_tx.send(ClientInEvent::GameInProgress);
                break;
            }
            Ok(ServerMsg::ProtocolMismatch { server_version }) => {
                let _ = reader_event_tx.send(ClientInEvent::ProtocolMismatch {
                    server_version,
                });
                break;
            }
            Ok(ServerMsg::Chat { author, text }) => {
                if reader_event_tx
                    .send(ClientInEvent::Chat { author, text })
                    .is_err()
                {
                    break;
                }
            }
            Err(_) => {
                let _ = reader_event_tx.send(ClientInEvent::Disconnected);
                break;
            }
        }
    });

    Ok(ClientConn {
        events: Arc::new(Mutex::new(event_rx)),
        sender: send_tx,
    })
}

pub fn broadcast(host: &HostConn, msg: &ServerMsg) {
    let senders = host.senders.lock().unwrap_or_else(|e| e.into_inner());
    for tx in senders.values() {
        let _ = tx.send(OutMsg::Msg(msg.clone()));
    }
}

/// Serialize a message once into its on-wire frame (4-byte big-endian length
/// prefix + bincode body — byte-identical to what `write_msg` produces) so it
/// can be broadcast to every client without re-serializing per connection.
/// Returns `None` if serialization fails (the caller simply skips the tick).
pub fn encode_frame<T: Serialize>(msg: &T) -> Option<Arc<[u8]>> {
    let bytes = bincode::serialize(msg).ok()?;
    let len = bytes.len() as u32;
    let mut framed = Vec::with_capacity(4 + bytes.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(&bytes);
    Some(framed.into())
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
    }
}

/// Mark the host's listener thread as "in game" so further joiners are
/// rejected with `ServerMsg::GameInProgress`.
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
}

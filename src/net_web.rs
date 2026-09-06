//! Browser transport: WebRTC data channels between players, brokered by the
//! `signaling/` WebSocket server.
//!
//! Topology is the same star as before — the host runs the authoritative
//! simulation and every joiner talks only to the host — just over a
//! `RTCDataChannel` per joiner instead of a TCP socket.  Channels are
//! ordered + reliable, so the protocol in `net.rs` (60 Hz snapshots,
//! sequence-numbered inputs, chat) is untouched.
//!
//! Flow:
//!   host    ws `create` → `created{code}` → for each `peer{id}`:
//!           RTCPeerConnection + data channel, offer → signaling → answer,
//!           ICE both ways; on channel open wait for `ClientMsg::Hello`.
//!   joiner  ws `join{code}` → `joined` → `signal{offer}` → answer back,
//!           `ondatachannel` → on open send `Hello`, then normal traffic.
//!
//! Everything lives in a thread-local because wasm is single-threaded and
//! `web_sys` handles aren't `Send`; the game-facing `HostConn`/`ClientConn`
//! are plain mpsc channels filled from JS event callbacks and drained by the
//! `pump` system each frame.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use bevy::log::{info, warn};
use js_sys::Reflect;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    BinaryType, MessageEvent, RtcConfiguration, RtcDataChannel, RtcDataChannelEvent,
    RtcDataChannelInit, RtcDataChannelType, RtcIceCandidateInit, RtcIceServer,
    RtcPeerConnection, RtcPeerConnectionIceEvent, RtcPeerConnectionState, RtcSdpType,
    RtcSessionDescriptionInit, WebSocket,
};

use crate::net::{
    decode_limited, sanitize_chat, sanitize_nickname, ClientConn, ClientInEvent, ClientMsg,
    HostConn, OutMsg, ServerEvent, ServerMsg, CLIENT_MSG_RATE_LIMIT, HELLO_TIMEOUT,
    MAX_CLIENT_MSG_SIZE, MAX_MSG_SIZE, MAX_PLAYERS, PROTOCOL_VERSION,
};

/// Public STUN so rooms also work across the internet; on a LAN the host
/// candidates connect directly and this is never consulted.
const STUN_URL: &str = "stun:stun.l.google.com:19302";
/// Give up on a join if no data channel opened within this long.
const CONNECT_TIMEOUT_MS: f64 = 15_000.0;
/// After sending a rejection (`FullLobby` etc.) keep the channel open this
/// long so the message actually leaves before we close the connection.
const REJECT_LINGER_MS: f64 = 500.0;
/// A peer whose data channel hasn't opened by now (ICE failed, joiner
/// vanished mid-handshake) is dropped so the entry doesn't leak.
const PEER_OPEN_TIMEOUT_MS: f64 = 30_000.0;
/// ICE candidates an unauthenticated joiner may queue before its remote
/// description is set.  Real sessions produce a handful.
const MAX_PENDING_ICE: usize = 64;
/// Don't queue more than this into a joiner's data channel; drop snapshot
/// frames until it drains (control messages still go through).  A stalled
/// link would otherwise grow the SCTP send buffer without bound.
const MAX_BUFFERED_BYTES: u32 = 512 * 1024;
/// Application-level keepalive on the signaling socket (browsers can't send
/// WebSocket pings): well inside the usual 60 s proxy idle cut-off.
const SIGNALING_PING_MS: f64 = 20_000.0;

// ── Signaling protocol (mirrors signaling/src/main.rs — keep in sync) ──────

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Sig {
    Create,
    Join {
        code: String,
    },
    Signal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<u32>,
        data: SigData,
    },
    Created {
        code: String,
    },
    Joined {
        peer: u32,
    },
    Peer {
        peer: u32,
    },
    PeerLeft {
        peer: u32,
    },
    Error {
        msg: String,
    },
    Ping,
    Pong,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SigData {
    Offer {
        sdp: String,
    },
    Answer {
        sdp: String,
    },
    Ice {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u16>,
    },
}

type JsClosure = Closure<dyn FnMut(JsValue)>;

thread_local! {
    static NET: RefCell<Option<WebNet>> = const { RefCell::new(None) };
    static NEXT_SESSION: Cell<u64> = const { Cell::new(1) };
}

struct WebNet {
    session: u64,
    ws: WebSocket,
    /// `Date.now()` of the last keepalive sent on `ws`.
    last_ping: f64,
    /// Keeps the JS callbacks alive; dropped (after detaching) in `Drop`.
    _ws_closures: Vec<JsClosure>,
    role: Role,
}

enum Role {
    Host(HostState),
    Client(ClientState),
}

struct HostState {
    event_tx: Sender<ServerEvent>,
    senders: Arc<Mutex<HashMap<u8, Sender<OutMsg>>>>,
    in_game: Arc<AtomicBool>,
    /// Keyed by the signaling server's peer id.
    peers: HashMap<u32, Peer>,
}

struct Peer {
    pc: RtcPeerConnection,
    dc: RtcDataChannel,
    _closures: Vec<JsClosure>,
    /// ICE candidates that arrived before the remote description was set.
    pending_ice: Vec<SigData>,
    remote_ready: bool,
    /// Data channel is open (set from `onopen`).
    open: bool,
    /// `Date.now()` when the channel opened — the Hello handshake must land
    /// within `HELLO_TIMEOUT` of this.
    opened_at: Option<f64>,
    /// Game player id once `Hello` was accepted.
    id: Option<u8>,
    /// Outgoing queue (the `Sender` half lives in `HostConn::senders`).
    out_rx: Option<Receiver<OutMsg>>,
    /// Scheduled close after a rejection message was sent.
    close_at: Option<f64>,
    /// `Date.now()` when the peer connection was created (open timeout).
    created_at: f64,
    // Rolling one-second message counter for the rate limit.
    window_start: f64,
    window_count: u32,
}

struct ClientState {
    event_tx: Sender<ClientInEvent>,
    out_rx: Receiver<ClientMsg>,
    hello: Vec<u8>,
    started_at: f64,
    pc: Option<RtcPeerConnection>,
    dc: Option<RtcDataChannel>,
    closures: Vec<JsClosure>,
    pending_ice: Vec<SigData>,
    remote_ready: bool,
    open: bool,
    welcomed: bool,
}

impl Drop for WebNet {
    fn drop(&mut self) {
        // Detach every handler before the closures go away, otherwise a late
        // browser event would call into a freed closure and panic.
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        self.ws.set_onerror(None);
        let _ = self.ws.close();
        match &mut self.role {
            Role::Host(h) => {
                for peer in h.peers.values() {
                    detach_and_close(&peer.pc, &peer.dc);
                }
            }
            Role::Client(c) => {
                if let (Some(pc), Some(dc)) = (&c.pc, &c.dc) {
                    detach_and_close(pc, dc);
                } else if let Some(pc) = &c.pc {
                    pc.set_onicecandidate(None);
                    pc.set_ondatachannel(None);
                    pc.set_onconnectionstatechange(None);
                    pc.close();
                }
            }
        }
    }
}

fn detach_and_close(pc: &RtcPeerConnection, dc: &RtcDataChannel) {
    dc.set_onopen(None);
    dc.set_onmessage(None);
    dc.set_onclose(None);
    dc.set_onerror(None);
    pc.set_onicecandidate(None);
    pc.set_ondatachannel(None);
    pc.set_onconnectionstatechange(None);
    dc.close();
    pc.close();
}

// ── Small helpers ──────────────────────────────────────────────────────────

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn js_err(v: JsValue) -> String {
    v.as_string()
        .or_else(|| Reflect::get(&v, &"message".into()).ok().and_then(|m| m.as_string()))
        .unwrap_or_else(|| format!("{v:?}"))
}

/// Run `f` against the live session, if it is still the one `session` names.
/// Callbacks capture only the session number, so a stale event from a torn
/// down session is ignored instead of poking at a newer one.  Returns `None`
/// when the session is gone or currently borrowed (a re-entrant JS event).
fn with_net<R>(session: u64, f: impl FnOnce(&mut WebNet) -> R) -> Option<R> {
    NET.with(|slot| {
        let mut guard = slot.try_borrow_mut().ok()?;
        let net = guard.as_mut()?;
        if net.session != session {
            return None;
        }
        Some(f(net))
    })
}

/// Drop the session `session`, if it is the live one.  Called from the
/// `HostConn` / `ClientConn` drops.
pub fn teardown(session: u64) {
    NET.with(|slot| {
        if let Ok(mut guard) = slot.try_borrow_mut() {
            if guard.as_ref().map(|n| n.session) == Some(session) {
                *guard = None;
            }
        }
    });
}

/// Room code from an invite link (`?room=K7X2`), exported by the page as
/// `window.ZG_ROOM`.  `None` when the page was opened plainly.
pub fn room_from_url() -> Option<String> {
    let win = web_sys::window()?;
    let code = Reflect::get(&win, &"ZG_ROOM".into()).ok()?.as_string()?;
    let code = code.trim().to_ascii_uppercase();
    (!code.is_empty()).then_some(code)
}

fn signaling_url() -> Result<String, String> {
    let win = web_sys::window().ok_or("no window")?;
    if let Ok(v) = Reflect::get(&win, &"ZG_SIGNALING".into()) {
        if let Some(s) = v.as_string() {
            if !s.is_empty() {
                return Ok(s);
            }
        }
    }
    let loc = win.location();
    let proto = loc.protocol().map_err(js_err)?;
    let host = loc.host().map_err(js_err)?;
    let scheme = if proto == "https:" { "wss" } else { "ws" };
    Ok(format!("{scheme}://{host}/ws"))
}

fn ws_send(ws: &WebSocket, msg: &Sig) {
    if let Ok(s) = serde_json::to_string(msg) {
        if let Err(e) = ws.send_with_str(&s) {
            warn!("signaling send failed: {}", js_err(e));
        }
    }
}

fn new_peer_connection() -> Result<RtcPeerConnection, String> {
    let cfg = RtcConfiguration::new();
    let servers = js_sys::Array::new();
    let stun = RtcIceServer::new();
    stun.set_urls_str(STUN_URL);
    servers.push(&stun);
    cfg.set_ice_servers(&servers);
    RtcPeerConnection::new_with_configuration(&cfg).map_err(js_err)
}

fn dc_send(dc: &RtcDataChannel, bytes: &[u8]) -> bool {
    match dc.send_with_u8_array(bytes) {
        Ok(()) => true,
        Err(e) => {
            warn!("data channel send failed: {}", js_err(e));
            false
        }
    }
}

fn message_bytes(ev: &MessageEvent) -> Option<Vec<u8>> {
    let data = ev.data();
    if data.is_instance_of::<js_sys::ArrayBuffer>() {
        Some(js_sys::Uint8Array::new(&data).to_vec())
    } else {
        None
    }
}

/// Feed one ICE candidate to `pc` (fire-and-forget; failures are logged).
fn add_ice(pc: &RtcPeerConnection, data: &SigData) {
    let SigData::Ice { candidate, sdp_mid, sdp_m_line_index } = data else {
        return;
    };
    let init = RtcIceCandidateInit::new(candidate);
    init.set_sdp_mid(sdp_mid.as_deref());
    init.set_sdp_m_line_index(*sdp_m_line_index);
    let promise = pc.add_ice_candidate_with_opt_rtc_ice_candidate_init(Some(&init));
    spawn_local(async move {
        if let Err(e) = JsFuture::from(promise).await {
            warn!("addIceCandidate failed: {}", js_err(e));
        }
    });
}

/// Wire `onconnectionstatechange`: a data channel only fires `onclose` when
/// the remote closes it deliberately; a dead link (network drop, killed tab)
/// surfaces as `Failed` / `Disconnected` here instead.  `on_lost` runs once
/// for the first such transition.
fn hook_connection_state(pc: &RtcPeerConnection, mut on_lost: impl FnMut() + 'static) -> JsClosure {
    let pc2 = pc.clone();
    let mut fired = false;
    let cb: JsClosure = Closure::new(move |_: JsValue| {
        use RtcPeerConnectionState as S;
        if !fired && matches!(pc2.connection_state(), S::Failed | S::Disconnected | S::Closed) {
            fired = true;
            on_lost();
        }
    });
    pc.set_onconnectionstatechange(Some(cb.as_ref().unchecked_ref()));
    cb
}

/// Wire `onicecandidate` so local candidates are relayed to `to` (host →
/// joiner) or to the host (`None`).
fn hook_ice(pc: &RtcPeerConnection, session: u64, to: Option<u32>) -> JsClosure {
    let on_ice: JsClosure = Closure::new(move |ev: JsValue| {
        let ev: RtcPeerConnectionIceEvent = ev.unchecked_into();
        let Some(c) = ev.candidate() else { return };
        let candidate = c.candidate();
        if candidate.is_empty() {
            return; // end-of-candidates marker
        }
        let data = SigData::Ice {
            candidate,
            sdp_mid: c.sdp_mid(),
            sdp_m_line_index: c.sdp_m_line_index(),
        };
        with_net(session, |net| {
            ws_send(&net.ws, &Sig::Signal { to, from: None, data });
        });
    });
    pc.set_onicecandidate(Some(on_ice.as_ref().unchecked_ref()));
    on_ice
}

// ── Host ───────────────────────────────────────────────────────────────────

pub fn start_host() -> Result<HostConn, String> {
    let url = signaling_url()?;
    let ws = WebSocket::new(&url).map_err(js_err)?;
    ws.set_binary_type(BinaryType::Arraybuffer);
    let session = NEXT_SESSION.with(|s| {
        let v = s.get();
        s.set(v + 1);
        v
    });

    let (event_tx, event_rx) = channel::<ServerEvent>();
    let senders: Arc<Mutex<HashMap<u8, Sender<OutMsg>>>> = Arc::new(Mutex::new(HashMap::new()));
    let in_game = Arc::new(AtomicBool::new(false));

    let ws_closures = hook_ws(&ws, session, Sig::Create, {
        let event_tx = event_tx.clone();
        move |reason| {
            let _ = event_tx.send(ServerEvent::HostError { reason });
        }
    });

    NET.with(|slot| {
        *slot.borrow_mut() = Some(WebNet {
            session,
            ws,
            last_ping: now_ms(),
            _ws_closures: ws_closures,
            role: Role::Host(HostState {
                event_tx,
                senders: senders.clone(),
                in_game: in_game.clone(),
                peers: HashMap::new(),
            }),
        });
    });
    info!("hosting via {url}");

    Ok(HostConn {
        events: Arc::new(Mutex::new(event_rx)),
        senders,
        in_game,
        session,
    })
}

/// Attach open/message/close/error handlers to the signaling socket.
/// `first` is sent as soon as the socket opens; `on_lost` fires if the socket
/// dies before the session is usable.
fn hook_ws(
    ws: &WebSocket,
    session: u64,
    first: Sig,
    on_lost: impl Fn(String) + 'static,
) -> Vec<JsClosure> {
    let first_json = serde_json::to_string(&first).unwrap_or_default();
    let on_open: JsClosure = Closure::new(move |_: JsValue| {
        with_net(session, |net| {
            if let Err(e) = net.ws.send_with_str(&first_json) {
                warn!("signaling send failed: {}", js_err(e));
            }
        });
    });
    let on_message: JsClosure = Closure::new(move |ev: JsValue| {
        let ev: MessageEvent = ev.unchecked_into();
        let Some(text) = ev.data().as_string() else { return };
        let sig = match serde_json::from_str::<Sig>(&text) {
            Ok(s) => s,
            Err(e) => {
                warn!("bad signaling message: {e}");
                return;
            }
        };
        with_net(session, |net| match &mut net.role {
            Role::Host(_) => host_on_sig(net, session, sig),
            Role::Client(_) => client_on_sig(net, session, sig),
        });
    });
    let on_close: JsClosure = Closure::new(move |_: JsValue| {
        with_net(session, |net| match &mut net.role {
            // Host: once peers are connected the signaling link is only
            // needed for *new* joiners; losing it is not fatal.  Before the
            // room exists it is.
            Role::Host(h) => {
                if h.peers.is_empty() {
                    on_lost("SIGNALING LOST".into());
                }
            }
            Role::Client(c) => {
                if !c.open {
                    let _ = c.event_tx.send(ClientInEvent::ConnectFailed {
                        reason: "SIGNALING LOST".into(),
                    });
                }
            }
        });
    });
    let on_error: JsClosure = Closure::new(move |_: JsValue| {
        warn!("signaling socket error");
    });
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    vec![on_open, on_message, on_close, on_error]
}

fn host_on_sig(net: &mut WebNet, session: u64, sig: Sig) {
    let WebNet { ws, role, .. } = net;
    let Role::Host(host) = role else { return };
    match sig {
        Sig::Created { code } => {
            info!("room {code} created");
            if let Some(win) = web_sys::window() {
                // Expose the code to the page: tests read `zgRoom`, and
                // `zgRoomCreated` (web/index.html) turns it into a shareable
                // `?room=` URL.
                let _ = Reflect::set(&win, &"zgRoom".into(), &code.clone().into());
                if let Ok(f) = Reflect::get(&win, &"zgRoomCreated".into()) {
                    if let Some(f) = f.dyn_ref::<js_sys::Function>() {
                        let _ = f.call1(&win, &code.clone().into());
                    }
                }
            }
            let _ = host.event_tx.send(ServerEvent::RoomCode { code });
        }
        Sig::Peer { peer } => {
            if let Err(e) = host_add_peer(host, ws, session, peer) {
                warn!("peer {peer}: {e}");
            }
        }
        Sig::Signal { from: Some(from), data, .. } => {
            let Some(peer) = host.peers.get_mut(&from) else { return };
            match data {
                SigData::Answer { sdp } => {
                    let pc = peer.pc.clone();
                    spawn_local(async move {
                        let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                        init.set_sdp(&sdp);
                        if let Err(e) = JsFuture::from(pc.set_remote_description(&init)).await {
                            warn!("setRemoteDescription(answer) failed: {}", js_err(e));
                            return;
                        }
                        with_net(session, |net| {
                            if let Role::Host(h) = &mut net.role {
                                if let Some(p) = h.peers.get_mut(&from) {
                                    p.remote_ready = true;
                                    for ice in p.pending_ice.drain(..) {
                                        add_ice(&p.pc, &ice);
                                    }
                                }
                            }
                        });
                    });
                }
                ice @ SigData::Ice { .. } => {
                    if peer.remote_ready {
                        add_ice(&peer.pc, &ice);
                    } else if peer.pending_ice.len() < MAX_PENDING_ICE {
                        peer.pending_ice.push(ice);
                    }
                }
                SigData::Offer { .. } => warn!("joiner sent an offer — ignored"),
            }
        }
        Sig::PeerLeft { peer } => {
            // The joiner's *signaling* socket went away.  Before the data
            // channel is up that means the join is dead; afterwards the
            // socket is irrelevant (a proxy idle-cut must not kick a live
            // player) — the data channel and connection state decide.
            if host.peers.get(&peer).map_or(false, |p| !p.open) {
                host_drop_peer(host, peer);
            }
        }
        Sig::Error { msg } => {
            warn!("signaling error: {msg}");
            if host.peers.is_empty() {
                let _ = host.event_tx.send(ServerEvent::HostError { reason: msg });
            }
        }
        Sig::Signal { from: None, .. } | Sig::Joined { .. } | Sig::Create | Sig::Join { .. } | Sig::Ping | Sig::Pong => {}
    }
}

/// New joiner announced by signaling: build the peer connection, open the
/// data channel and send an offer.
fn host_add_peer(host: &mut HostState, _ws: &WebSocket, session: u64, peer_id: u32) -> Result<(), String> {
    let pc = new_peer_connection()?;
    let init = RtcDataChannelInit::new();
    init.set_ordered(true);
    let dc = pc.create_data_channel_with_data_channel_dict("game", &init);
    dc.set_binary_type(RtcDataChannelType::Arraybuffer);

    let mut closures = vec![hook_ice(&pc, session, Some(peer_id))];
    closures.push(hook_connection_state(&pc, move || {
        with_net(session, |net| {
            if let Role::Host(h) = &mut net.role {
                warn!("peer {peer_id}: connection lost");
                host_drop_peer(h, peer_id);
            }
        });
    }));

    let on_open: JsClosure = Closure::new(move |_: JsValue| {
        with_net(session, |net| {
            if let Role::Host(h) = &mut net.role {
                if let Some(p) = h.peers.get_mut(&peer_id) {
                    p.open = true;
                    p.opened_at = Some(now_ms());
                }
            }
        });
    });
    dc.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    closures.push(on_open);

    let on_message: JsClosure = Closure::new(move |ev: JsValue| {
        let ev: MessageEvent = ev.unchecked_into();
        let Some(bytes) = message_bytes(&ev) else { return };
        with_net(session, |net| {
            if let Role::Host(h) = &mut net.role {
                host_on_peer_message(h, peer_id, &bytes);
            }
        });
    });
    dc.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    closures.push(on_message);

    let on_close: JsClosure = Closure::new(move |_: JsValue| {
        with_net(session, |net| {
            if let Role::Host(h) = &mut net.role {
                host_drop_peer(h, peer_id);
            }
        });
    });
    dc.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    closures.push(on_close);

    // Offer → local description → relay the SDP.
    let pc2 = pc.clone();
    spawn_local(async move {
        let res: Result<String, JsValue> = async {
            let offer = JsFuture::from(pc2.create_offer()).await?;
            let init: RtcSessionDescriptionInit = offer.unchecked_into();
            JsFuture::from(pc2.set_local_description(&init)).await?;
            Ok(Reflect::get(&init, &"sdp".into())?.as_string().unwrap_or_default())
        }
        .await;
        match res {
            Ok(sdp) => {
                with_net(session, |net| {
                    ws_send(
                        &net.ws,
                        &Sig::Signal { to: Some(peer_id), from: None, data: SigData::Offer { sdp } },
                    );
                });
            }
            Err(e) => warn!("offer failed: {}", js_err(e)),
        }
    });

    host.peers.insert(
        peer_id,
        Peer {
            pc,
            dc,
            _closures: closures,
            pending_ice: Vec::new(),
            remote_ready: false,
            open: false,
            opened_at: None,
            id: None,
            out_rx: None,
            close_at: None,
            created_at: now_ms(),
            window_start: now_ms(),
            window_count: 0,
        },
    );
    Ok(())
}

/// A joiner's channel closed (or signaling says they left): free their
/// player id and tell the game.
fn host_drop_peer(host: &mut HostState, peer_id: u32) {
    let Some(peer) = host.peers.remove(&peer_id) else { return };
    detach_and_close(&peer.pc, &peer.dc);
    if let Some(id) = peer.id {
        host.senders.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        let _ = host.event_tx.send(ServerEvent::Disconnected { id });
    }
}

/// One message from a joiner's data channel.  Mirrors the old per-client
/// reader thread: a `Hello` handshake first, then inputs / chat / leave.
fn host_on_peer_message(host: &mut HostState, peer_id: u32, bytes: &[u8]) {
    let Some(peer) = host.peers.get_mut(&peer_id) else { return };
    if peer.close_at.is_some() {
        return; // already rejected, draining
    }

    // Rate limit: more than CLIENT_MSG_RATE_LIMIT messages in any rolling
    // one-second window and the peer is dropped.
    let now = now_ms();
    if now - peer.window_start >= 1000.0 {
        peer.window_start = now;
        peer.window_count = 0;
    }
    peer.window_count += 1;
    if peer.window_count > CLIENT_MSG_RATE_LIMIT {
        warn!("peer {peer_id} exceeded message rate — dropping");
        host_drop_peer(host, peer_id);
        return;
    }

    let msg = match decode_limited::<ClientMsg>(bytes, MAX_CLIENT_MSG_SIZE) {
        Ok(m) => m,
        Err(e) => {
            warn!("peer {peer_id}: bad message ({e}) — dropping");
            host_drop_peer(host, peer_id);
            return;
        }
    };

    let Some(id) = peer.id else {
        // Handshake: the first message must be a Hello with our protocol
        // version; anything else (or a lobby that's full / a round already
        // running) gets a rejection message and a scheduled close so the
        // rejection actually reaches them.
        let ClientMsg::Hello { nickname, protocol_version } = msg else {
            warn!("peer {peer_id}: expected Hello — dropping");
            host_drop_peer(host, peer_id);
            return;
        };
        let reject = if protocol_version != PROTOCOL_VERSION {
            Some(ServerMsg::ProtocolMismatch { server_version: PROTOCOL_VERSION })
        } else if host.in_game.load(Ordering::Relaxed) {
            Some(ServerMsg::GameInProgress)
        } else {
            let count = host.senders.lock().unwrap_or_else(|e| e.into_inner()).len();
            if count >= (MAX_PLAYERS - 1) as usize {
                Some(ServerMsg::FullLobby)
            } else {
                None
            }
        };
        if let Some(reject) = reject {
            if let Ok(bytes) = bincode::serialize(&reject) {
                dc_send(&peer.dc, &bytes);
            }
            peer.close_at = Some(now + REJECT_LINGER_MS);
            return;
        }
        // First free id above the host's 0 — never wraps, never collides
        // with a stale entry.
        let id = {
            let senders = host.senders.lock().unwrap_or_else(|e| e.into_inner());
            (1..MAX_PLAYERS).find(|i| !senders.contains_key(i))
        };
        let Some(id) = id else {
            peer.close_at = Some(now + REJECT_LINGER_MS);
            return;
        };
        // Welcome goes straight out on the channel (synchronous), so it
        // precedes anything the lobby queues on `out_tx` — the client learns
        // its id before the first `LobbyState`.
        let welcome = ServerMsg::Welcome { your_id: id, protocol_version: PROTOCOL_VERSION };
        let sent = bincode::serialize(&welcome).map_or(false, |b| dc_send(&peer.dc, &b));
        if !sent {
            host_drop_peer(host, peer_id);
            return;
        }
        let (out_tx, out_rx) = channel::<OutMsg>();
        host.senders.lock().unwrap_or_else(|e| e.into_inner()).insert(id, out_tx);
        peer.out_rx = Some(out_rx);
        peer.id = Some(id);
        let _ = host.event_tx.send(ServerEvent::Connected { id });
        let _ = host.event_tx.send(ServerEvent::Hello { id, nickname: sanitize_nickname(&nickname) });
        return;
    };

    match msg {
        ClientMsg::Input(input) => {
            let _ = host.event_tx.send(ServerEvent::Input { id, input });
        }
        ClientMsg::Hello { nickname, .. } => {
            // Late re-Hello (client renamed itself) — version already checked.
            let _ = host.event_tx.send(ServerEvent::Hello { id, nickname: sanitize_nickname(&nickname) });
        }
        ClientMsg::Chat { text } => {
            if let Some(clean) = sanitize_chat(&text) {
                let _ = host.event_tx.send(ServerEvent::ChatRelay { id, text: clean });
            }
        }
        ClientMsg::Leave => host_drop_peer(host, peer_id),
    }
}

// ── Client ─────────────────────────────────────────────────────────────────

pub fn start_client(code: &str, nickname: &str) -> Result<ClientConn, String> {
    let url = signaling_url()?;
    let ws = WebSocket::new(&url).map_err(js_err)?;
    ws.set_binary_type(BinaryType::Arraybuffer);
    let session = NEXT_SESSION.with(|s| {
        let v = s.get();
        s.set(v + 1);
        v
    });

    let (event_tx, event_rx) = channel::<ClientInEvent>();
    let (send_tx, send_rx) = channel::<ClientMsg>();
    let hello = bincode::serialize(&ClientMsg::Hello {
        nickname: sanitize_nickname(nickname),
        protocol_version: PROTOCOL_VERSION,
    })
    .map_err(|e| e.to_string())?;

    let ws_closures = hook_ws(&ws, session, Sig::Join { code: code.trim().to_ascii_uppercase() }, |_| {});

    NET.with(|slot| {
        *slot.borrow_mut() = Some(WebNet {
            session,
            ws,
            last_ping: now_ms(),
            _ws_closures: ws_closures,
            role: Role::Client(ClientState {
                event_tx,
                out_rx: send_rx,
                hello,
                started_at: now_ms(),
                pc: None,
                dc: None,
                closures: Vec::new(),
                pending_ice: Vec::new(),
                remote_ready: false,
                open: false,
                welcomed: false,
            }),
        });
    });
    info!("joining room {code} via {url}");

    Ok(ClientConn {
        events: Arc::new(Mutex::new(event_rx)),
        sender: send_tx,
        session,
    })
}

fn client_on_sig(net: &mut WebNet, session: u64, sig: Sig) {
    let WebNet { ws, role, .. } = net;
    let Role::Client(client) = role else { return };
    match sig {
        Sig::Joined { peer } => info!("joined as peer {peer}; waiting for the host's offer"),
        Sig::Signal { data: SigData::Offer { sdp }, .. } => {
            if client.pc.is_some() {
                warn!("second offer — ignored");
                return;
            }
            let pc = match new_peer_connection() {
                Ok(pc) => pc,
                Err(e) => {
                    let _ = client.event_tx.send(ClientInEvent::ConnectFailed { reason: e });
                    return;
                }
            };
            client.closures.push(hook_ice(&pc, session, None));
            client.closures.push(hook_connection_state(&pc, move || {
                with_net(session, |net| {
                    if let Role::Client(c) = &mut net.role {
                        let _ = c.event_tx.send(if c.welcomed {
                            ClientInEvent::Disconnected
                        } else {
                            ClientInEvent::ConnectFailed { reason: "CONNECTION LOST".into() }
                        });
                    }
                });
            }));
            let on_dc: JsClosure = Closure::new(move |ev: JsValue| {
                let ev: RtcDataChannelEvent = ev.unchecked_into();
                let dc = ev.channel();
                with_net(session, |net| {
                    if let Role::Client(c) = &mut net.role {
                        client_attach_channel(c, session, dc);
                    }
                });
            });
            pc.set_ondatachannel(Some(on_dc.as_ref().unchecked_ref()));
            client.closures.push(on_dc);

            let pc2 = pc.clone();
            client.pc = Some(pc);
            spawn_local(async move {
                let res: Result<String, JsValue> = async {
                    let init = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                    init.set_sdp(&sdp);
                    JsFuture::from(pc2.set_remote_description(&init)).await?;
                    let answer = JsFuture::from(pc2.create_answer()).await?;
                    let ainit: RtcSessionDescriptionInit = answer.unchecked_into();
                    JsFuture::from(pc2.set_local_description(&ainit)).await?;
                    Ok(Reflect::get(&ainit, &"sdp".into())?.as_string().unwrap_or_default())
                }
                .await;
                with_net(session, |net| {
                    let WebNet { ws, role, .. } = net;
                    let Role::Client(c) = role else { return };
                    match res {
                        Ok(sdp) => {
                            c.remote_ready = true;
                            if let Some(pc) = &c.pc {
                                for ice in c.pending_ice.drain(..) {
                                    add_ice(pc, &ice);
                                }
                            }
                            ws_send(ws, &Sig::Signal { to: None, from: None, data: SigData::Answer { sdp } });
                        }
                        Err(e) => {
                            let _ = c.event_tx.send(ClientInEvent::ConnectFailed {
                                reason: format!("WEBRTC: {}", js_err(e)),
                            });
                        }
                    }
                });
            });
        }
        Sig::Signal { data: ice @ SigData::Ice { .. }, .. } => match (&client.pc, client.remote_ready) {
            (Some(pc), true) => add_ice(pc, &ice),
            _ => {
                if client.pending_ice.len() < MAX_PENDING_ICE {
                    client.pending_ice.push(ice);
                }
            }
        },
        Sig::Signal { data: SigData::Answer { .. }, .. } => warn!("host sent an answer — ignored"),
        Sig::Error { msg } => {
            // Before the channel opens this is why the join failed.  After,
            // the signaling socket no longer matters ("HOST LEFT" here can
            // just be the host's socket idling out behind a proxy); a dead
            // host is caught by the peer connection state instead.
            if client.open {
                warn!("signaling: {msg} (ignored — game channel is up)");
            } else {
                let _ = client.event_tx.send(ClientInEvent::ConnectFailed { reason: msg });
            }
        }
        Sig::Created { .. } | Sig::Peer { .. } | Sig::PeerLeft { .. } | Sig::Create | Sig::Join { .. } | Sig::Ping | Sig::Pong => {}
    }
    let _ = ws;
}

/// The host's data channel arrived: hook it up, and send `Hello` the moment
/// it opens.
fn client_attach_channel(client: &mut ClientState, session: u64, dc: RtcDataChannel) {
    dc.set_binary_type(RtcDataChannelType::Arraybuffer);
    let hello = client.hello.clone();
    let dc_for_open = dc.clone();
    let on_open: JsClosure = Closure::new(move |_: JsValue| {
        dc_send(&dc_for_open, &hello);
        with_net(session, |net| {
            if let Role::Client(c) = &mut net.role {
                c.open = true;
            }
        });
    });
    dc.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    let event_tx = client.event_tx.clone();
    let on_message: JsClosure = Closure::new(move |ev: JsValue| {
        let ev: MessageEvent = ev.unchecked_into();
        let Some(bytes) = message_bytes(&ev) else { return };
        let msg = match decode_limited::<ServerMsg>(&bytes, MAX_MSG_SIZE) {
            Ok(m) => m,
            Err(e) => {
                warn!("bad server message: {e}");
                let _ = event_tx.send(ClientInEvent::Disconnected);
                return;
            }
        };
        let ev = match msg {
            ServerMsg::Welcome { your_id, protocol_version } => {
                if protocol_version != PROTOCOL_VERSION {
                    ClientInEvent::ProtocolMismatch { server_version: protocol_version }
                } else {
                    with_net(session, |net| {
                        if let Role::Client(c) = &mut net.role {
                            c.welcomed = true;
                        }
                    });
                    ClientInEvent::Welcomed { your_id }
                }
            }
            ServerMsg::LobbyState { players } => ClientInEvent::LobbyState { players },
            ServerMsg::StartGame => ClientInEvent::Started,
            ServerMsg::CountdownStart { seconds } => ClientInEvent::CountdownStart { seconds },
            ServerMsg::CountdownCancel => ClientInEvent::CountdownCancel,
            ServerMsg::Snapshot(snap) => ClientInEvent::Snapshot(snap),
            ServerMsg::FullLobby => ClientInEvent::FullLobby,
            ServerMsg::GameInProgress => ClientInEvent::GameInProgress,
            ServerMsg::ProtocolMismatch { server_version } => {
                ClientInEvent::ProtocolMismatch { server_version }
            }
            ServerMsg::Chat { author, text } => ClientInEvent::Chat { author, text },
        };
        let _ = event_tx.send(ev);
    });
    dc.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let event_tx = client.event_tx.clone();
    let on_close: JsClosure = Closure::new(move |_: JsValue| {
        let welcomed = with_net(session, |net| match &net.role {
            Role::Client(c) => c.welcomed,
            _ => true,
        })
        .unwrap_or(true);
        let _ = event_tx.send(if welcomed {
            ClientInEvent::Disconnected
        } else {
            ClientInEvent::ConnectFailed { reason: "CONNECTION CLOSED".into() }
        });
    });
    dc.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    client.closures.push(on_open);
    client.closures.push(on_message);
    client.closures.push(on_close);
    client.dc = Some(dc);
}

// ── Per-frame pump ─────────────────────────────────────────────────────────

/// Bevy system (`Last`): push everything the game queued on the connection
/// handles into the data channels, and run the time-based housekeeping
/// (handshake timeout, deferred rejection closes, join timeout).
pub fn pump() {
    NET.with(|slot| {
        let Ok(mut guard) = slot.try_borrow_mut() else { return };
        let Some(net) = guard.as_mut() else { return };
        let now = now_ms();
        if now - net.last_ping > SIGNALING_PING_MS {
            net.last_ping = now;
            if net.ws.ready_state() == WebSocket::OPEN {
                ws_send(&net.ws, &Sig::Ping);
            }
        }
        match &mut net.role {
            Role::Host(h) => {
                let mut drop_list = Vec::new();
                for (&peer_id, peer) in h.peers.iter_mut() {
                    if let Some(at) = peer.close_at {
                        if now >= at {
                            drop_list.push(peer_id);
                        }
                        continue;
                    }
                    if !peer.open {
                        if now - peer.created_at > PEER_OPEN_TIMEOUT_MS {
                            warn!("peer {peer_id}: data channel never opened — dropping");
                            drop_list.push(peer_id);
                        }
                        continue;
                    }
                    if peer.id.is_none() {
                        if peer.opened_at.map_or(false, |t| now - t > HELLO_TIMEOUT.as_millis() as f64) {
                            warn!("peer {peer_id}: no Hello within timeout — dropping");
                            drop_list.push(peer_id);
                        }
                        continue;
                    }
                    let Some(rx) = peer.out_rx.as_ref() else { continue };
                    while let Ok(out) = rx.try_recv() {
                        let ok = match out {
                            // Snapshots are superseded every tick — when the
                            // link can't keep up, dropping them is the
                            // right call; control messages always go.
                            OutMsg::Frame(_) if peer.dc.buffered_amount() > MAX_BUFFERED_BYTES => true,
                            OutMsg::Frame(frame) => dc_send(&peer.dc, &frame),
                            OutMsg::Msg(msg) => match bincode::serialize(&msg) {
                                Ok(bytes) => dc_send(&peer.dc, &bytes),
                                Err(_) => true,
                            },
                        };
                        if !ok {
                            drop_list.push(peer_id);
                            break;
                        }
                    }
                }
                for peer_id in drop_list {
                    host_drop_peer(h, peer_id);
                }
            }
            Role::Client(c) => {
                if !c.open {
                    if now - c.started_at > CONNECT_TIMEOUT_MS {
                        let _ = c.event_tx.send(ClientInEvent::ConnectFailed {
                            reason: "CONNECTION TIMED OUT".into(),
                        });
                        // Stop re-sending: bump started_at far into the future.
                        c.started_at = f64::INFINITY;
                    }
                    return;
                }
                let Some(dc) = c.dc.as_ref() else { return };
                while let Ok(msg) = c.out_rx.try_recv() {
                    if let Ok(bytes) = bincode::serialize(&msg) {
                        if !dc_send(dc, &bytes) {
                            let _ = c.event_tx.send(ClientInEvent::Disconnected);
                            break;
                        }
                    }
                }
            }
        }
    });
}

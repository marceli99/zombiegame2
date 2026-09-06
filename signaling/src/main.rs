//! Signaling server for zombiegame2's browser multiplayer.
//!
//! Browsers can't listen on a port, so two players can only reach each other
//! through WebRTC — and WebRTC needs a third party to carry the initial
//! offer/answer/ICE handshake.  That's all this is: a host opens a room and
//! gets a 4-letter code, joiners present the code, and every `signal`
//! message is forwarded verbatim to the other side.  Once the data channel
//! is up the game traffic flows peer-to-peer and this server is idle.
//!
//! It also serves the static web build so one process is a complete
//! deployment:
//!
//!   cargo run -p signaling -- [--listen 0.0.0.0:8000] [--web ./web]
//!
//! Wire format (JSON text frames; `t` is the tag, mirrored by the `Sig`
//! enum in `src/net_web.rs` — keep the two in sync):
//!
//!   client → server   {"t":"create"}
//!                     {"t":"join","code":"K7X2"}
//!                     {"t":"signal","to":<peer>,"data":{...}}   (host → joiner)
//!                     {"t":"signal","data":{...}}               (joiner → host)
//!   server → client   {"t":"created","code":"K7X2"}
//!                     {"t":"joined","peer":<id>}
//!                     {"t":"peer","peer":<id>}                  (to host)
//!                     {"t":"peer_left","peer":<id>}             (to host)
//!                     {"t":"signal","from":<peer>,"data":{...}}
//!                     {"t":"error","msg":"ROOM NOT FOUND"}

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tower_http::services::ServeDir;

/// Same cap as the game's `MAX_PLAYERS` minus the host.
const MAX_JOINERS: usize = 3;
/// Rooms alive at once — a bound on memory, not a product limit.
const MAX_ROOMS: usize = 1000;
/// Failed `join`s a socket may attempt before it is dropped: enough for a
/// typo or two, far too few to brute-force a 32^4 code space.
const MAX_JOIN_ATTEMPTS: u32 = 5;
/// Anything bigger than a WebRTC offer with a fat ICE section is not ours.
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
/// Sockets that stay silent this long are dropped.  Clients ping every
/// ~20 s (see `Sig::Ping`), so this only triggers for dead connections —
/// and it is what reclaims rooms whose host vanished without a close.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Room codes avoid look-alike glyphs (0/O, 1/I) — they get read out loud.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 4;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Sig {
    Create,
    Join { code: String },
    Signal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<u32>,
        data: Value,
    },
    Created { code: String },
    Joined { peer: u32 },
    Peer { peer: u32 },
    PeerLeft { peer: u32 },
    Error { msg: String },
    /// Browser WebSockets can't send protocol pings, so clients send this
    /// every ~20 s to keep idle proxies from cutting the socket.
    Ping,
    Pong,
}

type Tx = UnboundedSender<String>;

struct Room {
    host: Tx,
    peers: HashMap<u32, Tx>,
    next_peer: u32,
}

#[derive(Default)]
struct Rooms(HashMap<String, Room>);

type Shared = Arc<Mutex<Rooms>>;

fn send(tx: &Tx, msg: &Sig) {
    if let Ok(s) = serde_json::to_string(msg) {
        let _ = tx.send(s);
    }
}

fn new_code(rooms: &Rooms) -> Option<String> {
    let mut rng = rand::thread_rng();
    // 32^4 codes vs. ≤ MAX_ROOMS live ones: a collision streak this long
    // means something is very wrong, not that we should spin forever.
    for _ in 0..64 {
        let code: String = (0..CODE_LEN)
            .map(|_| *CODE_ALPHABET.choose(&mut rng).unwrap() as char)
            .collect();
        if !rooms.0.contains_key(&code) {
            return Some(code);
        }
    }
    None
}

/// What this socket turned out to be, decided by its first message.
enum Role {
    Host { code: String },
    Joiner { code: String, peer: u32 },
}

async fn handle_socket(socket: WebSocket, rooms: Shared) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = unbounded_channel::<String>();
    // Writer task: everything queued for this socket goes out in order.
    let writer = tokio::spawn(async move {
        while let Some(s) = rx.recv().await {
            if sink.send(Message::Text(s)).await.is_err() {
                break;
            }
        }
    });

    let mut role: Option<Role> = None;
    let mut failed_joins: u32 = 0;
    loop {
        let msg = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(msg))) => msg,
            // Idle past the deadline, closed, or a protocol error.
            _ => break,
        };
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(sig) = serde_json::from_str::<Sig>(&text) else {
            send(&tx, &Sig::Error { msg: "BAD MESSAGE".into() });
            continue;
        };
        let mut rooms_guard = rooms.lock().unwrap_or_else(|e| e.into_inner());
        match (&role, sig) {
            (_, Sig::Ping) => send(&tx, &Sig::Pong),
            (None, Sig::Create) => {
                if rooms_guard.0.len() >= MAX_ROOMS {
                    send(&tx, &Sig::Error { msg: "SERVER FULL".into() });
                    continue;
                }
                let Some(code) = new_code(&rooms_guard) else {
                    send(&tx, &Sig::Error { msg: "SERVER FULL".into() });
                    continue;
                };
                rooms_guard.0.insert(
                    code.clone(),
                    Room { host: tx.clone(), peers: HashMap::new(), next_peer: 1 },
                );
                send(&tx, &Sig::Created { code: code.clone() });
                eprintln!("room {code} created");
                role = Some(Role::Host { code });
            }
            (None, Sig::Join { code }) => {
                let code = code.trim().to_ascii_uppercase();
                let Some(room) = rooms_guard.0.get_mut(&code) else {
                    send(&tx, &Sig::Error { msg: "ROOM NOT FOUND".into() });
                    failed_joins += 1;
                    if failed_joins >= MAX_JOIN_ATTEMPTS {
                        break;
                    }
                    continue;
                };
                if room.peers.len() >= MAX_JOINERS {
                    send(&tx, &Sig::Error { msg: "ROOM FULL".into() });
                    continue;
                }
                let peer = room.next_peer;
                room.next_peer += 1;
                room.peers.insert(peer, tx.clone());
                send(&tx, &Sig::Joined { peer });
                send(&room.host, &Sig::Peer { peer });
                eprintln!("room {code}: peer {peer} joined");
                role = Some(Role::Joiner { code, peer });
            }
            (Some(Role::Host { code }), Sig::Signal { to: Some(to), data, .. }) => {
                if let Some(peer_tx) = rooms_guard.0.get(code).and_then(|r| r.peers.get(&to)) {
                    send(peer_tx, &Sig::Signal { to: None, from: Some(0), data });
                }
            }
            (Some(Role::Joiner { code, peer }), Sig::Signal { data, .. }) => {
                if let Some(room) = rooms_guard.0.get(code) {
                    send(&room.host, &Sig::Signal { to: None, from: Some(*peer), data });
                }
            }
            (Some(_), Sig::Create | Sig::Join { .. }) => {
                send(&tx, &Sig::Error { msg: "ALREADY IN A ROOM".into() });
            }
            _ => {
                // Signal before joining, or a server→client tag echoed back.
                send(&tx, &Sig::Error { msg: "UNEXPECTED MESSAGE".into() });
            }
        }
    }

    // Socket gone — tidy the room.
    let mut rooms_guard = rooms.lock().unwrap_or_else(|e| e.into_inner());
    match role {
        Some(Role::Host { code }) => {
            if let Some(room) = rooms_guard.0.remove(&code) {
                for peer_tx in room.peers.values() {
                    send(peer_tx, &Sig::Error { msg: "HOST LEFT".into() });
                }
            }
            eprintln!("room {code} closed");
        }
        Some(Role::Joiner { code, peer }) => {
            if let Some(room) = rooms_guard.0.get_mut(&code) {
                room.peers.remove(&peer);
                send(&room.host, &Sig::PeerLeft { peer });
            }
            eprintln!("room {code}: peer {peer} left");
        }
        None => {}
    }
    drop(rooms_guard);
    writer.abort();
}

async fn ws_handler(ws: WebSocketUpgrade, State(rooms): State<Shared>) -> impl IntoResponse {
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, rooms))
}

#[tokio::main]
async fn main() {
    let mut listen: SocketAddr = "0.0.0.0:8000".parse().unwrap();
    let mut web = PathBuf::from("web");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().expect("--listen ADDR").parse().expect("bad --listen"),
            "--web" => web = PathBuf::from(args.next().expect("--web DIR")),
            other => {
                eprintln!("unknown flag {other}\nusage: signaling [--listen ADDR:PORT] [--web DIR]");
                std::process::exit(2);
            }
        }
    }

    // Plain static serving — `/` gets index.html, anything missing is a
    // 404.  No SPA-style index fallback: bevy_asset probes `<asset>.meta`
    // files and treats a 200 with HTML as a corrupt meta file, which silently
    // kills the font.  `.br` / `.gz` sidecars (written by build-web.sh) are
    // served in place of the 21 MB .wasm when the browser accepts them.
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(Shared::default())
        .fallback_service(ServeDir::new(&web).precompressed_br().precompressed_gzip());

    eprintln!("signaling on http://{listen}/  (serving {})", web.display());
    let listener = tokio::net::TcpListener::bind(listen).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}

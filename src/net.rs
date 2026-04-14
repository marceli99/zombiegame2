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
pub const MAX_MSG_SIZE: usize = 1_048_576;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ClientMsg {
    Hello,
    Input(NetInput),
    Leave,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ServerMsg {
    Welcome { your_id: u8 },
    LobbyState { players: Vec<u8> },
    StartGame,
    Snapshot(Box<NetSnapshot>),
    FullLobby,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct NetInput {
    pub move_x: f32,
    pub move_y: f32,
    pub aim_x: f32,
    pub aim_y: f32,
    pub shoot: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NetSnapshot {
    pub tick: u64,
    pub players: Vec<NetPlayerState>,
    pub zombies: Vec<NetZombieState>,
    pub bullets: Vec<NetBulletState>,
    pub score: u32,
    pub wave: u32,
    pub in_break: bool,
    pub break_secs: f32,
    pub zombies_to_spawn: u32,
    pub game_over: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct NetPlayerState {
    pub id: u8,
    pub x: f32,
    pub y: f32,
    pub rot: f32,
    pub hp: i32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct NetZombieState {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub rot: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct NetBulletState {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub rot: f32,
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
    Disconnected { id: u8 },
    Input { id: u8, input: NetInput },
}

pub enum ClientInEvent {
    Welcomed { your_id: u8 },
    LobbyState { players: Vec<u8> },
    Started,
    Snapshot(Box<NetSnapshot>),
    Disconnected,
    FullLobby,
}

pub struct HostConn {
    pub events: Arc<Mutex<Receiver<ServerEvent>>>,
    pub senders: Arc<Mutex<HashMap<u8, Sender<ServerMsg>>>>,
    pub shutdown: Arc<AtomicBool>,
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
    pub fn reset_alloc(&mut self) {
        self.next_zombie_net_id = 0;
        self.next_bullet_net_id = 0;
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

#[derive(Resource, Default)]
pub struct NetEntities {
    pub players: HashMap<u8, Entity>,
    pub zombies: HashMap<u32, Entity>,
    pub bullets: HashMap<u32, Entity>,
}

impl NetEntities {
    pub fn clear(&mut self) {
        self.players.clear();
        self.zombies.clear();
        self.bullets.clear();
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
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_MSG_SIZE {
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
    let senders: Arc<Mutex<HashMap<u8, Sender<ServerMsg>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let shutdown = Arc::new(AtomicBool::new(false));

    let senders_clone = senders.clone();
    let event_tx_clone = event_tx.clone();
    let shutdown_clone = shutdown.clone();
    thread::spawn(move || {
        let mut next_id: u8 = 1;
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

            let current_count = senders_clone.lock().unwrap().len();
            if current_count >= (MAX_PLAYERS - 1) as usize {
                let mut s = stream;
                let _ = write_msg(&mut s, &ServerMsg::FullLobby);
                let _ = s.shutdown(std::net::Shutdown::Both);
                continue;
            }

            let id = next_id;
            next_id = next_id.wrapping_add(1);

            let mut welcome_stream = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            };
            if write_msg(&mut welcome_stream, &ServerMsg::Welcome { your_id: id }).is_err() {
                continue;
            }

            let (out_tx, out_rx) = channel::<ServerMsg>();
            senders_clone.lock().unwrap().insert(id, out_tx);
            let _ = event_tx_clone.send(ServerEvent::Connected { id });

            let mut writer_stream = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            };
            thread::spawn(move || {
                while let Ok(msg) = out_rx.recv() {
                    if write_msg(&mut writer_stream, &msg).is_err() {
                        break;
                    }
                }
                let _ = writer_stream.shutdown(std::net::Shutdown::Both);
            });

            let reader_event_tx = event_tx_clone.clone();
            let reader_senders = senders_clone.clone();
            let mut reader_stream = stream;
            thread::spawn(move || loop {
                match read_msg::<ClientMsg>(&mut reader_stream) {
                    Ok(ClientMsg::Input(input)) => {
                        if reader_event_tx
                            .send(ServerEvent::Input { id, input })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(ClientMsg::Hello) => {}
                    Ok(ClientMsg::Leave) => {
                        reader_senders.lock().unwrap().remove(&id);
                        let _ = reader_event_tx.send(ServerEvent::Disconnected { id });
                        break;
                    }
                    Err(_) => {
                        reader_senders.lock().unwrap().remove(&id);
                        let _ = reader_event_tx.send(ServerEvent::Disconnected { id });
                        break;
                    }
                }
            });
        }
    });

    Ok(HostConn {
        events: Arc::new(Mutex::new(event_rx)),
        senders,
        shutdown,
    })
}

pub fn start_client(addr: SocketAddr) -> std::io::Result<ClientConn> {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_nodelay(true)?;

    let (event_tx, event_rx) = channel::<ClientInEvent>();
    let (send_tx, send_rx) = channel::<ClientMsg>();

    let mut hello_stream = stream.try_clone()?;
    write_msg(&mut hello_stream, &ClientMsg::Hello)?;

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
            Ok(ServerMsg::Welcome { your_id }) => {
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
            Ok(ServerMsg::Snapshot(snap)) => {
                if reader_event_tx.send(ClientInEvent::Snapshot(snap)).is_err() {
                    break;
                }
            }
            Ok(ServerMsg::FullLobby) => {
                let _ = reader_event_tx.send(ClientInEvent::FullLobby);
                break;
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
    let senders = host.senders.lock().unwrap();
    for tx in senders.values() {
        let _ = tx.send(msg.clone());
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
            .init_resource::<NetEntities>();
    }
}

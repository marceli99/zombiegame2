use bevy::prelude::*;

use crate::menu::{
    panel_bundle, spawn_background, spawn_divider, spawn_title_block, MenuError, BG_COLOR,
    TEXT_DIM, TEXT_HIGHLIGHT, TEXT_NORMAL, TEXT_SUBTITLE,
};
use crate::net::{
    broadcast, ClientInEvent, NetContext, NetMode, PlayerNicknames, send_to, ServerEvent,
    ServerMsg,
};
use crate::{GameState, UiAssets};

#[derive(Component)]
pub struct LobbyRoot;

#[derive(Component)]
pub struct LobbyPlayerList;

#[derive(Component)]
pub struct LobbyStatusText;

/// "ROOM CODE: XXXX" — what the host reads out to friends.
#[derive(Component)]
pub struct LobbyRoomText;

/// Pre-start countdown shared by host and client.  Host owns the
/// authoritative timer; client mirrors it from broadcast events purely
/// for the on-screen "STARTING IN N..." readout.
const LOBBY_COUNTDOWN_SECONDS: f32 = 3.0;

#[derive(Resource, Default)]
pub struct LobbyCountdown {
    /// Remaining time in seconds, or `None` when no countdown is running.
    pub remaining: Option<f32>,
}

/// Transient on-screen feedback for player join / leave events.  Both host
/// and client populate it locally by diffing `lobby_players` against the
/// previous frame's list, so each side gets feedback for any change they
/// observe — even if nickname Hello hasn't arrived yet.
#[derive(Resource, Default)]
pub struct LobbyToast {
    pub text: String,
    pub remaining: f32,
}

const LOBBY_TOAST_DURATION: f32 = 2.5;

/// Previous frame's roster baseline for `track_lobby_changes`.  A Resource
/// rather than a `Local` so `OnExit(GameState::Lobby)` can clear it — a
/// `Local` survives leaving the lobby, and the next session's first frame
/// would diff the fresh roster against the stale one, flashing a bogus
/// JOIN/LEFT toast.
#[derive(Resource, Default)]
struct PrevLobbyPlayers(Option<Vec<u8>>);

#[derive(Component)]
pub struct LobbyToastText;

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LobbyCountdown>()
            .init_resource::<LobbyToast>()
            .init_resource::<PrevLobbyPlayers>()
            .add_systems(OnEnter(GameState::Lobby), spawn_lobby)
            .add_systems(
                OnExit(GameState::Lobby),
                (despawn_lobby, reset_countdown, reset_toast, reset_roster_diff),
            )
            .add_systems(
                Update,
                (
                    poll_host_lobby_events,
                    poll_client_lobby_events,
                    lobby_input,
                    tick_lobby_countdown,
                    track_lobby_changes,
                    tick_lobby_toast,
                    update_lobby_ui,
                )
                    .chain()
                    .run_if(in_state(GameState::Lobby)),
            );
    }
}

fn reset_countdown(mut countdown: ResMut<LobbyCountdown>) {
    countdown.remaining = None;
}

fn reset_toast(mut toast: ResMut<LobbyToast>) {
    *toast = LobbyToast::default();
}

fn reset_roster_diff(mut prev_players: ResMut<PrevLobbyPlayers>) {
    prev_players.0 = None;
}

/// Diff `lobby_players` against the previous frame and surface a brief
/// on-screen toast for any join / leave.  Runs on both host and client —
/// each side sees the same delta because the host syncs the list to all
/// clients via `LobbyState` immediately after a Connected/Disconnected event.
fn track_lobby_changes(
    ctx: Res<NetContext>,
    nicknames: Res<PlayerNicknames>,
    mut toast: ResMut<LobbyToast>,
    mut prev_players: ResMut<PrevLobbyPlayers>,
) {
    let current = ctx.lobby_players.clone();
    let Some(prev) = prev_players.0.as_ref() else {
        // First observation this session — record but don't toast (would
        // fire on lobby entry for ourselves).  An empty list is a client
        // still waiting for its first LobbyState; keep waiting so the real
        // roster becomes the baseline instead of toasting the whole lobby.
        if !current.is_empty() {
            prev_players.0 = Some(current);
        }
        return;
    };
    let joined: Vec<u8> = current.iter().copied().filter(|id| !prev.contains(id)).collect();
    let left: Vec<u8> = prev.iter().copied().filter(|id| !current.contains(id)).collect();
    if let Some(&id) = joined.first() {
        let name = nicknames
            .0
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("P{id}"));
        toast.text = format!("{name} JOINED");
        toast.remaining = LOBBY_TOAST_DURATION;
    } else if let Some(&id) = left.first() {
        let name = nicknames
            .0
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("P{id}"));
        toast.text = format!("{name} LEFT");
        toast.remaining = LOBBY_TOAST_DURATION;
    }
    prev_players.0 = Some(current);
}

fn tick_lobby_toast(time: Res<Time>, mut toast: ResMut<LobbyToast>) {
    if toast.remaining > 0.0 {
        toast.remaining -= time.delta_seconds();
        if toast.remaining <= 0.0 {
            toast.remaining = 0.0;
            toast.text.clear();
        }
    }
}

/// Same framed panel as the main menu, so the flow menu → lobby → game
/// doesn't jump between two visual languages.
fn spawn_lobby(mut commands: Commands, assets: Res<UiAssets>, net: Res<NetMode>) {
    let font = assets.font.clone();
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                background_color: BackgroundColor(BG_COLOR),
                ..default()
            },
            LobbyRoot,
        ))
        .with_children(|root| {
            spawn_background(root, &assets);
            root.spawn(panel_bundle()).with_children(|panel| {
                spawn_title_block(panel, &font, "LOBBY");
                panel.spawn(TextBundle::from_section(
                    match *net {
                        NetMode::Host => "YOU ARE HOSTING",
                        NetMode::Client => "JOINED A ROOM",
                        _ => "",
                    },
                    TextStyle {
                        font: font.clone(),
                        font_size: 16.0,
                        color: TEXT_SUBTITLE,
                    },
                ));
                spawn_divider(panel);
                panel.spawn((
                    TextBundle::from_section(
                        "",
                        TextStyle {
                            font: font.clone(),
                            font_size: 24.0,
                            color: TEXT_HIGHLIGHT,
                        },
                    )
                    .with_style(Style {
                        min_height: Val::Px(30.0),
                        ..default()
                    }),
                    LobbyRoomText,
                ));
                panel.spawn((
                    TextBundle::from_section(
                        "PLAYERS: 1/4",
                        TextStyle {
                            font: font.clone(),
                            font_size: 16.0,
                            color: TEXT_NORMAL,
                        },
                    )
                    .with_style(Style {
                        margin: UiRect::top(Val::Px(12.0)),
                        ..default()
                    }),
                    LobbyPlayerList,
                ));
                panel.spawn((
                    TextBundle::from_section(
                        "",
                        TextStyle {
                            font: font.clone(),
                            font_size: 16.0,
                            color: TEXT_HIGHLIGHT,
                        },
                    )
                    .with_style(Style {
                        margin: UiRect::top(Val::Px(18.0)),
                        min_height: Val::Px(18.0),
                        ..default()
                    }),
                    LobbyStatusText,
                ));
                panel.spawn((
                    TextBundle::from_section(
                        "",
                        TextStyle {
                            font: font.clone(),
                            font_size: 16.0,
                            color: Color::srgba(0.6, 0.95, 0.6, 0.0),
                        },
                    )
                    .with_style(Style {
                        min_height: Val::Px(16.0),
                        ..default()
                    }),
                    LobbyToastText,
                ));
                spawn_divider(panel);
                panel.spawn(TextBundle::from_section(
                    match *net {
                        NetMode::Host => "ENTER - START   ESC - BACK",
                        _ => "ESC - LEAVE ROOM",
                    },
                    TextStyle {
                        font,
                        font_size: 16.0,
                        color: TEXT_DIM,
                    },
                ));
            });
        });
}

fn despawn_lobby(mut commands: Commands, q: Query<Entity, With<LobbyRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

#[allow(clippy::too_many_arguments)]
fn poll_host_lobby_events(
    mut ctx: ResMut<NetContext>,
    mut net: ResMut<NetMode>,
    mut nicknames: ResMut<PlayerNicknames>,
    countdown: Res<LobbyCountdown>,
    mut next_state: ResMut<NextState<GameState>>,
    mut menu_error: ResMut<MenuError>,
) {
    if *net != NetMode::Host {
        return;
    }
    // Drain the events channel into a Vec first so we don't hold a borrow on
    // `ctx.host` while iterating — the match arms below need to mutate
    // `ctx.lobby_players` and re-borrow `ctx.host` for the broadcast.
    let Some(events_arc) = ctx.host.as_ref().map(|h| h.events.clone()) else {
        return;
    };
    let mut new_events = Vec::new();
    if let Ok(rx) = events_arc.lock() {
        while let Ok(e) = rx.try_recv() {
            new_events.push(e);
        }
    }
    for e in new_events {
        match e {
            ServerEvent::Connected { id } => {
                if !ctx.lobby_players.contains(&id) {
                    ctx.lobby_players.push(id);
                }
                let players = ctx.lobby_players.clone();
                if let Some(host) = ctx.host.as_ref() {
                    broadcast(host, &ServerMsg::LobbyState { players });
                    // A joiner arriving mid-countdown missed the original
                    // CountdownStart broadcast — without this they'd sit on
                    // "WAITING FOR HOST..." and snap straight into gameplay
                    // when Started lands.  Targeted send so existing clients'
                    // already-ticking mirrors stay untouched.
                    if let Some(remaining) = countdown.remaining {
                        send_to(
                            host,
                            id,
                            &ServerMsg::CountdownStart {
                                seconds: remaining.ceil() as u8,
                            },
                        );
                    }
                }
            }
            ServerEvent::Disconnected { id } => {
                ctx.lobby_players.retain(|p| *p != id);
                nicknames.0.remove(&id);
                let players = ctx.lobby_players.clone();
                if let Some(host) = ctx.host.as_ref() {
                    broadcast(host, &ServerMsg::LobbyState { players });
                }
            }
            ServerEvent::Hello { id, nickname } => {
                nicknames.0.insert(id, nickname);
            }
            ServerEvent::RoomCode { code } => {
                ctx.room_code = code;
            }
            ServerEvent::HostError { reason } => {
                // No room to host in — back to the menu with the reason.
                menu_error.0 = reason;
                ctx.disconnect();
                *net = NetMode::SinglePlayer;
                next_state.set(GameState::Menu);
                return;
            }
            ServerEvent::Input { .. } | ServerEvent::ChatRelay { .. } => {}
        }
    }
}

fn poll_client_lobby_events(
    mut ctx: ResMut<NetContext>,
    mut next_state: ResMut<NextState<GameState>>,
    mut mode: ResMut<NetMode>,
    mut countdown: ResMut<LobbyCountdown>,
    mut menu_error: ResMut<MenuError>,
) {
    if *mode != NetMode::Client {
        return;
    }
    let Some(client) = ctx.client.as_ref() else {
        return;
    };
    let events_arc = client.events.clone();
    let mut new_events = Vec::new();
    if let Ok(rx) = events_arc.lock() {
        while let Ok(e) = rx.try_recv() {
            // Stop at `Started`: whatever follows (the first snapshots, whose
            // delta fields carry the full pickup/nickname state) belongs to
            // the in-game receiver, not the lobby's bit bucket.
            let started = matches!(e, ClientInEvent::Started);
            new_events.push(e);
            if started {
                break;
            }
        }
    }
    for e in new_events {
        match e {
            ClientInEvent::Welcomed { your_id } => {
                ctx.my_id = your_id;
            }
            ClientInEvent::LobbyState { players } => {
                ctx.lobby_players = players;
            }
            ClientInEvent::Started => {
                countdown.remaining = None;
                next_state.set(GameState::Playing);
            }
            ClientInEvent::CountdownStart { seconds } => {
                countdown.remaining = Some(seconds as f32);
            }
            ClientInEvent::CountdownCancel => {
                countdown.remaining = None;
            }
            ClientInEvent::Snapshot(_) | ClientInEvent::Chat { .. } => {}
            ClientInEvent::ConnectFailed { reason } => {
                menu_error.0 = reason;
                ctx.disconnect();
                *mode = NetMode::SinglePlayer;
                next_state.set(GameState::Menu);
            }
            ClientInEvent::Disconnected
            | ClientInEvent::FullLobby
            | ClientInEvent::GameInProgress
            | ClientInEvent::ProtocolMismatch { .. } => {
                // Disconnect, full lobby, mid-game rejection, or protocol
                // mismatch all funnel back to the menu — the lobby can't
                // recover from any of them.
                menu_error.0 = match e {
                    ClientInEvent::FullLobby => "ROOM FULL".into(),
                    ClientInEvent::GameInProgress => "GAME ALREADY RUNNING".into(),
                    ClientInEvent::ProtocolMismatch { .. } => "VERSION MISMATCH".into(),
                    _ => "DISCONNECTED".into(),
                };
                ctx.disconnect();
                *mode = NetMode::SinglePlayer;
                next_state.set(GameState::Menu);
            }
        }
    }
}

fn lobby_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut ctx: ResMut<NetContext>,
    mut mode: ResMut<NetMode>,
    mut next_state: ResMut<NextState<GameState>>,
    mut countdown: ResMut<LobbyCountdown>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        // Esc cancels a running countdown first; second Esc leaves the lobby.
        if countdown.remaining.is_some() {
            countdown.remaining = None;
            if *mode == NetMode::Host {
                if let Some(host) = ctx.host.as_ref() {
                    broadcast(host, &ServerMsg::CountdownCancel);
                }
            }
            return;
        }
        ctx.disconnect();
        *mode = NetMode::SinglePlayer;
        next_state.set(GameState::Menu);
        return;
    }
    if *mode == NetMode::Host && keys.just_pressed(KeyCode::Enter) {
        match countdown.remaining {
            None => {
                // First press: kick off the countdown so latecomers and
                // mid-config clients see a 3 s buffer before the world spawns.
                countdown.remaining = Some(LOBBY_COUNTDOWN_SECONDS);
                if let Some(host) = ctx.host.as_ref() {
                    broadcast(
                        host,
                        &ServerMsg::CountdownStart {
                            seconds: LOBBY_COUNTDOWN_SECONDS as u8,
                        },
                    );
                }
            }
            Some(_) => {
                // Second press: cancel — host changed their mind.
                countdown.remaining = None;
                if let Some(host) = ctx.host.as_ref() {
                    broadcast(host, &ServerMsg::CountdownCancel);
                }
            }
        }
    }
}

/// Tick the local countdown.  Host fires the actual start; client just
/// counts down for the visible UI (server's `Started` event is what
/// transitions client into gameplay).
fn tick_lobby_countdown(
    time: Res<Time>,
    mut countdown: ResMut<LobbyCountdown>,
    mode: Res<NetMode>,
    ctx: Res<NetContext>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let Some(remaining) = countdown.remaining.as_mut() else {
        return;
    };
    *remaining -= time.delta_seconds();
    if *remaining <= 0.0 {
        countdown.remaining = None;
        if *mode == NetMode::Host {
            if let Some(host) = ctx.host.as_ref() {
                broadcast(host, &ServerMsg::StartGame);
            }
            next_state.set(GameState::Playing);
        }
    }
}

#[allow(clippy::type_complexity)]
fn update_lobby_ui(
    ctx: Res<NetContext>,
    net: Res<NetMode>,
    countdown: Res<LobbyCountdown>,
    toast: Res<LobbyToast>,
    mut list: Query<
        &mut Text,
        (
            With<LobbyPlayerList>,
            Without<LobbyStatusText>,
            Without<LobbyToastText>,
        ),
    >,
    mut status: Query<
        &mut Text,
        (
            With<LobbyStatusText>,
            Without<LobbyPlayerList>,
            Without<LobbyToastText>,
        ),
    >,
    mut toast_text: Query<
        &mut Text,
        (
            With<LobbyToastText>,
            Without<LobbyPlayerList>,
            Without<LobbyStatusText>,
        ),
    >,
    mut room_text: Query<
        &mut Text,
        (
            With<LobbyRoomText>,
            Without<LobbyPlayerList>,
            Without<LobbyStatusText>,
            Without<LobbyToastText>,
        ),
    >,
) {
    if let Ok(mut text) = room_text.get_single_mut() {
        text.sections[0].value = if ctx.room_code.is_empty() {
            match *net {
                NetMode::Host => "CREATING ROOM...".to_string(),
                _ => String::new(),
            }
        } else {
            format!("ROOM CODE: {}", ctx.room_code)
        };
    }
    if let Ok(mut text) = list.get_single_mut() {
        let count = ctx.lobby_players.len();
        text.sections[0].value = format!("PLAYERS: {count}/4");
    }
    if let Ok(mut text) = status.get_single_mut() {
        text.sections[0].value = if let Some(remaining) = countdown.remaining {
            // Ceil so the user sees "3" for the first ~half of the first
            // second instead of jumping straight to "2".
            let secs = remaining.ceil() as u32;
            match *net {
                NetMode::Host => format!("STARTING IN {secs}... (ENTER/ESC TO CANCEL)"),
                NetMode::Client => format!("STARTING IN {secs}..."),
                _ => String::new(),
            }
        } else {
            match *net {
                NetMode::Host if ctx.lobby_players.len() < 2 => "WAITING FOR PLAYERS...".to_string(),
                NetMode::Host => "READY - PRESS ENTER TO START".to_string(),
                // No roster yet means the WebRTC handshake is still running.
                NetMode::Client if ctx.lobby_players.is_empty() => "CONNECTING...".to_string(),
                NetMode::Client => "WAITING FOR HOST...".to_string(),
                _ => String::new(),
            }
        };
    }
    if let Ok(mut text) = toast_text.get_single_mut() {
        text.sections[0].value = toast.text.clone();
        // Fade over the last 0.6 s of the toast lifetime.
        let alpha = (toast.remaining / 0.6).clamp(0.0, 1.0);
        text.sections[0].style.color = Color::srgba(0.6, 0.95, 0.6, alpha);
    }
}

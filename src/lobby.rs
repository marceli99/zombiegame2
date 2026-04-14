use bevy::prelude::*;

use crate::net::{broadcast, ClientInEvent, NetContext, NetMode, ServerEvent, ServerMsg};
use crate::{GameState, UiAssets};

#[derive(Component)]
pub struct LobbyRoot;

#[derive(Component)]
pub struct LobbyPlayerList;

#[derive(Component)]
pub struct LobbyStatusText;

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Lobby), spawn_lobby)
            .add_systems(OnExit(GameState::Lobby), despawn_lobby)
            .add_systems(
                Update,
                (
                    poll_host_lobby_events,
                    poll_client_lobby_events,
                    lobby_input,
                    update_lobby_ui,
                )
                    .chain()
                    .run_if(in_state(GameState::Lobby)),
            );
    }
}

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
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(18.0),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgb(0.05, 0.06, 0.08)),
                ..default()
            },
            LobbyRoot,
        ))
        .with_children(|parent| {
            let title = match *net {
                NetMode::Host => "LOBBY - HOST",
                NetMode::Client => "LOBBY - GRACZ",
                _ => "LOBBY",
            };
            parent.spawn(TextBundle::from_section(
                title,
                TextStyle {
                    font: font.clone(),
                    font_size: 48.0,
                    color: Color::srgb(0.85, 0.15, 0.15),
                },
            ));
            parent.spawn((
                TextBundle::from_section(
                    "GRACZE: 1/4",
                    TextStyle {
                        font: font.clone(),
                        font_size: 22.0,
                        color: Color::srgb(0.9, 0.9, 0.9),
                    },
                )
                .with_style(Style {
                    margin: UiRect::top(Val::Px(20.0)),
                    ..default()
                }),
                LobbyPlayerList,
            ));
            parent.spawn((
                TextBundle::from_section(
                    "",
                    TextStyle {
                        font: font.clone(),
                        font_size: 16.0,
                        color: Color::srgb(1.0, 0.82, 0.2),
                    },
                )
                .with_style(Style {
                    margin: UiRect::top(Val::Px(26.0)),
                    ..default()
                }),
                LobbyStatusText,
            ));
            parent.spawn(
                TextBundle::from_section(
                    "ESC - powrot do menu",
                    TextStyle {
                        font,
                        font_size: 11.0,
                        color: Color::srgb(0.5, 0.5, 0.5),
                    },
                )
                .with_style(Style {
                    margin: UiRect::top(Val::Px(60.0)),
                    ..default()
                }),
            );
        });
}

fn despawn_lobby(mut commands: Commands, q: Query<Entity, With<LobbyRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn poll_host_lobby_events(mut ctx: ResMut<NetContext>, net: Res<NetMode>) {
    if *net != NetMode::Host {
        return;
    }
    let Some(host) = ctx.host.as_ref() else {
        return;
    };
    let events_arc = host.events.clone();
    let senders_arc = host.senders.clone();

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
                let senders = senders_arc.lock().unwrap();
                for tx in senders.values() {
                    let _ = tx.send(ServerMsg::LobbyState {
                        players: players.clone(),
                    });
                }
            }
            ServerEvent::Disconnected { id } => {
                ctx.lobby_players.retain(|p| *p != id);
                let players = ctx.lobby_players.clone();
                let senders = senders_arc.lock().unwrap();
                for tx in senders.values() {
                    let _ = tx.send(ServerMsg::LobbyState {
                        players: players.clone(),
                    });
                }
            }
            ServerEvent::Input { .. } => {}
        }
    }
}

fn poll_client_lobby_events(
    mut ctx: ResMut<NetContext>,
    mut next_state: ResMut<NextState<GameState>>,
    mut mode: ResMut<NetMode>,
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
            new_events.push(e);
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
                next_state.set(GameState::Playing);
            }
            ClientInEvent::Snapshot(_) => {}
            ClientInEvent::Disconnected | ClientInEvent::FullLobby => {
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
) {
    if keys.just_pressed(KeyCode::Escape) {
        ctx.disconnect();
        *mode = NetMode::SinglePlayer;
        next_state.set(GameState::Menu);
        return;
    }
    if *mode == NetMode::Host && keys.just_pressed(KeyCode::Enter) {
        if let Some(host) = ctx.host.as_ref() {
            broadcast(host, &ServerMsg::StartGame);
        }
        next_state.set(GameState::Playing);
    }
}

fn update_lobby_ui(
    ctx: Res<NetContext>,
    net: Res<NetMode>,
    mut list: Query<&mut Text, (With<LobbyPlayerList>, Without<LobbyStatusText>)>,
    mut status: Query<&mut Text, (With<LobbyStatusText>, Without<LobbyPlayerList>)>,
) {
    if let Ok(mut text) = list.get_single_mut() {
        let count = ctx.lobby_players.len();
        text.sections[0].value = format!("GRACZE: {count}/4");
    }
    if let Ok(mut text) = status.get_single_mut() {
        text.sections[0].value = match *net {
            NetMode::Host => "ENTER - rozpocznij gre".to_string(),
            NetMode::Client => "CZEKANIE NA HOSTA...".to_string(),
            _ => String::new(),
        };
    }
}

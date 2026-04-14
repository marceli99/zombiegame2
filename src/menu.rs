use bevy::app::AppExit;
use bevy::prelude::*;
use std::net::{IpAddr, SocketAddr};

use crate::net::{start_client, start_host, NetContext, NetMode, NET_PORT};
use crate::{GameState, UiAssets};

#[derive(Component)]
pub struct MenuRoot;

#[derive(Component)]
pub struct MenuItem {
    pub index: usize,
}

#[derive(Component)]
pub struct MenuErrorText;

#[derive(Component)]
pub struct JoinPromptRoot;

#[derive(Component)]
pub struct JoinPromptIpText;

#[derive(Component)]
pub struct JoinPromptErrorText;

#[derive(Resource, Default)]
pub struct MenuSelection(pub usize);

#[derive(Resource)]
pub struct MenuError(pub String);

impl Default for MenuError {
    fn default() -> Self {
        Self(String::new())
    }
}

#[derive(Resource)]
pub struct JoinAddress {
    pub text: String,
    pub error: String,
}

impl Default for JoinAddress {
    fn default() -> Self {
        Self {
            text: "127.0.0.1".to_string(),
            error: String::new(),
        }
    }
}

const ITEMS: [&str; 4] = ["SINGLE PLAYER", "HOST LAN", "DOLACZ LAN", "WYJSCIE"];

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuSelection>()
            .init_resource::<MenuError>()
            .init_resource::<JoinAddress>()
            .add_systems(OnEnter(GameState::Menu), spawn_menu)
            .add_systems(OnExit(GameState::Menu), despawn_menu)
            .add_systems(
                Update,
                (menu_navigate, menu_highlight, update_menu_error)
                    .run_if(in_state(GameState::Menu)),
            )
            .add_systems(OnEnter(GameState::JoinPrompt), spawn_join_prompt)
            .add_systems(OnExit(GameState::JoinPrompt), despawn_join_prompt)
            .add_systems(
                Update,
                join_prompt_input.run_if(in_state(GameState::JoinPrompt)),
            );
    }
}

fn spawn_menu(
    mut commands: Commands,
    mut selection: ResMut<MenuSelection>,
    assets: Res<UiAssets>,
) {
    selection.0 = 0;
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
            MenuRoot,
        ))
        .with_children(|parent| {
            parent.spawn(TextBundle::from_section(
                "ZOMBIAKI",
                TextStyle {
                    font: font.clone(),
                    font_size: 80.0,
                    color: Color::srgb(0.85, 0.15, 0.15),
                },
            ));
            parent.spawn(
                TextBundle::from_section(
                    "FALE  PRZETRWANIA",
                    TextStyle {
                        font: font.clone(),
                        font_size: 20.0,
                        color: Color::srgb(0.6, 0.6, 0.55),
                    },
                )
                .with_style(Style {
                    margin: UiRect::bottom(Val::Px(34.0)),
                    ..default()
                }),
            );
            for (i, label) in ITEMS.iter().enumerate() {
                parent.spawn((
                    TextBundle::from_section(
                        *label,
                        TextStyle {
                            font: font.clone(),
                            font_size: 24.0,
                            color: Color::srgb(0.7, 0.7, 0.7),
                        },
                    ),
                    MenuItem { index: i },
                ));
            }
            parent.spawn((
                TextBundle::from_section(
                    "",
                    TextStyle {
                        font: font.clone(),
                        font_size: 12.0,
                        color: Color::srgb(0.9, 0.35, 0.35),
                    },
                )
                .with_style(Style {
                    margin: UiRect::top(Val::Px(18.0)),
                    ..default()
                }),
                MenuErrorText,
            ));
            parent.spawn(
                TextBundle::from_section(
                    "Strzalki - wybor   ENTER - zatwierdz",
                    TextStyle {
                        font,
                        font_size: 11.0,
                        color: Color::srgb(0.45, 0.45, 0.45),
                    },
                )
                .with_style(Style {
                    margin: UiRect::top(Val::Px(28.0)),
                    ..default()
                }),
            );
        });
}

fn despawn_menu(mut commands: Commands, q: Query<Entity, With<MenuRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn menu_navigate(
    keys: Res<ButtonInput<KeyCode>>,
    mut selection: ResMut<MenuSelection>,
    mut next_state: ResMut<NextState<GameState>>,
    mut exit: EventWriter<AppExit>,
    mut ctx: ResMut<NetContext>,
    mut net_mode: ResMut<NetMode>,
    mut error: ResMut<MenuError>,
) {
    let up = keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW);
    let down = keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS);
    if up {
        selection.0 = (selection.0 + ITEMS.len() - 1) % ITEMS.len();
    }
    if down {
        selection.0 = (selection.0 + 1) % ITEMS.len();
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        match selection.0 {
            0 => {
                ctx.disconnect();
                *net_mode = NetMode::SinglePlayer;
                ctx.my_id = 0;
                ctx.lobby_players = vec![0];
                next_state.set(GameState::Playing);
            }
            1 => match start_host() {
                Ok(host) => {
                    ctx.disconnect();
                    ctx.host = Some(host);
                    ctx.my_id = 0;
                    ctx.lobby_players = vec![0];
                    *net_mode = NetMode::Host;
                    error.0.clear();
                    next_state.set(GameState::Lobby);
                }
                Err(e) => {
                    error.0 = format!("Host fail: {e}");
                }
            },
            2 => {
                error.0.clear();
                next_state.set(GameState::JoinPrompt);
            }
            3 => {
                exit.send(AppExit::Success);
            }
            _ => {}
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        exit.send(AppExit::Success);
    }
}

fn menu_highlight(selection: Res<MenuSelection>, mut items: Query<(&MenuItem, &mut Text)>) {
    if !selection.is_changed() && !selection.is_added() {
        return;
    }
    for (item, mut text) in &mut items {
        let selected = item.index == selection.0;
        let prefix = if selected { "> " } else { "  " };
        let label = ITEMS[item.index];
        text.sections[0].value = format!("{prefix}{label}");
        text.sections[0].style.color = if selected {
            Color::srgb(1.0, 0.85, 0.3)
        } else {
            Color::srgb(0.6, 0.6, 0.6)
        };
    }
}

fn update_menu_error(error: Res<MenuError>, mut q: Query<&mut Text, With<MenuErrorText>>) {
    if !error.is_changed() {
        return;
    }
    if let Ok(mut text) = q.get_single_mut() {
        text.sections[0].value = error.0.clone();
    }
}

fn spawn_join_prompt(mut commands: Commands, assets: Res<UiAssets>, addr: Res<JoinAddress>) {
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
                    row_gap: Val::Px(20.0),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgb(0.05, 0.06, 0.08)),
                ..default()
            },
            JoinPromptRoot,
        ))
        .with_children(|parent| {
            parent.spawn(TextBundle::from_section(
                "DOLACZ LAN",
                TextStyle {
                    font: font.clone(),
                    font_size: 48.0,
                    color: Color::srgb(0.85, 0.15, 0.15),
                },
            ));
            parent.spawn(
                TextBundle::from_section(
                    "Wpisz IP hosta (cyfry i kropki):",
                    TextStyle {
                        font: font.clone(),
                        font_size: 14.0,
                        color: Color::srgb(0.7, 0.7, 0.7),
                    },
                )
                .with_style(Style {
                    margin: UiRect::top(Val::Px(14.0)),
                    ..default()
                }),
            );
            parent.spawn((
                TextBundle::from_section(
                    format!("IP: {}_", addr.text),
                    TextStyle {
                        font: font.clone(),
                        font_size: 28.0,
                        color: Color::srgb(1.0, 0.85, 0.3),
                    },
                )
                .with_style(Style {
                    margin: UiRect::top(Val::Px(8.0)),
                    ..default()
                }),
                JoinPromptIpText,
            ));
            parent.spawn((
                TextBundle::from_section(
                    "",
                    TextStyle {
                        font: font.clone(),
                        font_size: 12.0,
                        color: Color::srgb(0.9, 0.35, 0.35),
                    },
                )
                .with_style(Style {
                    margin: UiRect::top(Val::Px(10.0)),
                    ..default()
                }),
                JoinPromptErrorText,
            ));
            parent.spawn(
                TextBundle::from_section(
                    "ENTER - polacz   BACKSPACE - usun   ESC - wroc",
                    TextStyle {
                        font,
                        font_size: 10.0,
                        color: Color::srgb(0.5, 0.5, 0.5),
                    },
                )
                .with_style(Style {
                    margin: UiRect::top(Val::Px(40.0)),
                    ..default()
                }),
            );
        });
}

fn despawn_join_prompt(mut commands: Commands, q: Query<Entity, With<JoinPromptRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn keycode_to_digit(k: KeyCode) -> Option<char> {
    match k {
        KeyCode::Digit0 | KeyCode::Numpad0 => Some('0'),
        KeyCode::Digit1 | KeyCode::Numpad1 => Some('1'),
        KeyCode::Digit2 | KeyCode::Numpad2 => Some('2'),
        KeyCode::Digit3 | KeyCode::Numpad3 => Some('3'),
        KeyCode::Digit4 | KeyCode::Numpad4 => Some('4'),
        KeyCode::Digit5 | KeyCode::Numpad5 => Some('5'),
        KeyCode::Digit6 | KeyCode::Numpad6 => Some('6'),
        KeyCode::Digit7 | KeyCode::Numpad7 => Some('7'),
        KeyCode::Digit8 | KeyCode::Numpad8 => Some('8'),
        KeyCode::Digit9 | KeyCode::Numpad9 => Some('9'),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn join_prompt_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut addr: ResMut<JoinAddress>,
    mut ctx: ResMut<NetContext>,
    mut net_mode: ResMut<NetMode>,
    mut next_state: ResMut<NextState<GameState>>,
    mut ip_text: Query<
        &mut Text,
        (With<JoinPromptIpText>, Without<JoinPromptErrorText>),
    >,
    mut err_text: Query<
        &mut Text,
        (With<JoinPromptErrorText>, Without<JoinPromptIpText>),
    >,
) {
    let mut changed = false;
    for key in keys.get_just_pressed() {
        if let Some(d) = keycode_to_digit(*key) {
            if addr.text.len() < 21 {
                addr.text.push(d);
                changed = true;
            }
        } else if matches!(key, KeyCode::Period | KeyCode::NumpadDecimal) {
            if addr.text.len() < 21 {
                addr.text.push('.');
                changed = true;
            }
        } else if *key == KeyCode::Backspace {
            addr.text.pop();
            changed = true;
        }
    }
    if changed {
        if let Ok(mut text) = ip_text.get_single_mut() {
            text.sections[0].value = format!("IP: {}_", addr.text);
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        next_state.set(GameState::Menu);
        return;
    }
    if keys.just_pressed(KeyCode::Enter) {
        let parse: Result<IpAddr, _> = addr.text.parse();
        match parse {
            Ok(ip) => {
                let sock = SocketAddr::new(ip, NET_PORT);
                match start_client(sock) {
                    Ok(client) => {
                        ctx.disconnect();
                        ctx.client = Some(client);
                        *net_mode = NetMode::Client;
                        addr.error.clear();
                        next_state.set(GameState::Lobby);
                    }
                    Err(e) => {
                        addr.error = format!("Blad: {e}");
                        if let Ok(mut t) = err_text.get_single_mut() {
                            t.sections[0].value = addr.error.clone();
                        }
                    }
                }
            }
            Err(_) => {
                addr.error = "Niepoprawny IP".to_string();
                if let Ok(mut t) = err_text.get_single_mut() {
                    t.sections[0].value = addr.error.clone();
                }
            }
        }
    }
}

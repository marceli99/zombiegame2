use bevy::prelude::*;
use std::net::{IpAddr, SocketAddr};

use crate::audio::SfxEvent;
use crate::net::{start_client, start_host, NetContext, NetMode, NET_PORT};
use crate::settings::GraphicsSettings;
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

#[derive(Component)]
pub struct SettingsRoot;

#[derive(Component)]
pub struct SettingsRow {
    pub index: usize,
}

#[derive(Component)]
pub struct SettingsValueText {
    pub index: usize,
}

#[derive(Resource, Default)]
pub struct MenuSelection(pub usize);

#[derive(Resource, Default)]
pub struct SettingsSelection(pub usize);

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

const ITEMS: [&str; 5] = [
    "SINGLE PLAYER",
    "HOST LAN",
    "JOIN LAN",
    "SETTINGS",
    "QUIT",
];

const SETTINGS_ROW_COUNT: usize = 5;
const SETTINGS_LABELS: [&str; SETTINGS_ROW_COUNT] = [
    "RESOLUTION",
    "WINDOW MODE",
    "VSYNC",
    "FPS LIMIT",
    "BACK",
];

const BG_COLOR: Color = Color::srgb(0.012, 0.016, 0.022);
const PANEL_COLOR: Color = Color::srgba(0.035, 0.04, 0.05, 0.94);
const PANEL_BORDER: Color = Color::srgb(0.22, 0.28, 0.32);
const PANEL_BORDER_DARK: Color = Color::srgb(0.08, 0.1, 0.12);
const ACCENT: Color = Color::srgb(0.42, 0.12, 0.08);
const ACCENT_DIM: Color = Color::srgb(0.22, 0.07, 0.05);
const TITLE_SHADOW: Color = Color::srgba(0.0, 0.0, 0.0, 0.95);
const TEXT_DIM: Color = Color::srgb(0.32, 0.34, 0.38);
const TEXT_NORMAL: Color = Color::srgb(0.55, 0.58, 0.62);
const TEXT_HIGHLIGHT: Color = Color::srgb(0.82, 0.72, 0.28);
const TEXT_SUBTITLE: Color = Color::srgb(0.48, 0.36, 0.2);
const ERROR_COLOR: Color = Color::srgb(0.78, 0.24, 0.2);
const VIGNETTE_COLOR: Color = Color::srgba(0.0, 0.0, 0.0, 0.58);
const FOG_COLOR: Color = Color::srgba(0.08, 0.09, 0.11, 0.35);

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuSelection>()
            .init_resource::<SettingsSelection>()
            .init_resource::<MenuError>()
            .init_resource::<JoinAddress>()
            .add_systems(OnEnter(GameState::Menu), spawn_menu)
            .add_systems(OnExit(GameState::Menu), despawn_menu)
            .add_systems(
                Update,
                (menu_navigate, menu_highlight, update_menu_error)
                    .run_if(in_state(GameState::Menu)),
            )
            .add_systems(OnEnter(GameState::Settings), spawn_settings)
            .add_systems(OnExit(GameState::Settings), despawn_settings)
            .add_systems(
                Update,
                (settings_input, settings_refresh).run_if(in_state(GameState::Settings)),
            )
            .add_systems(OnEnter(GameState::JoinPrompt), spawn_join_prompt)
            .add_systems(OnExit(GameState::JoinPrompt), despawn_join_prompt)
            .add_systems(
                Update,
                join_prompt_input.run_if(in_state(GameState::JoinPrompt)),
            );
    }
}

fn spawn_background(parent: &mut ChildBuilder) {
    parent.spawn(NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            ..default()
        },
        background_color: BackgroundColor(FOG_COLOR),
        ..default()
    });

    for (left, top) in [
        (Val::Px(0.0), Val::Px(0.0)),
        (Val::Auto, Val::Px(0.0)),
        (Val::Px(0.0), Val::Auto),
        (Val::Auto, Val::Auto),
    ] {
        let right = if matches!(left, Val::Auto) {
            Val::Px(0.0)
        } else {
            Val::Auto
        };
        let bottom = if matches!(top, Val::Auto) {
            Val::Px(0.0)
        } else {
            Val::Auto
        };
        parent.spawn(NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                left,
                top,
                right,
                bottom,
                width: Val::Px(360.0),
                height: Val::Px(260.0),
                ..default()
            },
            background_color: BackgroundColor(VIGNETTE_COLOR),
            ..default()
        });
    }

    parent.spawn(NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            height: Val::Px(2.0),
            ..default()
        },
        background_color: BackgroundColor(ACCENT_DIM),
        ..default()
    });
    parent.spawn(NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            bottom: Val::Px(0.0),
            height: Val::Px(2.0),
            ..default()
        },
        background_color: BackgroundColor(ACCENT_DIM),
        ..default()
    });
    parent.spawn(NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            width: Val::Px(2.0),
            ..default()
        },
        background_color: BackgroundColor(PANEL_BORDER_DARK),
        ..default()
    });
    parent.spawn(NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            width: Val::Px(2.0),
            ..default()
        },
        background_color: BackgroundColor(PANEL_BORDER_DARK),
        ..default()
    });
}

fn spawn_title_block(parent: &mut ChildBuilder, font: &Handle<Font>, title: &str) {
    parent
        .spawn(NodeBundle {
            style: Style {
                position_type: PositionType::Relative,
                margin: UiRect::bottom(Val::Px(4.0)),
                ..default()
            },
            ..default()
        })
        .with_children(|stack| {
            stack.spawn(
                TextBundle::from_section(
                    title,
                    TextStyle {
                        font: font.clone(),
                        font_size: 72.0,
                        color: TITLE_SHADOW,
                    },
                )
                .with_style(Style {
                    position_type: PositionType::Absolute,
                    left: Val::Px(4.0),
                    top: Val::Px(4.0),
                    ..default()
                }),
            );
            stack.spawn(TextBundle::from_section(
                title,
                TextStyle {
                    font: font.clone(),
                    font_size: 72.0,
                    color: ACCENT,
                },
            ));
        });
}

fn spawn_divider(parent: &mut ChildBuilder) {
    parent.spawn(NodeBundle {
        style: Style {
            width: Val::Px(360.0),
            height: Val::Px(1.0),
            margin: UiRect::vertical(Val::Px(14.0)),
            ..default()
        },
        background_color: BackgroundColor(Color::srgba(0.25, 0.28, 0.32, 0.65)),
        ..default()
    });
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
                    ..default()
                },
                background_color: BackgroundColor(BG_COLOR),
                ..default()
            },
            MenuRoot,
        ))
        .with_children(|root| {
            spawn_background(root);
            root.spawn(NodeBundle {
                style: Style {
                    width: Val::Px(560.0),
                    padding: UiRect::all(Val::Px(36.0)),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(10.0),
                    border: UiRect::all(Val::Px(3.0)),
                    ..default()
                },
                background_color: BackgroundColor(PANEL_COLOR),
                border_color: BorderColor(PANEL_BORDER),
                ..default()
            })
            .with_children(|panel| {
                spawn_title_block(panel, &font, "ZOMBIES");
                panel.spawn(TextBundle::from_section(
                    "WAVES  OF  SURVIVAL",
                    TextStyle {
                        font: font.clone(),
                        font_size: 18.0,
                        color: TEXT_SUBTITLE,
                    },
                ));
                spawn_divider(panel);
                panel
                    .spawn(NodeBundle {
                        style: Style {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            row_gap: Val::Px(12.0),
                            margin: UiRect::vertical(Val::Px(8.0)),
                            ..default()
                        },
                        ..default()
                    })
                    .with_children(|list| {
                        for (i, label) in ITEMS.iter().enumerate() {
                            list.spawn((
                                TextBundle::from_section(
                                    *label,
                                    TextStyle {
                                        font: font.clone(),
                                        font_size: 24.0,
                                        color: TEXT_NORMAL,
                                    },
                                ),
                                MenuItem { index: i },
                            ));
                        }
                    });
                spawn_divider(panel);
                panel.spawn((
                    TextBundle::from_section(
                        "",
                        TextStyle {
                            font: font.clone(),
                            font_size: 12.0,
                            color: ERROR_COLOR,
                        },
                    ),
                    MenuErrorText,
                ));
                panel.spawn(
                    TextBundle::from_section(
                        "ARROWS - SELECT     ENTER - CONFIRM",
                        TextStyle {
                            font,
                            font_size: 11.0,
                            color: TEXT_DIM,
                        },
                    )
                    .with_style(Style {
                        margin: UiRect::top(Val::Px(8.0)),
                        ..default()
                    }),
                );
            });
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
    mut ctx: ResMut<NetContext>,
    mut net_mode: ResMut<NetMode>,
    mut error: ResMut<MenuError>,
    mut sfx: EventWriter<SfxEvent>,
) {
    let up = keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW);
    let down = keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS);
    if up {
        selection.0 = (selection.0 + ITEMS.len() - 1) % ITEMS.len();
        sfx.send(SfxEvent::MenuMove);
    }
    if down {
        selection.0 = (selection.0 + 1) % ITEMS.len();
        sfx.send(SfxEvent::MenuMove);
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        sfx.send(SfxEvent::MenuSelect);
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
                error.0.clear();
                next_state.set(GameState::Settings);
            }
            4 => {
                ctx.disconnect();
                std::process::exit(0);
            }
            _ => {}
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        ctx.disconnect();
        std::process::exit(0);
    }
}

fn menu_highlight(selection: Res<MenuSelection>, mut items: Query<(&MenuItem, &mut Text)>) {
    if !selection.is_changed() && !selection.is_added() {
        return;
    }
    for (item, mut text) in &mut items {
        let selected = item.index == selection.0;
        let label = ITEMS[item.index];
        text.sections[0].value = if selected {
            format!(">  {label}  <")
        } else {
            format!("   {label}   ")
        };
        text.sections[0].style.color = if selected { TEXT_HIGHLIGHT } else { TEXT_NORMAL };
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

fn spawn_settings(
    mut commands: Commands,
    mut selection: ResMut<SettingsSelection>,
    assets: Res<UiAssets>,
    settings: Res<GraphicsSettings>,
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
                    ..default()
                },
                background_color: BackgroundColor(BG_COLOR),
                ..default()
            },
            SettingsRoot,
        ))
        .with_children(|root| {
            spawn_background(root);
            root.spawn(NodeBundle {
                style: Style {
                    width: Val::Px(620.0),
                    padding: UiRect::all(Val::Px(36.0)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(10.0),
                    border: UiRect::all(Val::Px(3.0)),
                    ..default()
                },
                background_color: BackgroundColor(PANEL_COLOR),
                border_color: BorderColor(PANEL_BORDER),
                ..default()
            })
            .with_children(|panel| {
                spawn_title_block(panel, &font, "SETTINGS");
                panel.spawn(TextBundle::from_section(
                    "GRAPHICS",
                    TextStyle {
                        font: font.clone(),
                        font_size: 16.0,
                        color: TEXT_SUBTITLE,
                    },
                ));
                spawn_divider(panel);
                panel
                    .spawn(NodeBundle {
                        style: Style {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(12.0),
                            margin: UiRect::vertical(Val::Px(6.0)),
                            ..default()
                        },
                        ..default()
                    })
                    .with_children(|list| {
                        for i in 0..SETTINGS_ROW_COUNT {
                            spawn_settings_row(list, &font, i, &settings);
                        }
                    });
                spawn_divider(panel);
                panel.spawn(
                    TextBundle::from_section(
                        "ARROWS - CHANGE     ESC - BACK",
                        TextStyle {
                            font,
                            font_size: 11.0,
                            color: TEXT_DIM,
                        },
                    )
                    .with_style(Style {
                        margin: UiRect::top(Val::Px(8.0)),
                        ..default()
                    }),
                );
            });
        });
}

fn spawn_settings_row(
    list: &mut ChildBuilder,
    font: &Handle<Font>,
    index: usize,
    settings: &GraphicsSettings,
) {
    list.spawn((
        NodeBundle {
            style: Style {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                padding: UiRect::horizontal(Val::Px(28.0)),
                ..default()
            },
            ..default()
        },
        SettingsRow { index },
    ))
    .with_children(|row| {
        row.spawn(TextBundle::from_section(
            SETTINGS_LABELS[index],
            TextStyle {
                font: font.clone(),
                font_size: 18.0,
                color: TEXT_NORMAL,
            },
        ));
        let value = settings_value_text(settings, index);
        row.spawn((
            TextBundle::from_section(
                value,
                TextStyle {
                    font: font.clone(),
                    font_size: 18.0,
                    color: TEXT_NORMAL,
                },
            ),
            SettingsValueText { index },
        ));
    });
}

fn settings_value_text(settings: &GraphicsSettings, index: usize) -> String {
    match index {
        0 => settings.resolution_label(),
        1 => settings.window_mode_label().to_string(),
        2 => settings.vsync_label().to_string(),
        3 => settings.fps_cap_label(),
        4 => String::new(),
        _ => String::new(),
    }
}

fn despawn_settings(mut commands: Commands, q: Query<Entity, With<SettingsRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn settings_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut selection: ResMut<SettingsSelection>,
    mut settings: ResMut<GraphicsSettings>,
    mut next_state: ResMut<NextState<GameState>>,
    mut sfx: EventWriter<SfxEvent>,
) {
    let up = keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW);
    let down = keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS);
    let left = keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::KeyA);
    let right = keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::KeyD);

    if up {
        selection.0 = (selection.0 + SETTINGS_ROW_COUNT - 1) % SETTINGS_ROW_COUNT;
        sfx.send(SfxEvent::MenuMove);
    }
    if down {
        selection.0 = (selection.0 + 1) % SETTINGS_ROW_COUNT;
        sfx.send(SfxEvent::MenuMove);
    }

    if left || right {
        let forward = right;
        match selection.0 {
            0 => settings.cycle_resolution(forward),
            1 => settings.cycle_window_mode(forward),
            2 => settings.toggle_vsync(),
            3 => settings.cycle_fps_cap(forward),
            _ => {}
        }
        if selection.0 != 4 {
            sfx.send(SfxEvent::MenuMove);
        }
    }

    if keys.just_pressed(KeyCode::Enter) && selection.0 == 4 {
        sfx.send(SfxEvent::MenuCancel);
        next_state.set(GameState::Menu);
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        sfx.send(SfxEvent::MenuCancel);
        next_state.set(GameState::Menu);
    }
}

fn settings_refresh(
    selection: Res<SettingsSelection>,
    settings: Res<GraphicsSettings>,
    mut rows: Query<(&SettingsRow, &Children)>,
    mut values: Query<(&SettingsValueText, &mut Text), Without<SettingsRow>>,
    mut labels: Query<&mut Text, (Without<SettingsValueText>, Without<SettingsRow>)>,
) {
    let selection_changed = selection.is_changed() || selection.is_added();
    let settings_changed = settings.is_changed() || settings.is_added();
    if !selection_changed && !settings_changed {
        return;
    }

    for (value_marker, mut text) in &mut values {
        let idx = value_marker.index;
        text.sections[0].value = settings_value_text(&settings, idx);
        let selected = idx == selection.0;
        text.sections[0].style.color = if selected {
            TEXT_HIGHLIGHT
        } else {
            TEXT_NORMAL
        };
    }

    for (row, children) in &mut rows {
        let selected = row.index == selection.0;
        for child in children.iter() {
            let Ok(mut text) = labels.get_mut(*child) else {
                continue;
            };
            let raw_label = SETTINGS_LABELS[row.index];
            text.sections[0].value = if selected {
                format!("> {raw_label}")
            } else {
                format!("  {raw_label}")
            };
            text.sections[0].style.color = if selected {
                TEXT_HIGHLIGHT
            } else {
                TEXT_NORMAL
            };
        }
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
                    ..default()
                },
                background_color: BackgroundColor(BG_COLOR),
                ..default()
            },
            JoinPromptRoot,
        ))
        .with_children(|root| {
            spawn_background(root);
            root.spawn(NodeBundle {
                style: Style {
                    width: Val::Px(560.0),
                    padding: UiRect::all(Val::Px(36.0)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(12.0),
                    border: UiRect::all(Val::Px(3.0)),
                    ..default()
                },
                background_color: BackgroundColor(PANEL_COLOR),
                border_color: BorderColor(PANEL_BORDER),
                ..default()
            })
            .with_children(|panel| {
                spawn_title_block(panel, &font, "JOIN");
                panel.spawn(TextBundle::from_section(
                    "LAN MODE",
                    TextStyle {
                        font: font.clone(),
                        font_size: 16.0,
                        color: TEXT_SUBTITLE,
                    },
                ));
                spawn_divider(panel);
                panel.spawn(TextBundle::from_section(
                    "ENTER HOST IP (DIGITS AND DOTS):",
                    TextStyle {
                        font: font.clone(),
                        font_size: 13.0,
                        color: TEXT_DIM,
                    },
                ));
                panel.spawn((
                    TextBundle::from_section(
                        format!("IP: {}_", addr.text),
                        TextStyle {
                            font: font.clone(),
                            font_size: 26.0,
                            color: TEXT_HIGHLIGHT,
                        },
                    )
                    .with_style(Style {
                        margin: UiRect::vertical(Val::Px(8.0)),
                        ..default()
                    }),
                    JoinPromptIpText,
                ));
                panel.spawn((
                    TextBundle::from_section(
                        "",
                        TextStyle {
                            font: font.clone(),
                            font_size: 12.0,
                            color: ERROR_COLOR,
                        },
                    ),
                    JoinPromptErrorText,
                ));
                spawn_divider(panel);
                panel.spawn(TextBundle::from_section(
                    "ENTER - CONNECT   BACKSPACE - DELETE   ESC - BACK",
                    TextStyle {
                        font,
                        font_size: 10.0,
                        color: TEXT_DIM,
                    },
                ));
            });
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
    mut sfx: EventWriter<SfxEvent>,
) {
    let mut changed = false;
    for key in keys.get_just_pressed() {
        if let Some(d) = keycode_to_digit(*key) {
            if addr.text.len() < 21 {
                addr.text.push(d);
                changed = true;
                sfx.send(SfxEvent::MenuMove);
            }
        } else if matches!(key, KeyCode::Period | KeyCode::NumpadDecimal) {
            if addr.text.len() < 21 {
                addr.text.push('.');
                changed = true;
                sfx.send(SfxEvent::MenuMove);
            }
        } else if *key == KeyCode::Backspace {
            addr.text.pop();
            changed = true;
            sfx.send(SfxEvent::MenuMove);
        }
    }
    if changed {
        if let Ok(mut text) = ip_text.get_single_mut() {
            text.sections[0].value = format!("IP: {}_", addr.text);
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        sfx.send(SfxEvent::MenuCancel);
        next_state.set(GameState::Menu);
        return;
    }
    if keys.just_pressed(KeyCode::Enter) {
        sfx.send(SfxEvent::MenuSelect);
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
                        addr.error = format!("Error: {e}");
                        if let Ok(mut t) = err_text.get_single_mut() {
                            t.sections[0].value = addr.error.clone();
                        }
                    }
                }
            }
            Err(_) => {
                addr.error = "Invalid IP".to_string();
                if let Ok(mut t) = err_text.get_single_mut() {
                    t.sections[0].value = addr.error.clone();
                }
            }
        }
    }
}

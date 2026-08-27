use bevy::prelude::*;
use std::net::{IpAddr, SocketAddr};

use crate::audio::SfxEvent;
use crate::net::{
    sanitize_nickname, start_client, start_host, LocalNickname, NetContext, NetMode,
    PlayerNicknames, NICKNAME_MAX_LEN, NET_PORT,
};
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
pub struct JoinPromptNickText;

#[derive(Component)]
pub struct JoinPromptErrorText;

#[derive(Component)]
pub struct GuideRoot;

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

#[derive(Resource, Default)]
pub struct MenuError(pub String);

#[derive(Resource)]
pub struct JoinPromptUiState {
    pub nick_active: bool,
}

impl Default for JoinPromptUiState {
    fn default() -> Self {
        Self { nick_active: true }
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

const ITEMS: [&str; 7] = [
    "SINGLE PLAYER",
    "HOST LAN",
    "JOIN LAN",
    "SETTINGS",
    "ACHIEVEMENTS",
    "HOW TO PLAY",
    "QUIT",
];

/// How long the QUIT confirm stays armed after an Escape / Back press; a
/// second press inside this window actually exits.
const QUIT_CONFIRM_WINDOW: f64 = 2.0;
/// Status-line hint shown while the quit confirm is armed.
const QUIT_HINT: &str = "PRESS AGAIN TO QUIT";

/// One row of the settings screen.  Rows are listed per-platform by
/// `settings_rows()` rather than indexed positionally, so mobile can show a
/// trimmed set without the value/action logic drifting out of sync.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingKind {
    Volume,
    Resolution,
    WindowMode,
    Vsync,
    FpsLimit,
    Quality,
    FpsCounter,
    ResetDefaults,
    Back,
}

impl SettingKind {
    fn label(self) -> &'static str {
        match self {
            SettingKind::Volume => "VOLUME",
            SettingKind::Resolution => "RESOLUTION",
            SettingKind::WindowMode => "WINDOW MODE",
            SettingKind::Vsync => "VSYNC",
            SettingKind::FpsLimit => "FPS LIMIT",
            SettingKind::Quality => "QUALITY",
            SettingKind::FpsCounter => "FPS COUNTER",
            SettingKind::ResetDefaults => "RESET DEFAULTS",
            SettingKind::Back => "BACK",
        }
    }

    fn value(self, s: &GraphicsSettings) -> String {
        match self {
            SettingKind::Volume => s.volume_label(),
            SettingKind::Resolution => s.resolution_label(),
            SettingKind::WindowMode => s.window_mode_label().to_string(),
            SettingKind::Vsync => s.vsync_label().to_string(),
            SettingKind::FpsLimit => s.fps_cap_label(),
            SettingKind::Quality => s.quality_label().to_string(),
            SettingKind::FpsCounter => s.show_fps_label().to_string(),
            // Action rows have no right-hand value.
            SettingKind::ResetDefaults | SettingKind::Back => String::new(),
        }
    }

    /// True for rows whose value left/right cycles (vs. Enter-only action rows).
    fn is_value_row(self) -> bool {
        !matches!(self, SettingKind::ResetDefaults | SettingKind::Back)
    }
}

/// The settings rows shown on this platform.  Phones get only the master volume
/// — resolution / window mode / vsync / fps / quality are meaningless on a
/// fullscreen, fixed-resolution device, so they're hidden there.
fn settings_rows() -> &'static [SettingKind] {
    use SettingKind::*;
    if crate::mobile_profile() {
        &[Volume, Back]
    } else {
        &[
            Volume,
            Resolution,
            WindowMode,
            Vsync,
            FpsLimit,
            Quality,
            FpsCounter,
            ResetDefaults,
            Back,
        ]
    }
}

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
            .init_resource::<JoinPromptUiState>()
            .add_systems(OnEnter(GameState::Menu), spawn_menu)
            .add_systems(OnExit(GameState::Menu), despawn_menu)
            .add_systems(
                Update,
                // `menu_touch_select` runs first so a tap's synthetic Enter is
                // visible to `menu_navigate` in the same frame.
                (
                    menu_touch_select,
                    menu_navigate,
                    menu_highlight,
                    update_menu_error,
                )
                    .chain()
                    .run_if(in_state(GameState::Menu)),
            )
            .add_systems(OnEnter(GameState::Settings), spawn_settings)
            .add_systems(OnExit(GameState::Settings), despawn_settings)
            .add_systems(
                Update,
                (settings_input, settings_refresh).run_if(in_state(GameState::Settings)),
            )
            .add_systems(
                OnEnter(GameState::JoinPrompt),
                (reset_join_prompt_state, spawn_join_prompt).chain(),
            )
            .add_systems(OnExit(GameState::JoinPrompt), despawn_join_prompt)
            .add_systems(
                Update,
                join_prompt_input.run_if(in_state(GameState::JoinPrompt)),
            )
            .add_systems(OnEnter(GameState::Guide), spawn_guide)
            .add_systems(OnExit(GameState::Guide), despawn_guide)
            .add_systems(
                Update,
                guide_input.run_if(in_state(GameState::Guide)),
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
                                )
                                // Padding enlarges the node's box so it's a
                                // comfortable touch target, not just the glyph
                                // bounds.  `Interaction` makes bevy_ui hit-test
                                // taps/clicks against it (handles touch + DPI).
                                .with_style(Style {
                                    padding: UiRect::axes(Val::Px(30.0), Val::Px(10.0)),
                                    ..default()
                                })
                                // Selection decorations must never word-wrap —
                                // a wrapped "<" doubles the item's height and
                                // the whole column visibly jumps.
                                .with_no_wrap(),
                                MenuItem { index: i },
                                Interaction::default(),
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
                    )
                    // Reserve one line of height: an empty Text measures 0px,
                    // so without this the panel grows/shrinks (and every item
                    // shifts vertically) each time the quit hint or an error
                    // toggles.  min_height, not height — real errors ("Host
                    // fail: …") may still wrap onto more lines.
                    .with_style(Style {
                        min_height: Val::Px(14.0),
                        ..default()
                    }),
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

/// Touch / mouse support for the main menu: tapping an item selects it and
/// fires a synthetic Enter, so the existing `menu_navigate` confirm logic runs
/// unchanged.  bevy_ui's focus system sets `Interaction::Pressed` for both
/// clicks and finger taps (hit-testing in logical UI space, so DPI/rotation are
/// handled for us) — which is why the menu text was previously un-tappable: the
/// items had no `Interaction` and only the on-screen nav buttons drove it.
fn menu_touch_select(
    mut selection: ResMut<MenuSelection>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut injected: ResMut<crate::mobile::InjectedKeys>,
    items: Query<(&MenuItem, &Interaction), Changed<Interaction>>,
) {
    for (item, interaction) in &items {
        if *interaction == Interaction::Pressed {
            selection.0 = item.index;
            // Record the synthetic Enter so `release_injected_keys` clears it —
            // otherwise it sticks in `pressed` and the *next* tap won't confirm.
            keys.press(KeyCode::Enter);
            injected.0.push(KeyCode::Enter);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn menu_navigate(
    keys: Res<ButtonInput<KeyCode>>,
    mut selection: ResMut<MenuSelection>,
    mut next_state: ResMut<NextState<GameState>>,
    mut ctx: ResMut<NetContext>,
    mut net_mode: ResMut<NetMode>,
    mut error: ResMut<MenuError>,
    mut sfx: EventWriter<SfxEvent>,
    local_nick: Res<LocalNickname>,
    mut nicknames: ResMut<PlayerNicknames>,
    time: Res<Time>,
    mut quit_armed: Local<Option<f64>>,
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
    // A pending quit confirm lapses after a short window, and moving the
    // cursor cancels it; drop the hint too, but never a real error message.
    let now = time.elapsed_seconds_f64();
    let lapsed = quit_armed.is_some_and(|armed| now - armed > QUIT_CONFIRM_WINDOW);
    if quit_armed.is_some() && (lapsed || up || down) {
        *quit_armed = None;
        if error.0 == QUIT_HINT {
            error.0.clear();
        }
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        sfx.send(SfxEvent::MenuSelect);
        match selection.0 {
            0 => {
                ctx.disconnect();
                *net_mode = NetMode::SinglePlayer;
                ctx.my_id = 0;
                ctx.lobby_players = vec![0];
                nicknames.0.clear();
                nicknames
                    .0
                    .insert(0, sanitize_nickname(&local_nick.0));
                next_state.set(GameState::Playing);
            }
            1 => match start_host() {
                Ok(host) => {
                    ctx.disconnect();
                    ctx.host = Some(host);
                    ctx.my_id = 0;
                    ctx.lobby_players = vec![0];
                    *net_mode = NetMode::Host;
                    nicknames.0.clear();
                    nicknames
                        .0
                        .insert(0, sanitize_nickname(&local_nick.0));
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
                error.0.clear();
                next_state.set(GameState::Achievements);
            }
            5 => {
                error.0.clear();
                next_state.set(GameState::Guide);
            }
            6 => {
                // Enter (including the tap-injected one from
                // `menu_touch_select`) goes through the same two-step confirm
                // as Escape — one stray press/tap on the QUIT row's big touch
                // hit box must not kill the app instantly.
                if quit_armed.is_some() {
                    ctx.disconnect();
                    std::process::exit(0);
                }
                *quit_armed = Some(now);
                error.0 = QUIT_HINT.to_string();
            }
            _ => {}
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        // Esc moves the cursor onto QUIT (index 6) — a second Esc (or Enter)
        // there within a short window exits.  This stops accidentally killing
        // the app when a player just wanted to back out of a sub-menu; the
        // mobile Back nav button injects this same Escape.
        if selection.0 == 6 && quit_armed.is_some() {
            sfx.send(SfxEvent::MenuSelect);
            ctx.disconnect();
            std::process::exit(0);
        }
        if selection.0 != 6 {
            selection.0 = 6;
            sfx.send(SfxEvent::MenuMove);
        }
        *quit_armed = Some(now);
        error.0 = QUIT_HINT.to_string();
    }
}

fn menu_highlight(selection: Res<MenuSelection>, mut items: Query<(&MenuItem, &mut Text)>) {
    if !selection.is_changed() && !selection.is_added() {
        return;
    }
    for (item, mut text) in &mut items {
        let selected = item.index == selection.0;
        let label = ITEMS[item.index];
        // Single-space decorations keep the longest label ("SINGLE PLAYER",
        // 17 chars decorated) inside the 560px panel's content box even with
        // the 30px touch padding — the old double-space variant hit 19 chars
        // (456px of PressStart2P at 24px) and word-wrapped the trailing "<"
        // onto a second line, shoving the whole item column around.
        text.sections[0].value = if selected {
            format!("> {label} <")
        } else {
            format!("  {label}  ")
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
                    // Mobile shows only the volume row, so "GRAPHICS" would lie.
                    if crate::mobile_profile() { "AUDIO" } else { "GRAPHICS" },
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
                        for i in 0..settings_rows().len() {
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
        let kind = settings_rows()[index];
        row.spawn(TextBundle::from_section(
            kind.label(),
            TextStyle {
                font: font.clone(),
                font_size: 18.0,
                color: TEXT_NORMAL,
            },
        ));
        let value = kind.value(settings);
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

    let rows = settings_rows();
    let count = rows.len();
    if up {
        selection.0 = (selection.0 + count - 1) % count;
        sfx.send(SfxEvent::MenuMove);
    }
    if down {
        selection.0 = (selection.0 + 1) % count;
        sfx.send(SfxEvent::MenuMove);
    }

    let kind = rows[selection.0.min(count - 1)];
    if left || right {
        let forward = right;
        match kind {
            SettingKind::Volume => settings.cycle_volume(forward),
            SettingKind::Resolution => settings.cycle_resolution(forward),
            SettingKind::WindowMode => settings.cycle_window_mode(forward),
            SettingKind::Vsync => settings.toggle_vsync(),
            SettingKind::FpsLimit => settings.cycle_fps_cap(forward),
            SettingKind::Quality => settings.cycle_quality(forward),
            SettingKind::FpsCounter => settings.toggle_show_fps(),
            // Action rows have no left/right value to cycle.
            SettingKind::ResetDefaults | SettingKind::Back => {}
        }
        if kind.is_value_row() {
            sfx.send(SfxEvent::MenuMove);
        }
    }

    if keys.just_pressed(KeyCode::Enter) {
        match kind {
            SettingKind::ResetDefaults => {
                // Replace the resource with defaults — `is_changed()` then
                // fires next frame and `apply_graphics_settings` pushes the
                // window/vsync/etc. back to factory values.  The save-on-change
                // system writes the new state to disk on the same beat.
                *settings = GraphicsSettings::default();
                sfx.send(SfxEvent::MenuSelect);
            }
            SettingKind::Back => {
                sfx.send(SfxEvent::MenuCancel);
                next_state.set(GameState::Menu);
                return;
            }
            _ => {}
        }
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

    let rows_list = settings_rows();
    for (value_marker, mut text) in &mut values {
        let idx = value_marker.index;
        text.sections[0].value = rows_list[idx].value(&settings);
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
            let raw_label = rows_list[row.index].label();
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

fn spawn_join_prompt(
    mut commands: Commands,
    assets: Res<UiAssets>,
    addr: Res<JoinAddress>,
    nick: Res<LocalNickname>,
) {
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
                    format!("NICK (LITERY, MAX {} ZNAKOW):", NICKNAME_MAX_LEN),
                    TextStyle {
                        font: font.clone(),
                        font_size: 13.0,
                        color: TEXT_DIM,
                    },
                ));
                panel.spawn((
                    TextBundle::from_section(
                        format!("NICK: {}_", nick.0),
                        TextStyle {
                            font: font.clone(),
                            font_size: 22.0,
                            color: TEXT_HIGHLIGHT,
                        },
                    )
                    .with_style(Style {
                        margin: UiRect::vertical(Val::Px(4.0)),
                        ..default()
                    }),
                    JoinPromptNickText,
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
                        format!("IP: {}", addr.text),
                        TextStyle {
                            font: font.clone(),
                            font_size: 22.0,
                            color: TEXT_NORMAL,
                        },
                    )
                    .with_style(Style {
                        margin: UiRect::vertical(Val::Px(4.0)),
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
                    "TAB - SWITCH FIELD   ENTER - CONNECT   ESC - BACK",
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

fn reset_join_prompt_state(mut s: ResMut<JoinPromptUiState>) {
    s.nick_active = true;
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

fn keycode_to_letter(k: KeyCode) -> Option<char> {
    match k {
        KeyCode::KeyA => Some('A'),
        KeyCode::KeyB => Some('B'),
        KeyCode::KeyC => Some('C'),
        KeyCode::KeyD => Some('D'),
        KeyCode::KeyE => Some('E'),
        KeyCode::KeyF => Some('F'),
        KeyCode::KeyG => Some('G'),
        KeyCode::KeyH => Some('H'),
        KeyCode::KeyI => Some('I'),
        KeyCode::KeyJ => Some('J'),
        KeyCode::KeyK => Some('K'),
        KeyCode::KeyL => Some('L'),
        KeyCode::KeyM => Some('M'),
        KeyCode::KeyN => Some('N'),
        KeyCode::KeyO => Some('O'),
        KeyCode::KeyP => Some('P'),
        KeyCode::KeyQ => Some('Q'),
        KeyCode::KeyR => Some('R'),
        KeyCode::KeyS => Some('S'),
        KeyCode::KeyT => Some('T'),
        KeyCode::KeyU => Some('U'),
        KeyCode::KeyV => Some('V'),
        KeyCode::KeyW => Some('W'),
        KeyCode::KeyX => Some('X'),
        KeyCode::KeyY => Some('Y'),
        KeyCode::KeyZ => Some('Z'),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn join_prompt_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut addr: ResMut<JoinAddress>,
    mut nick: ResMut<LocalNickname>,
    mut ui_state: ResMut<JoinPromptUiState>,
    mut ctx: ResMut<NetContext>,
    mut net_mode: ResMut<NetMode>,
    mut next_state: ResMut<NextState<GameState>>,
    mut ip_text: Query<
        &mut Text,
        (
            With<JoinPromptIpText>,
            Without<JoinPromptErrorText>,
            Without<JoinPromptNickText>,
        ),
    >,
    mut nick_text: Query<
        &mut Text,
        (
            With<JoinPromptNickText>,
            Without<JoinPromptErrorText>,
            Without<JoinPromptIpText>,
        ),
    >,
    mut err_text: Query<
        &mut Text,
        (
            With<JoinPromptErrorText>,
            Without<JoinPromptIpText>,
            Without<JoinPromptNickText>,
        ),
    >,
    mut sfx: EventWriter<SfxEvent>,
) {
    // Default to nick field on first appearance.  We start with nick active
    // so the player types their name first and TABs to the IP.
    if keys.just_pressed(KeyCode::Tab) {
        ui_state.nick_active = !ui_state.nick_active;
        sfx.send(SfxEvent::MenuMove);
    }

    let mut changed = false;
    for key in keys.get_just_pressed() {
        if *key == KeyCode::Tab {
            continue;
        }
        if ui_state.nick_active {
            if let Some(c) = keycode_to_letter(*key) {
                if nick.0.chars().count() < NICKNAME_MAX_LEN {
                    nick.0.push(c);
                    changed = true;
                    sfx.send(SfxEvent::MenuMove);
                }
            } else if let Some(d) = keycode_to_digit(*key) {
                if nick.0.chars().count() < NICKNAME_MAX_LEN {
                    nick.0.push(d);
                    changed = true;
                    sfx.send(SfxEvent::MenuMove);
                }
            } else if *key == KeyCode::Backspace {
                nick.0.pop();
                changed = true;
                sfx.send(SfxEvent::MenuMove);
            }
        } else if let Some(d) = keycode_to_digit(*key) {
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
        if let Ok(mut text) = nick_text.get_single_mut() {
            text.sections[0].value = if ui_state.nick_active {
                format!("NICK: {}_", nick.0)
            } else {
                format!("NICK: {}", nick.0)
            };
            text.sections[0].style.color = if ui_state.nick_active {
                TEXT_HIGHLIGHT
            } else {
                TEXT_NORMAL
            };
        }
        if let Ok(mut text) = ip_text.get_single_mut() {
            text.sections[0].value = if ui_state.nick_active {
                format!("IP: {}", addr.text)
            } else {
                format!("IP: {}_", addr.text)
            };
            text.sections[0].style.color = if ui_state.nick_active {
                TEXT_NORMAL
            } else {
                TEXT_HIGHLIGHT
            };
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
                let clean_nick = sanitize_nickname(&nick.0);
                nick.0 = clean_nick.clone();
                match start_client(sock, &clean_nick) {
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

fn spawn_guide(mut commands: Commands, assets: Res<UiAssets>) {
    let font = assets.font.clone();
    let title = TextStyle {
        font: font.clone(),
        font_size: 28.0,
        color: TEXT_HIGHLIGHT,
    };
    let heading = TextStyle {
        font: font.clone(),
        font_size: 16.0,
        color: Color::srgb(0.9, 0.75, 0.3),
    };
    let body = TextStyle {
        font: font.clone(),
        font_size: 12.0,
        color: TEXT_NORMAL,
    };
    let hint = TextStyle {
        font: font.clone(),
        font_size: 11.0,
        color: TEXT_DIM,
    };

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
            GuideRoot,
        ))
        .with_children(|root| {
            root.spawn(NodeBundle {
                style: Style {
                    width: Val::Px(680.0),
                    max_height: Val::Percent(88.0),
                    padding: UiRect::all(Val::Px(24.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    overflow: Overflow::clip_y(),
                    ..default()
                },
                background_color: BackgroundColor(PANEL_COLOR),
                border_color: BorderColor(PANEL_BORDER),
                ..default()
            })
            .with_children(|panel| {
                panel.spawn(TextBundle::from_section("HOW TO PLAY", title));

                panel.spawn(TextBundle::from_section("CONTROLS", heading.clone()));
                panel.spawn(TextBundle::from_section(
                    "WASD / Arrows  -  Move\n\
                     Mouse  -  Aim\n\
                     Left Click  -  Shoot\n\
                     Right Click  -  Throw grenade\n\
                     1 / 2 / 3  -  Switch weapon slot\n\
                     R  -  Reload weapon\n\
                     ESC  -  Pause",
                    body.clone(),
                ));

                panel.spawn(TextBundle::from_section("WEAPONS & ITEMS", heading.clone()));
                panel.spawn(TextBundle::from_section(
                    "Slot 1 & 2 hold weapons. Pick up new ones on the map.\n\
                     Slot 3 holds throwables: Grenades, Smoke, Molotovs.\n\
                     Weapons auto-reload when magazine is empty.\n\
                     Press R to reload manually at any time.",
                    body.clone(),
                ));

                panel.spawn(TextBundle::from_section("ENEMIES", heading.clone()));
                panel.spawn(TextBundle::from_section(
                    "Normal  -  Standard zombie\n\
                     Fast  -  Quick but fragile\n\
                     Exploder  -  Explodes on contact! Keep distance\n\
                     Burning  -  Ignites you (35 HP over 10s). Avoid!\n\
                     Giant  -  Massive HP, very slow. From wave 5 on",
                    body.clone(),
                ));

                panel.spawn(TextBundle::from_section("SURVIVAL TIPS", heading));
                panel.spawn(TextBundle::from_section(
                    "Keep moving - standing still is death.\n\
                     Prioritize Exploders and Burning zombies.\n\
                     Use grenades on large groups of zombies.\n\
                     Smoke grenades slow zombies - great for escaping.\n\
                     Collect weapon and health pickups on the map.\n\
                     Unlock new map zones to access the shop.",
                    body,
                ));

                panel.spawn(
                    TextBundle::from_section(
                        "PRESS ESC / ENTER TO GO BACK",
                        hint,
                    )
                    .with_style(Style {
                        margin: UiRect::top(Val::Px(10.0)),
                        ..default()
                    }),
                );
            });
        });
}

fn despawn_guide(mut commands: Commands, q: Query<Entity, With<GuideRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn guide_input(keys: Res<ButtonInput<KeyCode>>, mut next_state: ResMut<NextState<GameState>>) {
    if keys.just_pressed(KeyCode::Escape)
        || keys.just_pressed(KeyCode::Enter)
        || keys.just_pressed(KeyCode::Space)
    {
        next_state.set(GameState::Menu);
    }
}

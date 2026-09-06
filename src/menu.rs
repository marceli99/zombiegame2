use bevy::prelude::*;

use crate::audio::SfxEvent;
use crate::net::{
    sanitize_nickname, start_client, start_host, LocalNickname, NetContext, NetMode,
    PlayerNicknames, NICKNAME_MAX_LEN, ROOM_CODE_LEN, ROOM_CODE_MAX_LEN,
};
use crate::settings::GraphicsSettings;
use crate::{GameState, UiAssets};

#[derive(Component)]
pub struct MenuRoot;

#[derive(Component)]
pub struct MenuItem {
    pub index: usize,
}

/// The text inside a main-menu row (the row itself is `MenuItem`).
#[derive(Component)]
pub struct MenuItemLabel {
    pub index: usize,
}

#[derive(Component)]
pub struct MenuErrorText;

#[derive(Component)]
pub struct JoinPromptRoot;

#[derive(Component)]
pub struct JoinPromptCodeText;

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
            text: String::new(),
            error: String::new(),
        }
    }
}

/// Invite link (`?room=CODE`, browser only): skip the main menu and land on
/// the join screen with the code already typed — the player only adds a
/// nick and presses Enter.
fn open_invite_link(mut addr: ResMut<JoinAddress>, mut next_state: ResMut<NextState<GameState>>) {
    #[cfg(target_arch = "wasm32")]
    if let Some(code) = crate::net_web::room_from_url() {
        addr.text = code;
        next_state.set(GameState::JoinPrompt);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (&mut addr, &mut next_state);
}

const ITEMS: [&str; 7] = [
    "SINGLE PLAYER",
    "CREATE ROOM",
    "JOIN ROOM",
    "SETTINGS",
    "ACHIEVEMENTS",
    "HOW TO PLAY",
    "QUIT",
];
/// Position of QUIT in `ITEMS` — the Escape shortcut jumps there.
const QUIT_INDEX: usize = 6;

/// Main-menu rows for this platform.  The browser build drops QUIT: the tab
/// is the app and `process::exit` has nothing to exit, so the row would be a
/// dead button.
fn menu_items() -> &'static [&'static str] {
    if cfg!(target_arch = "wasm32") {
        &ITEMS[..QUIT_INDEX]
    } else {
        &ITEMS
    }
}

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
    } else if cfg!(target_arch = "wasm32") {
        // Browser: the canvas is sized by the page and frames are paced by
        // requestAnimationFrame, so resolution / window mode / vsync / fps
        // cap have nothing to act on.
        &[Volume, Quality, FpsCounter, ResetDefaults, Back]
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

pub(crate) const BG_COLOR: Color = Color::srgb(0.012, 0.016, 0.022);
pub(crate) const PANEL_COLOR: Color = Color::srgba(0.035, 0.04, 0.05, 0.94);
pub(crate) const PANEL_BORDER: Color = Color::srgb(0.22, 0.28, 0.32);
const PANEL_BORDER_DARK: Color = Color::srgb(0.08, 0.1, 0.12);
const ACCENT: Color = Color::srgb(0.42, 0.12, 0.08);
const ACCENT_DIM: Color = Color::srgb(0.22, 0.07, 0.05);
const TITLE_SHADOW: Color = Color::srgba(0.0, 0.0, 0.0, 0.95);
pub(crate) const TEXT_DIM: Color = Color::srgb(0.32, 0.34, 0.38);
pub(crate) const TEXT_NORMAL: Color = Color::srgb(0.55, 0.58, 0.62);
pub(crate) const TEXT_HIGHLIGHT: Color = Color::srgb(0.82, 0.72, 0.28);
pub(crate) const TEXT_SUBTITLE: Color = Color::srgb(0.48, 0.36, 0.2);
pub(crate) const ERROR_COLOR: Color = Color::srgb(0.78, 0.24, 0.2);
/// Faint bar behind the selected menu row — the arrows carry the selection,
/// this just anchors it.
const HIGHLIGHT_BAR: Color = Color::srgba(0.82, 0.72, 0.28, 0.08);
/// Every main-menu row is this wide so the highlight bar is one steady
/// shape; the longest decorated label ("> SINGLE PLAYER <", 17 glyphs at
/// 24 px ≈ 408 px) fits with room to spare inside the 560 px panel.
const MENU_ITEM_WIDTH: f32 = 460.0;
/// Standard menu panel: fixed width, 36 px inset, 3 px border.  Shared by
/// the main menu, settings, join and lobby screens so they line up.
pub(crate) const PANEL_WIDTH: f32 = 560.0;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuSelection>()
            .init_resource::<SettingsSelection>()
            .init_resource::<MenuError>()
            .init_resource::<JoinAddress>()
            .add_systems(Startup, open_invite_link)
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

/// Radial vignette for the menu screens: clear in the middle, darkening
/// smoothly towards the edges so the panel reads as the lit part of the
/// screen.  bevy_ui has no gradients, so it is a small generated texture
/// stretched over the viewport with linear filtering.
pub fn build_vignette_image() -> Image {
    const W: i32 = 128;
    const H: i32 = 72;
    let mut canvas = crate::pixelart::Canvas::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let nx = (x as f32 + 0.5) / W as f32 * 2.0 - 1.0;
            let ny = (y as f32 + 0.5) / H as f32 * 2.0 - 1.0;
            let d = (nx * nx + ny * ny).sqrt();
            // Untouched inside r=0.45, full strength at the corners (r≈1.4).
            let t = ((d - 0.45) / 0.9).clamp(0.0, 1.0);
            let a = t * t * (3.0 - 2.0 * t) * 0.8;
            canvas.put(x, y, [0, 0, 0, (a * 255.0) as u8]);
        }
    }
    let mut img = canvas.into_image();
    img.sampler = bevy::render::texture::ImageSampler::linear();
    img
}

/// Full-screen dressing behind every menu panel: the vignette plus a thin
/// accent line along the top and bottom edges.  The root node's `BG_COLOR`
/// is the ground it darkens.
pub(crate) fn spawn_background(parent: &mut ChildBuilder, assets: &UiAssets) {
    parent.spawn(ImageBundle {
        style: Style {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        image: UiImage::new(assets.vignette.clone()),
        ..default()
    });

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

/// The framed panel every menu screen is built in.
pub(crate) fn panel_bundle() -> NodeBundle {
    NodeBundle {
        style: Style {
            width: Val::Px(PANEL_WIDTH),
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
    }
}

/// Width of a panel's content box (`panel_bundle`: width minus padding and
/// border) — what titles and hint lines must fit into.
pub(crate) const PANEL_CONTENT_WIDTH: f32 = PANEL_WIDTH - 2.0 * 36.0 - 2.0 * 3.0;

/// PressStart2P is an 8×8 pixel font: only sizes that are multiples of 8
/// put every font pixel on whole screen pixels (anything else blurs into
/// grey fringes).  Titles take the largest such size that still fits the
/// panel — 64 for "ZOMBIES"/"JOIN"/"LOBBY", 56 for "SETTINGS".
fn title_size(title: &str) -> f32 {
    let fit = (PANEL_CONTENT_WIDTH / title.chars().count().max(1) as f32 / 8.0).floor() * 8.0;
    fit.clamp(32.0, 64.0)
}

pub(crate) fn spawn_title_block(parent: &mut ChildBuilder, font: &Handle<Font>, title: &str) {
    let size = title_size(title);
    // One font pixel of drop shadow — an offset that isn't on the font's
    // pixel grid renders as an uneven thin outline instead.
    let shadow = size / 8.0;
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
                        font_size: size,
                        color: TITLE_SHADOW,
                    },
                )
                .with_style(Style {
                    position_type: PositionType::Absolute,
                    left: Val::Px(shadow),
                    top: Val::Px(shadow),
                    ..default()
                }),
            );
            stack.spawn(TextBundle::from_section(
                title,
                TextStyle {
                    font: font.clone(),
                    font_size: size,
                    color: ACCENT,
                },
            ));
        });
}

pub(crate) fn spawn_divider(parent: &mut ChildBuilder) {
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
            spawn_background(root, &assets);
            root.spawn(panel_bundle())
            .with_children(|panel| {
                spawn_title_block(panel, &font, "ZOMBIES");
                panel.spawn(TextBundle::from_section(
                    "WAVES  OF  SURVIVAL",
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
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            row_gap: Val::Px(12.0),
                            margin: UiRect::vertical(Val::Px(8.0)),
                            ..default()
                        },
                        ..default()
                    })
                    .with_children(|list| {
                        for (i, label) in menu_items().iter().enumerate() {
                            // A fixed-width row is the touch/click target
                            // (`Interaction`) and carries the highlight bar;
                            // the text is a child so it is centred for real —
                            // bevy_ui 0.14 lays glyphs out from the node's
                            // corner and ignores padding, so padding on the
                            // text node itself shoved every label 30 px left.
                            list.spawn((
                                NodeBundle {
                                    style: Style {
                                        width: Val::Px(MENU_ITEM_WIDTH),
                                        padding: UiRect::vertical(Val::Px(8.0)),
                                        justify_content: JustifyContent::Center,
                                        align_items: AlignItems::Center,
                                        ..default()
                                    },
                                    background_color: BackgroundColor(Color::NONE),
                                    ..default()
                                },
                                MenuItem { index: i },
                                Interaction::default(),
                            ))
                            .with_children(|row| {
                                row.spawn((
                                    TextBundle::from_section(
                                        *label,
                                        TextStyle {
                                            font: font.clone(),
                                            font_size: 24.0,
                                            color: TEXT_NORMAL,
                                        },
                                    )
                                    // Selection decorations must never
                                    // word-wrap — a wrapped "<" doubles the
                                    // row's height and the column jumps.
                                    .with_no_wrap(),
                                    MenuItemLabel { index: i },
                                ));
                            });
                        }
                    });
                spawn_divider(panel);
                panel.spawn((
                    TextBundle::from_section(
                        "",
                        TextStyle {
                            font: font.clone(),
                            font_size: 16.0,
                            color: ERROR_COLOR,
                        },
                    )
                    // Reserve one line of height: an empty Text measures 0px,
                    // so without this the panel grows/shrinks (and every item
                    // shifts vertically) each time the quit hint or an error
                    // toggles.  min_height, not height — real errors ("Host
                    // fail: …") may still wrap onto more lines.
                    .with_style(Style {
                        min_height: Val::Px(18.0),
                        ..default()
                    }),
                    MenuErrorText,
                ));
                panel.spawn(
                    TextBundle::from_section(
                        "ARROWS - SELECT   ENTER - OK",
                        TextStyle {
                            font,
                            font_size: 16.0,
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
    let item_count = menu_items().len();
    if up {
        selection.0 = (selection.0 + item_count - 1) % item_count;
        sfx.send(SfxEvent::MenuMove);
    }
    if down {
        selection.0 = (selection.0 + 1) % item_count;
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
            1 => match {
                ctx.disconnect();
                start_host()
            } {
                Ok(host) => {
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
                    error.0 = e;
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
            QUIT_INDEX => {
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
    // No QUIT row in the browser build — Escape on the main menu is a no-op.
    if keys.just_pressed(KeyCode::Escape) && item_count > QUIT_INDEX {
        // Esc moves the cursor onto QUIT — a second Esc (or Enter) there
        // within a short window exits.  This stops accidentally killing the
        // app when a player just wanted to back out of a sub-menu; the
        // mobile Back nav button injects this same Escape.
        if selection.0 == QUIT_INDEX && quit_armed.is_some() {
            sfx.send(SfxEvent::MenuSelect);
            ctx.disconnect();
            std::process::exit(0);
        }
        if selection.0 != QUIT_INDEX {
            selection.0 = QUIT_INDEX;
            sfx.send(SfxEvent::MenuMove);
        }
        *quit_armed = Some(now);
        error.0 = QUIT_HINT.to_string();
    }
}

fn menu_highlight(
    selection: Res<MenuSelection>,
    mut rows: Query<(&MenuItem, &mut BackgroundColor)>,
    mut items: Query<(&MenuItemLabel, &mut Text)>,
) {
    if !selection.is_changed() && !selection.is_added() {
        return;
    }
    for (row, mut bg) in &mut rows {
        bg.0 = if row.index == selection.0 { HIGHLIGHT_BAR } else { Color::NONE };
    }
    for (item, mut text) in &mut items {
        let selected = item.index == selection.0;
        let label = menu_items()[item.index];
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
            spawn_background(root, &assets);
            root.spawn(panel_bundle()).with_children(|panel| {
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
                        "ARROWS - CHANGE   ESC - BACK",
                        TextStyle {
                            font,
                            font_size: 16.0,
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
                font_size: 16.0,
                color: TEXT_NORMAL,
            },
        ));
        let value = kind.value(settings);
        row.spawn((
            TextBundle::from_section(
                value,
                TextStyle {
                    font: font.clone(),
                    font_size: 16.0,
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
            spawn_background(root, &assets);
            root.spawn(panel_bundle()).with_children(|panel| {
                spawn_title_block(panel, &font, "JOIN");
                panel.spawn(TextBundle::from_section(
                    "BY ROOM CODE",
                    TextStyle {
                        font: font.clone(),
                        font_size: 16.0,
                        color: TEXT_SUBTITLE,
                    },
                ));
                spawn_divider(panel);
                panel.spawn(TextBundle::from_section(
                    format!("NICK (LETTERS, MAX {}):", NICKNAME_MAX_LEN),
                    TextStyle {
                        font: font.clone(),
                        font_size: 16.0,
                        color: TEXT_DIM,
                    },
                ));
                panel.spawn((
                    TextBundle::from_section(
                        format!("NICK: {}_", nick.0),
                        TextStyle {
                            font: font.clone(),
                            font_size: 24.0,
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
                    "ENTER ROOM CODE:",
                    TextStyle {
                        font: font.clone(),
                        font_size: 16.0,
                        color: TEXT_DIM,
                    },
                ));
                panel.spawn((
                    TextBundle::from_section(
                        format!("CODE: {}", addr.text),
                        TextStyle {
                            font: font.clone(),
                            font_size: 24.0,
                            color: TEXT_NORMAL,
                        },
                    )
                    .with_style(Style {
                        margin: UiRect::vertical(Val::Px(4.0)),
                        ..default()
                    }),
                    JoinPromptCodeText,
                ));
                panel.spawn((
                    TextBundle::from_section(
                        "",
                        TextStyle {
                            font: font.clone(),
                            font_size: 16.0,
                            color: ERROR_COLOR,
                        },
                    ),
                    JoinPromptErrorText,
                ));
                spawn_divider(panel);
                // Two nodes rather than one two-line text: PressStart2P's
                // line height equals its glyph height, so wrapped lines
                // touch; the panel's row gap spaces these properly.
                for hint in ["TAB - FIELD   ENTER - JOIN", "ESC - BACK"] {
                    panel.spawn(TextBundle::from_section(
                        hint,
                        TextStyle {
                            font: font.clone(),
                            font_size: 16.0,
                            color: TEXT_DIM,
                        },
                    ));
                }
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

pub(crate) fn keycode_to_digit(k: KeyCode) -> Option<char> {
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

pub(crate) fn keycode_to_letter(k: KeyCode) -> Option<char> {
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
    mut code_text: Query<
        &mut Text,
        (
            With<JoinPromptCodeText>,
            Without<JoinPromptErrorText>,
            Without<JoinPromptNickText>,
        ),
    >,
    mut nick_text: Query<
        &mut Text,
        (
            With<JoinPromptNickText>,
            Without<JoinPromptErrorText>,
            Without<JoinPromptCodeText>,
        ),
    >,
    mut err_text: Query<
        &mut Text,
        (
            With<JoinPromptErrorText>,
            Without<JoinPromptCodeText>,
            Without<JoinPromptNickText>,
        ),
    >,
    mut sfx: EventWriter<SfxEvent>,
) {
    // Default to nick field on first appearance.  We start with nick active
    // so the player types their name first and TABs to the room code.
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
        } else if let Some(c) = keycode_to_letter(*key).or_else(|| keycode_to_digit(*key)) {
            if addr.text.chars().count() < ROOM_CODE_MAX_LEN {
                addr.text.push(c.to_ascii_uppercase());
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
        if let Ok(mut text) = code_text.get_single_mut() {
            text.sections[0].value = if ui_state.nick_active {
                format!("CODE: {}", addr.text)
            } else {
                format!("CODE: {}_", addr.text)
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
        let code = addr.text.trim().to_ascii_uppercase();
        if code.len() < ROOM_CODE_LEN {
            addr.error = format!("ENTER THE {ROOM_CODE_LEN}-CHAR ROOM CODE");
            if let Ok(mut t) = err_text.get_single_mut() {
                t.sections[0].value = addr.error.clone();
            }
            return;
        }
        let clean_nick = sanitize_nickname(&nick.0);
        nick.0 = clean_nick.clone();
        ctx.disconnect();
        match start_client(&code, &clean_nick) {
            Ok(client) => {
                // Connecting is asynchronous — the lobby shows CONNECTING...
                // and falls back to the menu (with the reason) on failure.
                ctx.client = Some(client);
                ctx.room_code = code;
                *net_mode = NetMode::Client;
                addr.error.clear();
                next_state.set(GameState::Lobby);
            }
            Err(e) => {
                addr.error = e;
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

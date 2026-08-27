//! Touch (Android/iOS) + gamepad controls.
//!
//! All gameplay input ultimately funnels through `LocalInput` (a `NetInput`),
//! exactly like keyboard/mouse.  This module aggregates gamepad sticks/buttons
//! and on-screen touch controls into a `VirtualInput` resource each frame and
//! merges it into `LocalInput` *after* the keyboard/mouse pass, so all three
//! input methods coexist (handy: a controller works on desktop for testing,
//! touch lights up on phones).
//!
//! Menus are keyboard-driven, so rather than rewrite every menu we bridge the
//! gamepad (and the on-screen nav buttons) into synthetic `KeyCode` presses —
//! the existing menu code then "just works" with a pad or a thumb.
//!
//! Touch UI only activates when `MobileUi.enabled` (`crate::mobile_profile()`:
//! true on Android/iOS, or when `ZG_FORCE_TOUCH` is set so the layout can be
//! eyeballed on desktop — the same switch that picks the mobile zoom).  The
//! code always compiles on every platform; it's inert on desktop at runtime.

use bevy::input::gamepad::{GamepadAxis, GamepadAxisType, GamepadButton, GamepadButtonType};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::chat::ChatInputState;
use crate::net::{LocalInput, NetContext};
use crate::player::Player;
use crate::zombie::Zombie;
use crate::{GameState, UiAssets};

// ── Tuning ──────────────────────────────────────────────────────────────
const GP_MOVE_DEADZONE: f32 = 0.25;
const GP_AIM_DEADZONE: f32 = 0.45;
const TOUCH_STICK_RADIUS: f32 = 90.0;
const TOUCH_MOVE_DEADZONE: f32 = 0.20;
const TOUCH_AIM_DEADZONE: f32 = 0.20;

// ── Aim assist (touch/pad) ──────────────────────────────────────────────
// Precise 360° aiming with a thumb is brutal, so when the aim stick is pushed
// we snap fire toward the nearest zombie that sits within a cone around the
// stick direction.  The stick still *chooses* the target (point left → shoot
// the zombie on the left); assist only cleans up the last few degrees.
/// Only zombies this close (world px) are eligible — keeps far-off targets
/// from yanking the aim across the screen.
const AIM_ASSIST_RANGE: f32 = 760.0;
/// Cone half-angle as a dot-product threshold: dir·aim ≥ this.  cos(50°)≈0.64,
/// so anything within ~50° of where the thumb points can be locked onto.
const AIM_ASSIST_COS: f32 = 0.64;

pub struct MobileControlsPlugin;

impl Plugin for MobileControlsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MobileUi>()
            .init_resource::<VirtualInput>()
            .init_resource::<TouchSticks>()
            .init_resource::<InjectedKeys>()
            .add_systems(Startup, setup_mobile_ui)
            .add_systems(
                Update,
                (reset_virtual_input, read_gamepad, read_touch_gameplay).chain(),
            )
            .add_systems(
                Update,
                apply_virtual_to_local
                    .after(read_touch_gameplay)
                    .after(crate::player::gather_local_input),
            )
            // Refine the just-applied stick aim toward the nearest zombie.
            .add_systems(Update, apply_aim_assist.after(apply_virtual_to_local))
            // Menu nav injects synthetic key presses.  Run it in PreUpdate
            // *after* bevy's input systems (which clear `just_pressed` each
            // frame): the press then survives into Update so every menu's
            // keyboard handler sees it, regardless of system order.  In Update
            // the order vs. those handlers is ambiguous, which left the
            // on-screen nav buttons firing only intermittently.
            .add_systems(
                PreUpdate,
                (gamepad_menu_nav, touch_menu_nav).after(bevy::input::InputSystem),
            )
            .add_systems(
                Update,
                (
                    update_mobile_ui_layout,
                    update_mobile_ui_visibility,
                    update_stick_visuals,
                ),
            )
            // Release every synthetic key press at the very end of the frame
            // (after all menu handlers have read `just_pressed`).  Without this
            // a pressed key lingers forever — and a second `press()` of an
            // already-pressed key does NOT re-arm `just_pressed`, so every menu
            // confirm after the first one silently did nothing.
            .add_systems(Last, release_injected_keys);
    }
}

/// Master switch for the on-screen touch UI + touch reading.
#[derive(Resource)]
pub struct MobileUi {
    pub enabled: bool,
}

impl Default for MobileUi {
    fn default() -> Self {
        Self {
            enabled: crate::mobile_profile(),
        }
    }
}

/// Keys the gamepad/touch→keyboard bridge synthesised this frame.  Released in
/// `Last` (see `release_injected_keys`) so a synthetic press never sticks in
/// `ButtonInput::pressed`.  Shared with `menu::menu_touch_select`, which also
/// injects an Enter when a menu item is tapped.
#[derive(Resource, Default)]
pub struct InjectedKeys(pub Vec<KeyCode>);

/// Per-frame aggregated control state from gamepad + touch.  Reset every frame,
/// OR-merged into `LocalInput` by `apply_virtual_to_local`.
#[derive(Resource, Default)]
struct VirtualInput {
    move_dir: Vec2,
    /// Zero ⇒ not aiming this frame (keep keyboard/mouse aim).
    aim_dir: Vec2,
    fire: bool,
    reload: bool,
    throw: bool,
    interact: bool,
    interact_held: bool,
    /// 0 ⇒ no slot change requested.
    slot: u8,
}

/// Tracks which touch id currently owns each floating stick + where it started.
#[derive(Resource, Default)]
struct TouchSticks {
    move_id: Option<u64>,
    move_origin: Vec2,
    move_cur: Vec2,
    aim_id: Option<u64>,
    aim_origin: Vec2,
    aim_cur: Vec2,
}

// ── Button identity ─────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq)]
enum Btn {
    Reload,
    Grenade,
    Interact,
    Slot1,
    Slot2,
    Slot3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavBtn {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
}

#[derive(Component)]
struct GameBtnNode(Btn);
#[derive(Component)]
struct NavBtnNode(NavBtn);
#[derive(Component)]
struct GameControlsRoot;
#[derive(Component)]
struct NavControlsRoot;
#[derive(Component)]
struct MoveRing;
#[derive(Component)]
struct MoveKnob;
#[derive(Component)]
struct AimRing;
#[derive(Component)]
struct AimKnob;

// ── Layout (screen-space, recomputed from the window each frame so it adapts
//    to orientation / resolution).  The same function feeds the visual node
//    positions and the touch hit-testing so they never drift apart. ─────────
fn game_buttons(win: Vec2) -> [(Btn, Vec2, f32); 6] {
    // Bigger discs than before (0.072 vs 0.065) so they're easy thumb targets.
    let r = (win.x.min(win.y) * 0.072).max(40.0);
    let (w, h) = (win.x, win.y);
    [
        // Action cluster hugs the bottom edge; the mid/upper-right stays clear
        // for the floating aim stick so a tap to aim never lands on a button.
        (Btn::Interact, Vec2::new(w * 0.60, h * 0.85), r * 1.25), // USE, biggest
        (Btn::Reload, Vec2::new(w * 0.74, h * 0.88), r),
        (Btn::Grenade, Vec2::new(w * 0.87, h * 0.85), r),
        // Weapon slots: top-right row, dropped to 0.13 to clear the status bar.
        (Btn::Slot1, Vec2::new(w * 0.74, h * 0.13), r * 0.85),
        (Btn::Slot2, Vec2::new(w * 0.83, h * 0.13), r * 0.85),
        (Btn::Slot3, Vec2::new(w * 0.92, h * 0.13), r * 0.85),
    ]
}

fn nav_buttons(win: Vec2) -> [(NavBtn, Vec2, f32); 6] {
    let r = (win.x.min(win.y) * 0.075).max(38.0);
    let (w, h) = (win.x, win.y);
    // D-pad clustered bottom-right; Confirm/Back in the empty left margin so
    // neither covers the centred menu text.
    [
        (NavBtn::Up, Vec2::new(w * 0.88, h * 0.50), r),
        (NavBtn::Down, Vec2::new(w * 0.88, h * 0.86), r),
        (NavBtn::Left, Vec2::new(w * 0.80, h * 0.68), r),
        (NavBtn::Right, Vec2::new(w * 0.96, h * 0.68), r),
        (NavBtn::Confirm, Vec2::new(w * 0.10, h * 0.86), r),
        (NavBtn::Back, Vec2::new(w * 0.10, h * 0.55), r),
    ]
}

fn btn_label(b: Btn) -> &'static str {
    match b {
        Btn::Reload => "R",
        Btn::Grenade => "G",
        // The interact key unlocks map gates, takes stairs AND picks up / swaps
        // weapons — "USE" reads clearer on a thumb button than the desktop "E".
        Btn::Interact => "USE",
        Btn::Slot1 => "1",
        Btn::Slot2 => "2",
        Btn::Slot3 => "3",
    }
}

/// Colour-code the action buttons so they're tellable apart at a glance instead
/// of six identical grey discs.  Alpha kept low-ish so they don't crowd the
/// playfield; the visible ring + label carry the meaning.
fn btn_color(b: Btn) -> Color {
    match b {
        Btn::Interact => Color::srgba(0.30, 0.80, 0.38, 0.34), // green = primary action
        Btn::Grenade => Color::srgba(0.88, 0.52, 0.20, 0.34),  // orange = throw
        Btn::Reload => Color::srgba(0.45, 0.68, 0.95, 0.32),   // blue = reload
        Btn::Slot1 | Btn::Slot2 | Btn::Slot3 => Color::srgba(0.90, 0.90, 0.96, 0.30),
    }
}

fn nav_label(b: NavBtn) -> &'static str {
    match b {
        NavBtn::Up => "^",
        NavBtn::Down => "v",
        NavBtn::Left => "<",
        NavBtn::Right => ">",
        NavBtn::Confirm => "OK",
        NavBtn::Back => "X",
    }
}

// ── UI setup ────────────────────────────────────────────────────────────
fn setup_mobile_ui(mut commands: Commands, mobile: Res<MobileUi>, ui: Res<UiAssets>) {
    if !mobile.enabled {
        return;
    }
    let win = Vec2::new(crate::WINDOW_WIDTH, crate::WINDOW_HEIGHT);

    // Gameplay controls root.
    commands
        .spawn((
            NodeBundle {
                style: full_screen_style(),
                z_index: ZIndex::Global(50),
                ..default()
            },
            GameControlsRoot,
        ))
        .with_children(|root| {
            for (b, c, r) in game_buttons(win) {
                spawn_round_button(root, c, r, btn_label(b), btn_color(b), &ui, GameBtnNode(b));
            }
            // Floating stick rings (hidden until a finger lands).
            spawn_ring(root, TOUCH_STICK_RADIUS, MoveRing);
            spawn_knob(root, MoveKnob);
            spawn_ring(root, TOUCH_STICK_RADIUS, AimRing);
            spawn_knob(root, AimKnob);
        });

    // Menu navigation root.
    commands
        .spawn((
            NodeBundle {
                style: full_screen_style(),
                z_index: ZIndex::Global(50),
                ..default()
            },
            NavControlsRoot,
        ))
        .with_children(|root| {
            for (b, c, r) in nav_buttons(win) {
                spawn_round_button_nav(root, c, r, nav_label(b), &ui, NavBtnNode(b));
            }
        });
}

fn full_screen_style() -> Style {
    Style {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        top: Val::Px(0.0),
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    }
}

fn spawn_round_button(
    parent: &mut ChildBuilder,
    center: Vec2,
    radius: f32,
    label: &str,
    tint: Color,
    ui: &UiAssets,
    marker: GameBtnNode,
) {
    parent
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    left: Val::Px(center.x - radius),
                    top: Val::Px(center.y - radius),
                    width: Val::Px(radius * 2.0),
                    height: Val::Px(radius * 2.0),
                    border: UiRect::all(Val::Px(3.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                background_color: BackgroundColor(tint),
                border_color: BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            marker,
        ))
        .with_children(|b| {
            // Shrink the glyph a touch for multi-char labels ("USE") so it fits.
            let scale = if label.len() > 1 { 0.42 } else { 0.7 };
            b.spawn(TextBundle::from_section(
                label,
                TextStyle {
                    font: ui.font.clone(),
                    font_size: radius * scale,
                    color: Color::srgba(1.0, 1.0, 1.0, 0.95),
                },
            ));
        });
}

fn spawn_round_button_nav(
    parent: &mut ChildBuilder,
    center: Vec2,
    radius: f32,
    label: &str,
    ui: &UiAssets,
    marker: NavBtnNode,
) {
    parent
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    left: Val::Px(center.x - radius),
                    top: Val::Px(center.y - radius),
                    width: Val::Px(radius * 2.0),
                    height: Val::Px(radius * 2.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.9, 0.9, 0.95, 0.20)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            marker,
        ))
        .with_children(|b| {
            b.spawn(TextBundle::from_section(
                label,
                TextStyle {
                    font: ui.font.clone(),
                    font_size: radius * 0.55,
                    color: Color::srgba(1.0, 1.0, 1.0, 0.9),
                },
            ));
        });
}

fn spawn_ring<M: Component>(parent: &mut ChildBuilder, radius: f32, marker: M) {
    parent.spawn((
        NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                width: Val::Px(radius * 2.0),
                height: Val::Px(radius * 2.0),
                ..default()
            },
            background_color: BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.10)),
            visibility: Visibility::Hidden,
            border_radius: BorderRadius::MAX,
            ..default()
        },
        marker,
    ));
}

fn spawn_knob<M: Component>(parent: &mut ChildBuilder, marker: M) {
    let d = TOUCH_STICK_RADIUS * 0.8;
    parent.spawn((
        NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                width: Val::Px(d),
                height: Val::Px(d),
                ..default()
            },
            background_color: BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.30)),
            visibility: Visibility::Hidden,
            border_radius: BorderRadius::MAX,
            ..default()
        },
        marker,
    ));
}

// ── Layout: reposition button nodes whenever the window size changes ───────
fn update_mobile_ui_layout(
    mobile: Res<MobileUi>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut game_q: Query<(&GameBtnNode, &mut Style), Without<NavBtnNode>>,
    mut nav_q: Query<(&NavBtnNode, &mut Style), Without<GameBtnNode>>,
    mut last: Local<Vec2>,
) {
    if !mobile.enabled {
        return;
    }
    let Ok(window) = windows.get_single() else {
        return;
    };
    let win = Vec2::new(window.width(), window.height());
    // Writing `Style` marks it Changed even with identical values, which makes
    // bevy_ui relayout the button subtrees every frame.  The positions are a
    // pure function of the window size, so only touch the styles when that
    // size actually changes (`Local` starts at ZERO ⇒ the first frame runs).
    if *last == win {
        return;
    }
    *last = win;
    let gb = game_buttons(win);
    for (node, mut style) in game_q.iter_mut() {
        if let Some((_, c, r)) = gb.iter().copied().find(|(b, _, _)| *b == node.0) {
            style.left = Val::Px(c.x - r);
            style.top = Val::Px(c.y - r);
            style.width = Val::Px(r * 2.0);
            style.height = Val::Px(r * 2.0);
        }
    }
    let nb = nav_buttons(win);
    for (node, mut style) in nav_q.iter_mut() {
        if let Some((_, c, r)) = nb.iter().copied().find(|(b, _, _)| *b == node.0) {
            style.left = Val::Px(c.x - r);
            style.top = Val::Px(c.y - r);
            style.width = Val::Px(r * 2.0);
            style.height = Val::Px(r * 2.0);
        }
    }
}

// ── Show gameplay controls only while Playing, menu nav only otherwise ─────
fn update_mobile_ui_visibility(
    mobile: Res<MobileUi>,
    game: Res<State<GameState>>,
    mut game_root: Query<
        &mut Visibility,
        (With<GameControlsRoot>, Without<NavControlsRoot>),
    >,
    mut nav_root: Query<&mut Visibility, (With<NavControlsRoot>, Without<GameControlsRoot>)>,
) {
    if !mobile.enabled {
        return;
    }
    let playing = *game.get() == GameState::Playing;
    // Compare before writing: a `Visibility` write marks it Changed and makes
    // visibility propagation re-walk the whole control subtree.
    if let Ok(mut v) = game_root.get_single_mut() {
        let want = if playing {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    if let Ok(mut v) = nav_root.get_single_mut() {
        let want = if playing {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *v != want {
            *v = want;
        }
    }
}

// ── Input aggregation ──────────────────────────────────────────────────────
fn reset_virtual_input(mut v: ResMut<VirtualInput>) {
    *v = VirtualInput::default();
}

fn read_gamepad(
    gamepads: Res<Gamepads>,
    axes: Res<Axis<GamepadAxis>>,
    buttons: Res<ButtonInput<GamepadButton>>,
    game: Res<State<GameState>>,
    mut v: ResMut<VirtualInput>,
) {
    if *game.get() != GameState::Playing {
        return;
    }
    let Some(gp) = gamepads.iter().next() else {
        return;
    };

    let lx = axes
        .get(GamepadAxis::new(gp, GamepadAxisType::LeftStickX))
        .unwrap_or(0.0);
    let ly = axes
        .get(GamepadAxis::new(gp, GamepadAxisType::LeftStickY))
        .unwrap_or(0.0);
    let mv = Vec2::new(lx, ly);
    if mv.length() >= GP_MOVE_DEADZONE {
        v.move_dir = mv.clamp_length_max(1.0);
    }

    let rx = axes
        .get(GamepadAxis::new(gp, GamepadAxisType::RightStickX))
        .unwrap_or(0.0);
    let ry = axes
        .get(GamepadAxis::new(gp, GamepadAxisType::RightStickY))
        .unwrap_or(0.0);
    let aim = Vec2::new(rx, ry);
    if aim.length() >= GP_AIM_DEADZONE {
        v.aim_dir = aim.normalize();
        v.fire = true; // autofire while the aim stick is pushed
    }
    if buttons.pressed(GamepadButton::new(gp, GamepadButtonType::RightTrigger2)) {
        v.fire = true;
    }
    if buttons.just_pressed(GamepadButton::new(gp, GamepadButtonType::West)) {
        v.reload = true;
    }
    if buttons.just_pressed(GamepadButton::new(gp, GamepadButtonType::North)) {
        v.throw = true;
    }
    if buttons.just_pressed(GamepadButton::new(gp, GamepadButtonType::South)) {
        v.interact = true;
    }
    v.interact_held |= buttons.pressed(GamepadButton::new(gp, GamepadButtonType::South));
    if buttons.just_pressed(GamepadButton::new(gp, GamepadButtonType::DPadLeft)) {
        v.slot = 1;
    }
    if buttons.just_pressed(GamepadButton::new(gp, GamepadButtonType::DPadUp)) {
        v.slot = 2;
    }
    if buttons.just_pressed(GamepadButton::new(gp, GamepadButtonType::DPadRight)) {
        v.slot = 3;
    }
}

fn read_touch_gameplay(
    mobile: Res<MobileUi>,
    game: Res<State<GameState>>,
    touches: Res<Touches>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut sticks: ResMut<TouchSticks>,
    mut v: ResMut<VirtualInput>,
) {
    if !mobile.enabled || *game.get() != GameState::Playing {
        sticks.move_id = None;
        sticks.aim_id = None;
        return;
    }
    let Ok(window) = windows.get_single() else {
        return;
    };
    let win = Vec2::new(window.width(), window.height());
    let buttons = game_buttons(win);

    // New touches: a button tap, otherwise claim a floating stick by screen half.
    for t in touches.iter_just_pressed() {
        let p = t.position();
        if let Some((b, _, _)) = buttons.iter().copied().find(|(_, c, r)| p.distance(*c) <= *r) {
            match b {
                Btn::Reload => v.reload = true,
                Btn::Grenade => v.throw = true,
                Btn::Interact => v.interact = true,
                Btn::Slot1 => v.slot = 1,
                Btn::Slot2 => v.slot = 2,
                Btn::Slot3 => v.slot = 3,
            }
            continue;
        }
        if p.x < win.x * 0.5 {
            if sticks.move_id.is_none() {
                sticks.move_id = Some(t.id());
                sticks.move_origin = p;
                sticks.move_cur = p;
            }
        } else if sticks.aim_id.is_none() {
            sticks.aim_id = Some(t.id());
            sticks.aim_origin = p;
            sticks.aim_cur = p;
        }
    }

    for t in touches.iter_just_released() {
        if Some(t.id()) == sticks.move_id {
            sticks.move_id = None;
        }
        if Some(t.id()) == sticks.aim_id {
            sticks.aim_id = None;
        }
    }

    // Held interact button (revive / manhole channels need hold-progress).
    for t in touches.iter() {
        let p = t.position();
        if let Some((Btn::Interact, _, _)) =
            buttons.iter().copied().find(|(_, c, r)| p.distance(*c) <= *r)
        {
            v.interact_held = true;
        }
        if Some(t.id()) == sticks.move_id {
            sticks.move_cur = p;
        }
        if Some(t.id()) == sticks.aim_id {
            sticks.aim_cur = p;
        }
    }

    // Resolve stick vectors.  Screen Y is down-positive; world up is +Y, so flip.
    if sticks.move_id.is_some() {
        let d = sticks.move_cur - sticks.move_origin;
        let mut mv = Vec2::new(d.x, -d.y) / TOUCH_STICK_RADIUS;
        if mv.length() >= TOUCH_MOVE_DEADZONE {
            mv = mv.clamp_length_max(1.0);
            v.move_dir = mv;
        }
    }
    if sticks.aim_id.is_some() {
        let d = sticks.aim_cur - sticks.aim_origin;
        let aim = Vec2::new(d.x, -d.y);
        if aim.length() >= TOUCH_AIM_DEADZONE * TOUCH_STICK_RADIUS {
            v.aim_dir = aim.normalize();
            v.fire = true;
        }
    }
}

fn apply_virtual_to_local(
    v: Res<VirtualInput>,
    chat: Res<ChatInputState>,
    game: Res<State<GameState>>,
    mut local: ResMut<LocalInput>,
) {
    if *game.get() != GameState::Playing || chat.open {
        return;
    }
    if v.move_dir.length() > 0.05 {
        local.0.move_x = v.move_dir.x;
        local.0.move_y = v.move_dir.y;
    }
    if v.aim_dir != Vec2::ZERO {
        local.0.aim_x = v.aim_dir.x;
        local.0.aim_y = v.aim_dir.y;
    }
    if v.fire {
        local.0.shoot = true;
    }
    if v.throw {
        local.0.throw = true;
    }
    if v.reload {
        local.0.reload = true;
    }
    if v.interact {
        local.0.interact = true;
    }
    if v.interact_held {
        local.0.interact_held = true;
    }
    if v.slot != 0 {
        local.0.switch_slot = v.slot;
    }
}

// ── Aim assist: snap stick aim onto the nearest zombie in the cone ──────────
// Runs after `apply_virtual_to_local`, so `local.aim_*` already holds the raw
// stick direction.  We only touch it when a stick actually drove the aim this
// frame (`VirtualInput.aim_dir != 0`), which leaves desktop mouse aim alone and
// only kicks in on the touch/pad path.  No eligible zombie ⇒ raw aim stands.
fn apply_aim_assist(
    v: Res<VirtualInput>,
    game: Res<State<GameState>>,
    ctx: Res<NetContext>,
    players: Query<(&Transform, &Player)>,
    zombies: Query<&Transform, With<Zombie>>,
    mut local: ResMut<LocalInput>,
) {
    if *game.get() != GameState::Playing {
        return;
    }
    // Only assist a stick-driven aim (touch right-stick / gamepad right-stick).
    let aim = v.aim_dir.normalize_or_zero();
    if aim == Vec2::ZERO {
        return;
    }
    let Some(origin) = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .or_else(|| players.iter().next())
        .map(|(t, _)| t.translation.truncate())
    else {
        return;
    };

    // Pick the zombie best aligned with the thumb direction (highest dir·aim),
    // nudged toward nearer ones to break ties between two in the same cone.
    let mut best: Option<(f32, Vec2)> = None;
    for zt in zombies.iter() {
        let to = zt.translation.truncate() - origin;
        let dist = to.length();
        if dist < 8.0 || dist > AIM_ASSIST_RANGE {
            continue;
        }
        let dir = to / dist;
        let dot = dir.dot(aim);
        if dot < AIM_ASSIST_COS {
            continue;
        }
        let score = dot - dist * 0.0003;
        if best.map_or(true, |(b, _)| score > b) {
            best = Some((score, dir));
        }
    }
    if let Some((_, dir)) = best {
        local.0.aim_x = dir.x;
        local.0.aim_y = dir.y;
    }
}

// ── Move the floating stick visuals to the finger ──────────────────────────
fn update_stick_visuals(
    mobile: Res<MobileUi>,
    sticks: Res<TouchSticks>,
    mut set: ParamSet<(
        Query<(&mut Style, &mut Visibility), With<MoveRing>>,
        Query<(&mut Style, &mut Visibility), With<MoveKnob>>,
        Query<(&mut Style, &mut Visibility), With<AimRing>>,
        Query<(&mut Style, &mut Visibility), With<AimKnob>>,
    )>,
) {
    if !mobile.enabled {
        return;
    }
    let knob = TOUCH_STICK_RADIUS * 0.8;
    place_ring(
        set.p0().get_single_mut().ok(),
        sticks.move_id.is_some(),
        sticks.move_origin,
        TOUCH_STICK_RADIUS,
    );
    place_knob(
        set.p1().get_single_mut().ok(),
        sticks.move_id.is_some(),
        sticks.move_origin,
        sticks.move_cur,
        knob,
    );
    place_ring(
        set.p2().get_single_mut().ok(),
        sticks.aim_id.is_some(),
        sticks.aim_origin,
        TOUCH_STICK_RADIUS,
    );
    place_knob(
        set.p3().get_single_mut().ok(),
        sticks.aim_id.is_some(),
        sticks.aim_origin,
        sticks.aim_cur,
        knob,
    );
}

fn place_ring(q: Option<(Mut<Style>, Mut<Visibility>)>, active: bool, origin: Vec2, r: f32) {
    if let Some((mut style, mut vis)) = q {
        let want = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        if active {
            style.left = Val::Px(origin.x - r);
            style.top = Val::Px(origin.y - r);
        }
    }
}

fn place_knob(
    q: Option<(Mut<Style>, Mut<Visibility>)>,
    active: bool,
    origin: Vec2,
    cur: Vec2,
    d: f32,
) {
    if let Some((mut style, mut vis)) = q {
        let want = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        if active {
            let clamped = origin + (cur - origin).clamp_length_max(TOUCH_STICK_RADIUS);
            style.left = Val::Px(clamped.x - d * 0.5);
            style.top = Val::Px(clamped.y - d * 0.5);
        }
    }
}

// ── Menu navigation: gamepad → synthetic key presses ───────────────────────
// Menus read `ButtonInput<KeyCode>`; pressing the matching key here lets a pad
// (and the on-screen nav buttons) drive every existing keyboard menu unchanged.
fn gamepad_menu_nav(
    gamepads: Res<Gamepads>,
    pad: Res<ButtonInput<GamepadButton>>,
    axes: Res<Axis<GamepadAxis>>,
    game: Res<State<GameState>>,
    mut sticks: Local<bool>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut injected: ResMut<InjectedKeys>,
) {
    if *game.get() == GameState::Playing {
        return;
    }
    let Some(gp) = gamepads.iter().next() else {
        return;
    };

    let mut press = |k: KeyCode| {
        keys.press(k);
        injected.0.push(k);
    };

    if pad.just_pressed(GamepadButton::new(gp, GamepadButtonType::DPadUp)) {
        press(KeyCode::ArrowUp);
    }
    if pad.just_pressed(GamepadButton::new(gp, GamepadButtonType::DPadDown)) {
        press(KeyCode::ArrowDown);
    }
    if pad.just_pressed(GamepadButton::new(gp, GamepadButtonType::DPadLeft)) {
        press(KeyCode::ArrowLeft);
    }
    if pad.just_pressed(GamepadButton::new(gp, GamepadButtonType::DPadRight)) {
        press(KeyCode::ArrowRight);
    }
    if pad.just_pressed(GamepadButton::new(gp, GamepadButtonType::South)) {
        press(KeyCode::Enter);
    }
    if pad.just_pressed(GamepadButton::new(gp, GamepadButtonType::East)) {
        press(KeyCode::Escape);
    }

    // Left stick edge → arrow taps (debounced so a held stick = one step).
    let ly = axes
        .get(GamepadAxis::new(gp, GamepadAxisType::LeftStickY))
        .unwrap_or(0.0);
    let lx = axes
        .get(GamepadAxis::new(gp, GamepadAxisType::LeftStickX))
        .unwrap_or(0.0);
    let pushed = ly.abs().max(lx.abs()) >= 0.6;
    if pushed && !*sticks {
        if ly >= 0.6 {
            press(KeyCode::ArrowUp);
        } else if ly <= -0.6 {
            press(KeyCode::ArrowDown);
        } else if lx >= 0.6 {
            press(KeyCode::ArrowRight);
        } else if lx <= -0.6 {
            press(KeyCode::ArrowLeft);
        }
    }
    *sticks = pushed;
}

// ── Menu navigation: on-screen nav buttons → synthetic key presses ─────────
fn touch_menu_nav(
    mobile: Res<MobileUi>,
    game: Res<State<GameState>>,
    touches: Res<Touches>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut injected: ResMut<InjectedKeys>,
) {
    if !mobile.enabled || *game.get() == GameState::Playing {
        return;
    }
    let Ok(window) = windows.get_single() else {
        return;
    };
    let win = Vec2::new(window.width(), window.height());
    let nb = nav_buttons(win);
    for t in touches.iter_just_pressed() {
        let p = t.position();
        if let Some((b, _, _)) = nb.iter().copied().find(|(_, c, r)| p.distance(*c) <= *r) {
            let k = match b {
                NavBtn::Up => KeyCode::ArrowUp,
                NavBtn::Down => KeyCode::ArrowDown,
                NavBtn::Left => KeyCode::ArrowLeft,
                NavBtn::Right => KeyCode::ArrowRight,
                NavBtn::Confirm => KeyCode::Enter,
                NavBtn::Back => KeyCode::Escape,
            };
            keys.press(k);
            injected.0.push(k);
        }
    }
}

// ── Release synthetic key presses at end-of-frame ──────────────────────────
// `ButtonInput::press` leaves a key in `pressed` until something releases it,
// and a synthetic press has no matching release event.  A stuck key makes the
// next `press()` a no-op for `just_pressed`, so menus confirmed only on the
// very first tap.  Draining here (in `Last`, after every reader) keeps each
// injected press a clean one-frame edge.
fn release_injected_keys(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut injected: ResMut<InjectedKeys>,
) {
    for k in injected.0.drain(..) {
        keys.release(k);
    }
}

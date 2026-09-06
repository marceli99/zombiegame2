use bevy::prelude::*;

use crate::net::{NetContext, NetMode};
use crate::{GameState, PauseState, UiAssets};

#[derive(Component)]
pub struct PauseUi;

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            pause_toggle_input
                .before(crate::chat::ChatInputSet)
                .run_if(in_state(GameState::Playing)),
        )
        .add_systems(OnEnter(PauseState::Paused), spawn_pause_ui)
        .add_systems(OnExit(PauseState::Paused), despawn_pause_ui)
        .add_systems(
            Update,
            pause_menu_input.run_if(in_state(PauseState::Paused)),
        )
        .add_systems(OnExit(GameState::Playing), force_resume);
    }
}

fn pause_toggle_input(
    keys: Res<ButtonInput<KeyCode>>,
    current: Res<State<PauseState>>,
    chat: Res<crate::chat::ChatInputState>,
    mut next: ResMut<NextState<PauseState>>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    // Esc is a chat-cancel too.  This runs *before* `chat_input_system`
    // (which closes the box on the same press), so an open box means the
    // press belongs to the chat and must not also toggle pause.
    if chat.open {
        return;
    }
    match current.get() {
        PauseState::Running => next.set(PauseState::Paused),
        PauseState::Paused => next.set(PauseState::Running),
    }
}

fn pause_menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut next_game: ResMut<NextState<GameState>>,
    mut next_pause: ResMut<NextState<PauseState>>,
    mut ctx: ResMut<NetContext>,
    mut mode: ResMut<NetMode>,
) {
    if keys.just_pressed(KeyCode::KeyQ) || keys.just_pressed(KeyCode::KeyM) {
        if *mode != NetMode::SinglePlayer {
            ctx.disconnect();
            *mode = NetMode::SinglePlayer;
        }
        next_game.set(GameState::Menu);
        next_pause.set(PauseState::Running);
    }
}

fn force_resume(mut next: ResMut<NextState<PauseState>>) {
    next.set(PauseState::Running);
}

fn spawn_pause_ui(mut commands: Commands, assets: Res<UiAssets>, net: Res<NetMode>) {
    let font = assets.font.clone();
    let is_multi = *net != NetMode::SinglePlayer;
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(22.0),
                    ..default()
                },
                // MP runs the world while the local "pause" UI is up (it's
                // really just an in-place menu, not a world-pause), so we keep
                // the overlay lighter so the player can still read what's
                // happening behind it.  SP fully pauses the simulation, so a
                // darker overlay reads cleaner without any motion to peek at.
                background_color: BackgroundColor(Color::srgba(
                    0.0,
                    0.0,
                    0.0,
                    if is_multi { 0.5 } else { 0.7 },
                )),
                z_index: ZIndex::Global(100),
                ..default()
            },
            PauseUi,
        ))
        .with_children(|parent| {
            parent.spawn(
                TextBundle::from_section(
                    "PAUSED",
                    TextStyle {
                        font: font.clone(),
                        font_size: 56.0,
                        color: Color::srgb(1.0, 0.85, 0.3),
                    },
                )
                .with_style(Style {
                    margin: UiRect::bottom(Val::Px(30.0)),
                    ..default()
                }),
            );
            parent.spawn(TextBundle::from_section(
                "ESC - resume",
                TextStyle {
                    font: font.clone(),
                    font_size: 18.0,
                    color: Color::srgb(0.9, 0.9, 0.9),
                },
            ));
            parent.spawn(TextBundle::from_section(
                if is_multi { "Q - disconnect" } else { "Q - main menu" },
                TextStyle {
                    font,
                    font_size: 18.0,
                    color: Color::srgb(0.9, 0.9, 0.9),
                },
            ));
        });
}

fn despawn_pause_ui(mut commands: Commands, q: Query<Entity, With<PauseUi>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

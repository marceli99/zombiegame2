use std::collections::VecDeque;

use bevy::prelude::*;

use crate::menu::{keycode_to_digit, keycode_to_letter};

use crate::net::{
    broadcast, sanitize_chat, ClientMsg, LocalNickname, NetContext, NetMode, ServerMsg,
    CHAT_MAX_LEN,
};
use crate::{GameState, UiAssets};

/// Time before a chat line stops rendering in the overlay.  The line stays
/// in `ChatLog` until trimmed by `MAX_LINES`, but visually fades out.
pub const CHAT_LINE_LIFETIME_SECS: f32 = 9.0;
/// Hard cap on history depth — old lines drop off the bottom.
const MAX_LINES: usize = 8;
/// Y-offset and spacing for stacked chat lines in the overlay.
const LINE_FONT_SIZE: f32 = 14.0;
const LINE_GAP_PX: f32 = 2.0;

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub author: String,
    pub text: String,
    /// Seconds since received — drives the fade-out.
    pub age_secs: f32,
}

#[derive(Resource, Default)]
pub struct ChatLog {
    pub lines: VecDeque<ChatLine>,
}

impl ChatLog {
    pub fn push(&mut self, author: String, text: String) {
        self.lines.push_back(ChatLine {
            author,
            text,
            age_secs: 0.0,
        });
        while self.lines.len() > MAX_LINES {
            self.lines.pop_front();
        }
    }
}

#[derive(Resource, Default)]
pub struct ChatInputState {
    pub open: bool,
    pub buffer: String,
}

#[derive(Component)]
pub struct ChatRoot;

#[derive(Component)]
pub struct ChatHistoryRoot;

#[derive(Component)]
pub struct ChatHistoryLine {
    pub slot: usize,
}

#[derive(Component)]
pub struct ChatInputBox;

#[derive(Component)]
pub struct ChatInputText;

/// Bevy `SystemSet` for the chat input handler, exposed so other modules
/// (notably `pause.rs`, which also listens for Escape) can order their own
/// keyboard systems after chat to avoid same-frame races on the same key.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChatInputSet;

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChatLog>()
            .init_resource::<ChatInputState>()
            .add_systems(OnEnter(GameState::Playing), spawn_chat_overlay)
            .add_systems(OnExit(GameState::Playing), (despawn_chat_overlay, reset_chat_state))
            // Close any open chat input on pause so the typed buffer isn't
            // committed when the player Esc's into the pause menu.
            .add_systems(OnEnter(crate::PauseState::Paused), close_chat_on_pause)
            .add_systems(
                Update,
                (
                    // Chat input is gated on `Running` so Q/M presses during
                    // the pause menu don't double-fire (pause menu disconnect
                    // + chat buffer append on the same frame).
                    chat_input_system
                        .in_set(ChatInputSet)
                        .run_if(in_state(crate::PauseState::Running)),
                    chat_age_lines,
                    chat_render_overlay,
                )
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn close_chat_on_pause(mut state: ResMut<ChatInputState>) {
    state.open = false;
    state.buffer.clear();
}

fn spawn_chat_overlay(mut commands: Commands, assets: Res<UiAssets>) {
    let font = assets.font.clone();
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    left: Val::Px(20.0),
                    bottom: Val::Px(180.0),
                    width: Val::Px(640.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::FlexEnd,
                    row_gap: Val::Px(LINE_GAP_PX),
                    ..default()
                },
                z_index: ZIndex::Global(50),
                ..default()
            },
            ChatRoot,
        ))
        .with_children(|parent| {
            // History stack — pre-allocated lines, hidden when no message.
            parent
                .spawn((
                    NodeBundle {
                        style: Style {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(LINE_GAP_PX),
                            ..default()
                        },
                        ..default()
                    },
                    ChatHistoryRoot,
                ))
                .with_children(|stack| {
                    for slot in 0..MAX_LINES {
                        stack.spawn((
                            TextBundle::from_section(
                                "",
                                TextStyle {
                                    font: font.clone(),
                                    font_size: LINE_FONT_SIZE,
                                    color: Color::srgba(0.0, 0.0, 0.0, 0.0),
                                },
                            ),
                            ChatHistoryLine { slot },
                        ));
                    }
                });

            // Input box — only visible while typing.
            parent
                .spawn((
                    NodeBundle {
                        style: Style {
                            margin: UiRect::top(Val::Px(8.0)),
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        background_color: BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
                        visibility: Visibility::Hidden,
                        ..default()
                    },
                    ChatInputBox,
                ))
                .with_children(|row| {
                    row.spawn((
                        TextBundle::from_section(
                            "",
                            TextStyle {
                                font,
                                font_size: 16.0,
                                color: Color::srgb(1.0, 0.95, 0.7),
                            },
                        ),
                        ChatInputText,
                    ));
                });
        });
}

fn despawn_chat_overlay(mut commands: Commands, q: Query<Entity, With<ChatRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn reset_chat_state(mut state: ResMut<ChatInputState>, mut log: ResMut<ChatLog>) {
    state.open = false;
    state.buffer.clear();
    log.lines.clear();
}

#[allow(clippy::too_many_arguments)]
fn chat_input_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<ChatInputState>,
    mut log: ResMut<ChatLog>,
    ctx: Res<NetContext>,
    net: Res<NetMode>,
    local_nick: Res<LocalNickname>,
) {
    // Open/close hotkey: T toggles open, Escape closes.  When closed, the T
    // key starts an empty buffer; when open, every other key flows into the
    // buffer (Enter to send, Backspace to delete).
    if !state.open {
        if keys.just_pressed(KeyCode::KeyT) {
            state.open = true;
            state.buffer.clear();
        }
        return;
    }

    if keys.just_pressed(KeyCode::Escape) {
        state.open = false;
        state.buffer.clear();
        return;
    }

    if keys.just_pressed(KeyCode::Enter) {
        let buf = std::mem::take(&mut state.buffer);
        state.open = false;
        if let Some(clean) = sanitize_chat(&buf) {
            send_local_chat(&clean, &ctx, &net, &local_nick, &mut log);
        }
        return;
    }

    if keys.just_pressed(KeyCode::Backspace) {
        state.buffer.pop();
        return;
    }

    for key in keys.get_just_pressed() {
        // Letter keys produce uppercase — matches the nickname renderer.
        if let Some(c) = keycode_to_letter(*key) {
            push_char(&mut state.buffer, c);
        } else if let Some(d) = keycode_to_digit(*key) {
            push_char(&mut state.buffer, d);
        } else if matches!(key, KeyCode::Space) {
            push_char(&mut state.buffer, ' ');
        } else if matches!(key, KeyCode::Period | KeyCode::NumpadDecimal) {
            push_char(&mut state.buffer, '.');
        } else if matches!(key, KeyCode::Comma) {
            push_char(&mut state.buffer, ',');
        } else if matches!(key, KeyCode::Slash) {
            push_char(&mut state.buffer, '?');
        } else if matches!(key, KeyCode::Minus | KeyCode::NumpadSubtract) {
            push_char(&mut state.buffer, '-');
        } else if matches!(key, KeyCode::Equal | KeyCode::NumpadAdd) {
            push_char(&mut state.buffer, '+');
        } else if matches!(key, KeyCode::Quote) {
            push_char(&mut state.buffer, '\'');
        } else if matches!(key, KeyCode::Semicolon) {
            push_char(&mut state.buffer, ':');
        }
    }
}

fn push_char(buf: &mut String, c: char) {
    if buf.chars().count() < CHAT_MAX_LEN {
        buf.push(c);
    }
}

/// Resolve our own author name for outgoing chat.  Host uses `LocalNickname`;
/// clients use the same (server displays the nickname they Hello'd with).
fn local_author(local_nick: &LocalNickname) -> String {
    if local_nick.0.trim().is_empty() {
        "GRACZ".to_string()
    } else {
        local_nick.0.clone()
    }
}

fn send_local_chat(
    text: &str,
    ctx: &NetContext,
    net: &NetMode,
    local_nick: &LocalNickname,
    log: &mut ChatLog,
) {
    match *net {
        NetMode::SinglePlayer => {
            log.push(local_author(local_nick), text.to_string());
        }
        NetMode::Host => {
            let author = local_author(local_nick);
            log.push(author.clone(), text.to_string());
            if let Some(host) = ctx.host.as_ref() {
                broadcast(
                    host,
                    &ServerMsg::Chat {
                        author,
                        text: text.to_string(),
                    },
                );
            }
        }
        NetMode::Client => {
            // Clients send to host and let the broadcast echo back so the
            // line appears in our own log too — keeps everyone's view in
            // sync with whatever the server decided to emit.  If the send
            // itself fails (writer thread gone → channel rx dropped) we
            // echo locally as a fallback so the player still sees their
            // own message instead of typing into the void.
            let send_ok = ctx
                .client
                .as_ref()
                .map(|c| {
                    c.sender
                        .send(ClientMsg::Chat {
                            text: text.to_string(),
                        })
                        .is_ok()
                })
                .unwrap_or(false);
            if !send_ok {
                log.push(local_author(local_nick), text.to_string());
            }
        }
    }
}

fn chat_age_lines(time: Res<Time>, mut log: ResMut<ChatLog>) {
    let dt = time.delta_seconds();
    for line in log.lines.iter_mut() {
        line.age_secs += dt;
    }
}

fn chat_render_overlay(
    state: Res<ChatInputState>,
    log: Res<ChatLog>,
    mut history: Query<(&ChatHistoryLine, &mut Text)>,
    mut input_box: Query<&mut Visibility, With<ChatInputBox>>,
    mut input_text: Query<&mut Text, (With<ChatInputText>, Without<ChatHistoryLine>)>,
) {
    // Render history bottom-up so newest sits closest to the input box.
    // Only write to a `Text` when its content actually changes — a mutable
    // deref marks the component `Changed` and re-triggers text shaping and
    // UI layout every frame, a constant background cost on mobile.
    let count = log.lines.len().min(MAX_LINES);

    for (line_marker, mut text) in history.iter_mut() {
        // Slot 0 is the bottom-most rendered line; slot N-1 is at the top.
        // We want newest at the bottom, so map slot i → visible[count-1-i].
        let idx_from_top = MAX_LINES - 1 - line_marker.slot;
        if idx_from_top >= count {
            // Blank slots stay transparent whenever the value is empty, so
            // one guard covers both writes.
            if !text.sections[0].value.is_empty() {
                text.sections[0].value.clear();
                text.sections[0].style.color = Color::srgba(0.0, 0.0, 0.0, 0.0);
            }
            continue;
        }
        // Newest line sits at the back of the deque, so index from the back
        // directly instead of collecting a reversed snapshot per frame.
        let line = &log.lines[log.lines.len() - 1 - idx_from_top];
        // Stay open while typing so the player has full context; otherwise
        // fade out after the lifetime threshold.
        let alpha = if state.open {
            1.0
        } else {
            let remaining = (CHAT_LINE_LIFETIME_SECS - line.age_secs)
                .clamp(0.0, CHAT_LINE_LIFETIME_SECS);
            // Linear fade over the last 1.5 s of life.
            (remaining / 1.5).clamp(0.0, 1.0)
        };
        if alpha <= 0.005 {
            if !text.sections[0].value.is_empty() {
                text.sections[0].value.clear();
                text.sections[0].style.color = Color::srgba(0.0, 0.0, 0.0, 0.0);
            }
            continue;
        }
        // Outside the fade window alpha is a constant 1.0, so these guards
        // skip the write on every frame where nothing moved.
        let rendered = format!("{}: {}", line.author, line.text);
        if text.sections[0].value != rendered {
            text.sections[0].value = rendered;
        }
        let color = Color::srgba(1.0, 0.95, 0.85, alpha);
        if text.sections[0].style.color != color {
            text.sections[0].style.color = color;
        }
    }

    // Input box — visible only while typing.  Same guarded writes so an
    // idle overlay doesn't dirty the UI tree every frame.
    if let Ok(mut vis) = input_box.get_single_mut() {
        let desired = if state.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *vis != desired {
            *vis = desired;
        }
    }
    if let Ok(mut text) = input_text.get_single_mut() {
        if state.open {
            let rendered = format!("> {}_", state.buffer);
            if text.sections[0].value != rendered {
                text.sections[0].value = rendered;
            }
        } else if !text.sections[0].value.is_empty() {
            text.sections[0].value.clear();
        }
    }
}

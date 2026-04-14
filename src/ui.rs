use bevy::prelude::*;

use crate::net::{NetContext, NetMode};
use crate::player::{Player, PLAYER_MAX_HP};
use crate::wave::WaveState;
use crate::zombie::Zombie;
use crate::{GameState, Score, UiAssets};

#[derive(Component)]
pub struct HudRoot;

#[derive(Component)]
pub struct HpBarFill;

#[derive(Component)]
pub struct HpText;

#[derive(Component)]
pub struct WaveTitleText;

#[derive(Component)]
pub struct WaveStatusText;

#[derive(Component)]
pub struct ScoreValueText;

#[derive(Component)]
pub struct GameOverUi;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), spawn_hud)
            .add_systems(OnExit(GameState::Playing), despawn_hud)
            .add_systems(Update, update_hud.run_if(in_state(GameState::Playing)))
            .add_systems(OnEnter(GameState::GameOver), spawn_game_over)
            .add_systems(OnExit(GameState::GameOver), despawn_game_over)
            .add_systems(
                Update,
                game_over_input.run_if(in_state(GameState::GameOver)),
            );
    }
}

fn panel_bg() -> BackgroundColor {
    BackgroundColor(Color::srgba(0.04, 0.04, 0.06, 0.85))
}

fn panel_border() -> BorderColor {
    BorderColor(Color::srgb(0.35, 0.35, 0.4))
}

fn spawn_hud(mut commands: Commands, assets: Res<UiAssets>) {
    let font = assets.font.clone();
    let label = TextStyle {
        font: font.clone(),
        font_size: 11.0,
        color: Color::srgb(0.65, 0.65, 0.65),
    };
    let value = TextStyle {
        font: font.clone(),
        font_size: 16.0,
        color: Color::srgb(1.0, 0.95, 0.85),
    };
    let wave_style = TextStyle {
        font: font.clone(),
        font_size: 22.0,
        color: Color::srgb(1.0, 0.82, 0.2),
    };
    let status_style = TextStyle {
        font: font.clone(),
        font_size: 10.0,
        color: Color::srgb(0.75, 0.75, 0.75),
    };

    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(14.0),
                    left: Val::Px(14.0),
                    right: Val::Px(14.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::FlexStart,
                    column_gap: Val::Px(14.0),
                    ..default()
                },
                ..default()
            },
            HudRoot,
        ))
        .with_children(|parent| {
            parent
                .spawn(NodeBundle {
                    style: Style {
                        padding: UiRect::all(Val::Px(12.0)),
                        border: UiRect::all(Val::Px(2.0)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        ..default()
                    },
                    background_color: panel_bg(),
                    border_color: panel_border(),
                    ..default()
                })
                .with_children(|panel| {
                    panel.spawn(TextBundle::from_section("HP", label.clone()));
                    panel
                        .spawn(NodeBundle {
                            style: Style {
                                flex_direction: FlexDirection::Row,
                                column_gap: Val::Px(10.0),
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn(NodeBundle {
                                style: Style {
                                    width: Val::Px(188.0),
                                    height: Val::Px(16.0),
                                    border: UiRect::all(Val::Px(1.0)),
                                    padding: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                background_color: BackgroundColor(Color::srgb(
                                    0.14, 0.03, 0.03,
                                )),
                                border_color: BorderColor(Color::srgb(0.32, 0.1, 0.1)),
                                ..default()
                            })
                            .with_children(|bar| {
                                bar.spawn((
                                    NodeBundle {
                                        style: Style {
                                            width: Val::Percent(100.0),
                                            height: Val::Percent(100.0),
                                            ..default()
                                        },
                                        background_color: BackgroundColor(Color::srgb(
                                            0.86, 0.18, 0.18,
                                        )),
                                        ..default()
                                    },
                                    HpBarFill,
                                ));
                            });
                            row.spawn((
                                TextBundle::from_section("100/100", value.clone()),
                                HpText,
                            ));
                        });
                });

            parent
                .spawn(NodeBundle {
                    style: Style {
                        padding: UiRect::all(Val::Px(12.0)),
                        border: UiRect::all(Val::Px(2.0)),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(6.0),
                        min_width: Val::Px(200.0),
                        ..default()
                    },
                    background_color: panel_bg(),
                    border_color: panel_border(),
                    ..default()
                })
                .with_children(|panel| {
                    panel.spawn((
                        TextBundle::from_section("FALA 1", wave_style.clone()),
                        WaveTitleText,
                    ));
                    panel.spawn((
                        TextBundle::from_section("", status_style.clone()),
                        WaveStatusText,
                    ));
                });

            parent
                .spawn(NodeBundle {
                    style: Style {
                        padding: UiRect::all(Val::Px(12.0)),
                        border: UiRect::all(Val::Px(2.0)),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexEnd,
                        row_gap: Val::Px(8.0),
                        min_width: Val::Px(130.0),
                        ..default()
                    },
                    background_color: panel_bg(),
                    border_color: panel_border(),
                    ..default()
                })
                .with_children(|panel| {
                    panel.spawn(TextBundle::from_section("SCORE", label.clone()));
                    panel.spawn((
                        TextBundle::from_section("0", value.clone()),
                        ScoreValueText,
                    ));
                });
        });
}

fn despawn_hud(mut commands: Commands, q: Query<Entity, With<HudRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

#[allow(clippy::too_many_arguments)]
fn update_hud(
    ctx: Res<NetContext>,
    players: Query<&Player>,
    score: Res<Score>,
    wave: Res<WaveState>,
    zombies: Query<(), With<Zombie>>,
    mut hp_bar: Query<&mut Style, With<HpBarFill>>,
    mut hp_text: Query<
        &mut Text,
        (
            With<HpText>,
            Without<WaveTitleText>,
            Without<WaveStatusText>,
            Without<ScoreValueText>,
        ),
    >,
    mut wave_title: Query<
        &mut Text,
        (
            With<WaveTitleText>,
            Without<HpText>,
            Without<WaveStatusText>,
            Without<ScoreValueText>,
        ),
    >,
    mut wave_status: Query<
        &mut Text,
        (
            With<WaveStatusText>,
            Without<HpText>,
            Without<WaveTitleText>,
            Without<ScoreValueText>,
        ),
    >,
    mut score_text: Query<
        &mut Text,
        (
            With<ScoreValueText>,
            Without<HpText>,
            Without<WaveTitleText>,
            Without<WaveStatusText>,
        ),
    >,
) {
    let Some(player) = players.iter().find(|p| p.id == ctx.my_id) else {
        return;
    };
    let hp_pct = (player.hp.max(0) as f32 / PLAYER_MAX_HP as f32 * 100.0).clamp(0.0, 100.0);
    if let Ok(mut style) = hp_bar.get_single_mut() {
        style.width = Val::Percent(hp_pct);
    }
    if let Ok(mut text) = hp_text.get_single_mut() {
        text.sections[0].value = format!("{}/{}", player.hp.max(0), PLAYER_MAX_HP);
    }
    if let Ok(mut text) = wave_title.get_single_mut() {
        text.sections[0].value = if wave.in_break && wave.current_wave == 0 {
            "PRZYGOTUJ SIE".to_string()
        } else if wave.in_break {
            format!("FALA {} ZA...", wave.current_wave + 1)
        } else {
            format!("FALA {}", wave.current_wave)
        };
    }
    if let Ok(mut text) = wave_status.get_single_mut() {
        text.sections[0].value = if wave.in_break {
            format!("{:.1}s", wave.break_timer.remaining_secs())
        } else {
            let alive = zombies.iter().count();
            let left = wave.zombies_to_spawn as usize + alive;
            format!("ZOMBIE: {left}")
        };
    }
    if let Ok(mut text) = score_text.get_single_mut() {
        text.sections[0].value = format!("{}", score.0);
    }
}

fn spawn_game_over(
    mut commands: Commands,
    score: Res<Score>,
    wave: Res<WaveState>,
    assets: Res<UiAssets>,
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
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(22.0),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.78)),
                ..default()
            },
            GameOverUi,
        ))
        .with_children(|parent| {
            parent.spawn(TextBundle::from_section(
                "GAME OVER",
                TextStyle {
                    font: font.clone(),
                    font_size: 64.0,
                    color: Color::srgb(0.9, 0.18, 0.18),
                },
            ));
            parent.spawn(TextBundle::from_section(
                format!("FALA {}    SCORE {}", wave.current_wave, score.0),
                TextStyle {
                    font: font.clone(),
                    font_size: 22.0,
                    color: Color::WHITE,
                },
            ));
            parent.spawn(TextBundle::from_section(
                "SPACJA - menu glowne",
                TextStyle {
                    font,
                    font_size: 16.0,
                    color: Color::srgb(0.75, 0.75, 0.75),
                },
            ));
        });
}

fn despawn_game_over(mut commands: Commands, q: Query<Entity, With<GameOverUi>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn game_over_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut ctx: ResMut<NetContext>,
    mut mode: ResMut<NetMode>,
) {
    if keys.just_pressed(KeyCode::Space) || keys.just_pressed(KeyCode::Enter) {
        ctx.disconnect();
        *mode = NetMode::SinglePlayer;
        next_state.set(GameState::Menu);
    }
}

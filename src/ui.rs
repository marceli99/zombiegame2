use std::collections::VecDeque;

use bevy::prelude::*;

use crate::net::{NetContext, NetMode};
use crate::player::{Player, PLAYER_MAX_HP};
use crate::settings::GraphicsSettings;
use crate::wave::WaveState;
use crate::weapon::{ThrowableAssets, ThrowableKind, WeaponAssets};
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
pub struct WeaponText;

#[derive(Component)]
pub struct AmmoText;

#[derive(Component)]
pub struct FpsCounterRoot;

#[derive(Component)]
pub struct FpsCounterText;

#[derive(Component)]
pub struct GameOverUi;

#[derive(Component)]
pub struct SlotIcon(pub u8);

#[derive(Component)]
pub struct SlotBorder(pub u8);

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), spawn_hud)
            .add_systems(OnExit(GameState::Playing), despawn_hud)
            .add_systems(
                Update,
                (update_hud, update_slot_icons, update_fps_counter)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(OnEnter(GameState::GameOver), spawn_game_over)
            .add_systems(OnExit(GameState::GameOver), despawn_game_over)
            .add_systems(
                Update,
                game_over_input.run_if(in_state(GameState::GameOver)),
            );
    }
}

fn spawn_hud(
    mut commands: Commands,
    assets: Res<UiAssets>,
    weapon_assets: Res<WeaponAssets>,
    throwable_assets: Res<ThrowableAssets>,
) {
    let font = assets.font.clone();

    // ── Bottom-left: Slots + Weapon + Ammo + HP ──
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(16.0),
                    left: Val::Px(16.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(5.0),
                    ..default()
                },
                ..default()
            },
            HudRoot,
        ))
        .with_children(|parent| {
            // Slot icons row
            parent
                .spawn(NodeBundle {
                    style: Style {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(3.0),
                        ..default()
                    },
                    ..default()
                })
                .with_children(|row| {
                    for i in 0..3u8 {
                        let active = Color::srgba(1.0, 0.85, 0.2, 0.95);
                        let inactive = Color::srgba(0.4, 0.4, 0.45, 0.4);
                        let bc = if i == 0 { active } else { inactive };
                        let img = match i {
                            0 => weapon_assets.images[0].clone(),
                            2 => throwable_assets.grenade.clone(),
                            _ => weapon_assets.images[0].clone(),
                        };
                        let vis = if i == 1 {
                            Visibility::Hidden
                        } else {
                            Visibility::Inherited
                        };
                        row.spawn((
                            NodeBundle {
                                style: Style {
                                    width: Val::Px(34.0),
                                    height: Val::Px(34.0),
                                    border: UiRect::all(Val::Px(1.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                border_color: BorderColor(bc),
                                background_color: BackgroundColor(
                                    Color::srgba(0.06, 0.06, 0.08, 0.5),
                                ),
                                ..default()
                            },
                            SlotBorder(i),
                        ))
                        .with_children(|slot| {
                            slot.spawn((
                                ImageBundle {
                                    style: Style {
                                        width: Val::Px(24.0),
                                        height: Val::Px(24.0),
                                        ..default()
                                    },
                                    image: UiImage::new(img),
                                    visibility: vis,
                                    ..default()
                                },
                                SlotIcon(i),
                            ));
                        });
                    }
                });
            // Weapon name
            parent.spawn((
                TextBundle::from_section(
                    "[1] PISTOL",
                    TextStyle {
                        font: font.clone(),
                        font_size: 10.0,
                        color: Color::srgba(1.0, 0.88, 0.35, 0.9),
                    },
                ),
                WeaponText,
            ));
            // Ammo
            parent.spawn((
                TextBundle::from_section(
                    "",
                    TextStyle {
                        font: font.clone(),
                        font_size: 9.0,
                        color: Color::srgba(0.65, 0.65, 0.6, 0.8),
                    },
                ),
                AmmoText,
            ));
            // HP bar row
            parent
                .spawn(NodeBundle {
                    style: Style {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(6.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(NodeBundle {
                        style: Style {
                            width: Val::Px(140.0),
                            height: Val::Px(5.0),
                            ..default()
                        },
                        background_color: BackgroundColor(Color::srgba(
                            0.15, 0.03, 0.03, 0.5,
                        )),
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
                                    0.9, 0.18, 0.18,
                                )),
                                ..default()
                            },
                            HpBarFill,
                        ));
                    });
                    row.spawn((
                        TextBundle::from_section(
                            "100",
                            TextStyle {
                                font: font.clone(),
                                font_size: 9.0,
                                color: Color::srgba(0.85, 0.3, 0.3, 0.85),
                            },
                        ),
                        HpText,
                    ));
                });
        });

    // ── Top-center: Wave ──
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(10.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                ..default()
            },
            HudRoot,
        ))
        .with_children(|parent| {
            parent.spawn((
                TextBundle::from_section(
                    "WAVE 1",
                    TextStyle {
                        font: font.clone(),
                        font_size: 16.0,
                        color: Color::srgba(1.0, 0.85, 0.25, 0.85),
                    },
                ),
                WaveTitleText,
            ));
            parent.spawn((
                TextBundle::from_section(
                    "",
                    TextStyle {
                        font: font.clone(),
                        font_size: 8.0,
                        color: Color::srgba(0.7, 0.7, 0.7, 0.6),
                    },
                ),
                WaveStatusText,
            ));
        });

    // ── Top-right: Score ──
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(10.0),
                    right: Val::Px(16.0),
                    ..default()
                },
                ..default()
            },
            HudRoot,
        ))
        .with_children(|parent| {
            parent.spawn((
                TextBundle::from_section(
                    "$0",
                    TextStyle {
                        font: font.clone(),
                        font_size: 13.0,
                        color: Color::srgba(0.55, 0.95, 0.4, 0.9),
                    },
                ),
                ScoreValueText,
            ));
        });

    // FPS counter (bottom-right, hidden by default)
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(14.0),
                    right: Val::Px(14.0),
                    ..default()
                },
                visibility: Visibility::Hidden,
                ..default()
            },
            HudRoot,
            FpsCounterRoot,
        ))
        .with_children(|node| {
            node.spawn((
                TextBundle::from_section(
                    "",
                    TextStyle {
                        font,
                        font_size: 9.0,
                        color: Color::srgba(0.5, 0.5, 0.5, 0.6),
                    },
                ),
                FpsCounterText,
            ));
        });
}

fn despawn_hud(mut commands: Commands, q: Query<Entity, With<HudRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
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
            Without<WeaponText>,
            Without<AmmoText>,
        ),
    >,
    mut wave_title: Query<
        &mut Text,
        (
            With<WaveTitleText>,
            Without<HpText>,
            Without<WaveStatusText>,
            Without<ScoreValueText>,
            Without<WeaponText>,
            Without<AmmoText>,
        ),
    >,
    mut wave_status: Query<
        &mut Text,
        (
            With<WaveStatusText>,
            Without<HpText>,
            Without<WaveTitleText>,
            Without<ScoreValueText>,
            Without<WeaponText>,
            Without<AmmoText>,
        ),
    >,
    mut score_text: Query<
        &mut Text,
        (
            With<ScoreValueText>,
            Without<HpText>,
            Without<WaveTitleText>,
            Without<WaveStatusText>,
            Without<WeaponText>,
            Without<AmmoText>,
        ),
    >,
    mut weapon_text: Query<
        &mut Text,
        (
            With<WeaponText>,
            Without<HpText>,
            Without<WaveTitleText>,
            Without<WaveStatusText>,
            Without<ScoreValueText>,
            Without<AmmoText>,
        ),
    >,
    mut ammo_text: Query<
        &mut Text,
        (
            With<AmmoText>,
            Without<HpText>,
            Without<WaveTitleText>,
            Without<WaveStatusText>,
            Without<ScoreValueText>,
            Without<WeaponText>,
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
        text.sections[0].value = format!("{}", player.hp.max(0));
    }
    // Weapon / slot display
    if let Ok(mut text) = weapon_text.get_single_mut() {
        let slot = player.active_slot;
        if slot <= 1 {
            let label = player.active_weapon().label();
            text.sections[0].value = format!("[{}] {}", slot + 1, label);
        } else {
            text.sections[0].value = format!("[3] {}", player.throwable_kind.label());
        }
    }
    // Ammo display
    if let Ok(mut text) = ammo_text.get_single_mut() {
        let slot = player.active_slot as usize;
        if slot <= 1 {
            let weapon = player.active_weapon();
            if weapon.has_infinite_ammo() {
                text.sections[0].value = "INF".to_string();
            } else if player.reload_timer > 0.0 {
                text.sections[0].value = "RELOADING...".to_string();
            } else {
                text.sections[0].value = format!(
                    "{}/{}  [{}]",
                    player.ammo[slot],
                    weapon.magazine_size(),
                    player.reserve_ammo[slot],
                );
            }
        } else {
            text.sections[0].value = format!("x{}", player.throwable_count);
        }
    }
    if let Ok(mut text) = wave_title.get_single_mut() {
        text.sections[0].value = if wave.in_break && wave.current_wave == 0 {
            "GET READY".to_string()
        } else if wave.in_break {
            format!("WAVE {} IN...", wave.current_wave + 1)
        } else {
            format!("WAVE {}", wave.current_wave)
        };
    }
    if let Ok(mut text) = wave_status.get_single_mut() {
        text.sections[0].value = if wave.in_break {
            format!("{:.1}s", wave.break_timer.remaining_secs())
        } else {
            let alive = zombies.iter().count();
            let left = wave.zombies_to_spawn as usize + alive;
            format!("ZOMBIES: {left}")
        };
    }
    if let Ok(mut text) = score_text.get_single_mut() {
        text.sections[0].value = format!("${}", score.0);
    }
}

fn update_slot_icons(
    ctx: Res<NetContext>,
    players: Query<&Player>,
    weapon_assets: Res<WeaponAssets>,
    throwable_assets: Res<ThrowableAssets>,
    mut icons: Query<(&mut UiImage, &mut Visibility, &SlotIcon)>,
    mut borders: Query<(&mut BorderColor, &SlotBorder)>,
) {
    let Some(player) = players.iter().find(|p| p.id == ctx.my_id) else {
        return;
    };
    for (mut img, mut vis, slot) in &mut icons {
        match slot.0 {
            0 | 1 => {
                let idx = slot.0 as usize;
                if let Some(w) = player.slots[idx] {
                    img.texture = weapon_assets.images[w.as_u8() as usize].clone();
                    *vis = Visibility::Inherited;
                } else {
                    *vis = Visibility::Hidden;
                }
            }
            2 => {
                if player.throwable_count > 0 {
                    img.texture = match player.throwable_kind {
                        ThrowableKind::Grenade => throwable_assets.grenade.clone(),
                        ThrowableKind::Smoke => throwable_assets.smoke.clone(),
                        ThrowableKind::Molotov => throwable_assets.molotov.clone(),
                    };
                    *vis = Visibility::Inherited;
                } else {
                    *vis = Visibility::Hidden;
                }
            }
            _ => {}
        }
    }
    for (mut border, slot) in &mut borders {
        border.0 = if slot.0 == player.active_slot {
            Color::srgba(1.0, 0.85, 0.2, 0.95)
        } else {
            Color::srgba(0.4, 0.4, 0.45, 0.4)
        };
    }
}

#[allow(clippy::type_complexity)]
fn update_fps_counter(
    time: Res<Time>,
    settings: Res<GraphicsSettings>,
    mut root_vis: Query<&mut Visibility, With<FpsCounterRoot>>,
    mut fps_text: Query<
        &mut Text,
        (
            With<FpsCounterText>,
            Without<HpText>,
            Without<WaveTitleText>,
            Without<WaveStatusText>,
            Without<ScoreValueText>,
            Without<WeaponText>,
            Without<AmmoText>,
        ),
    >,
    mut history: Local<VecDeque<f32>>,
) {
    let dt = time.delta_seconds();
    if dt > 0.0 {
        history.push_back(dt);
    }
    while history.len() > 60 {
        history.pop_front();
    }

    if let Ok(mut vis) = root_vis.get_single_mut() {
        *vis = if settings.show_fps {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    if settings.show_fps {
        if let Ok(mut text) = fps_text.get_single_mut() {
            let avg: f32 = if history.is_empty() {
                0.0
            } else {
                history.iter().sum::<f32>() / history.len() as f32
            };
            let fps = if avg > 0.0 {
                (1.0 / avg).round() as u32
            } else {
                0
            };
            text.sections[0].value = format!("{fps} FPS");
        }
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
                format!("WAVE {}    ${}", wave.current_wave, score.0),
                TextStyle {
                    font: font.clone(),
                    font_size: 22.0,
                    color: Color::WHITE,
                },
            ));
            parent.spawn(TextBundle::from_section(
                "SPACE - main menu",
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

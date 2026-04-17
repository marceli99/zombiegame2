use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use crate::audio::SfxEvent;
use crate::player::PlayerDamagedEvent;
use crate::wave::WaveState;
use crate::zombie::{ZombieKilledEvent, ZombieKind};
use crate::zones::ZoneState;
use crate::{GameState, UiAssets};

const TOAST_DURATION: f32 = 3.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AchievementId {
    FirstBlood,
    Centurion,
    Massacre,
    ChainReaction,
    ZombieHunter,
    WaveSurvivor10,
    WaveSurvivor20,
    Untouchable,
    FullArsenal,
    Explorer,
    Demolition,
    Speedrunner,
}

pub const ALL_ACHIEVEMENTS: [AchievementId; 12] = [
    AchievementId::FirstBlood,
    AchievementId::Centurion,
    AchievementId::Massacre,
    AchievementId::ChainReaction,
    AchievementId::ZombieHunter,
    AchievementId::WaveSurvivor10,
    AchievementId::WaveSurvivor20,
    AchievementId::Untouchable,
    AchievementId::FullArsenal,
    AchievementId::Explorer,
    AchievementId::Demolition,
    AchievementId::Speedrunner,
];

impl AchievementId {
    pub fn name(self) -> &'static str {
        match self {
            Self::FirstBlood => "FIRST BLOOD",
            Self::Centurion => "CENTURION",
            Self::Massacre => "MASSACRE",
            Self::ChainReaction => "CHAIN REACTION",
            Self::ZombieHunter => "ZOMBIE HUNTER",
            Self::WaveSurvivor10 => "VETERAN",
            Self::WaveSurvivor20 => "LEGEND",
            Self::Untouchable => "UNTOUCHABLE",
            Self::FullArsenal => "FULL ARSENAL",
            Self::Explorer => "EXPLORER",
            Self::Demolition => "DEMOLITION",
            Self::Speedrunner => "SPEEDRUNNER",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::FirstBlood => "Kill your first zombie",
            Self::Centurion => "Kill 100 zombies in a single game",
            Self::Massacre => "Kill 100 zombies in 100 seconds",
            Self::ChainReaction => "Kill 5 exploders in 10 seconds",
            Self::ZombieHunter => "Kill 1000 zombies total",
            Self::WaveSurvivor10 => "Reach wave 10",
            Self::WaveSurvivor20 => "Reach wave 20",
            Self::Untouchable => "Complete a wave without taking damage",
            Self::FullArsenal => "Buy every weapon from shops",
            Self::Explorer => "Unlock all zones",
            Self::Demolition => "Kill 20 zombies with explosions in one game",
            Self::Speedrunner => "Reach wave 5 in under 3 minutes",
        }
    }
}

// ── Persistence ───────────────────────────────────────────────────

fn save_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".zombiegame2")
}

#[derive(Resource, Default, Serialize, Deserialize)]
pub struct AchievementSave {
    pub unlocked: HashSet<AchievementId>,
    pub total_kills: u64,
}

impl AchievementSave {
    pub fn load() -> Self {
        let mut path = save_dir();
        path.push("save.json");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let dir = save_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mut path = dir;
        path.push("save.json");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

// ── Per-game tracker ──────────────────────────────────────────────

#[derive(Resource)]
pub struct AchievementTracker {
    pub game_kills: u32,
    pub explosion_kills: u32,
    pub exploder_kill_times: VecDeque<f64>,
    pub kill_times: VecDeque<f64>,
    pub weapons_bought: u8,
    pub took_damage_this_wave: bool,
    pub prev_in_break: bool,
    pub game_start_time: f64,
    pub pending_toasts: VecDeque<AchievementId>,
}

impl Default for AchievementTracker {
    fn default() -> Self {
        Self {
            game_kills: 0,
            explosion_kills: 0,
            exploder_kill_times: VecDeque::new(),
            kill_times: VecDeque::new(),
            weapons_bought: 0,
            took_damage_this_wave: false,
            prev_in_break: true,
            game_start_time: 0.0,
            pending_toasts: VecDeque::new(),
        }
    }
}

// ── Components ────────────────────────────────────────────────────

#[derive(Component)]
struct AchievementToast {
    timer: f32,
}

#[derive(Component)]
struct AchievementMenuRoot;

// ── Plugin ────────────────────────────────────────────────────────

pub struct AchievementsPlugin;

impl Plugin for AchievementsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(AchievementSave::load())
            .init_resource::<AchievementTracker>()
            .add_systems(OnEnter(GameState::Playing), reset_tracker)
            .add_systems(OnExit(GameState::Playing), save_on_exit)
            .add_systems(
                Update,
                (track_kills, track_damage, check_achievements)
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(Update, update_toasts)
            .add_systems(OnEnter(GameState::Achievements), spawn_achievement_menu)
            .add_systems(OnExit(GameState::Achievements), despawn_achievement_menu)
            .add_systems(
                Update,
                achievement_menu_input.run_if(in_state(GameState::Achievements)),
            );
    }
}

// ── Systems ───────────────────────────────────────────────────────

fn reset_tracker(mut tracker: ResMut<AchievementTracker>, time: Res<Time>) {
    *tracker = AchievementTracker {
        game_start_time: time.elapsed_seconds_f64(),
        prev_in_break: true,
        ..default()
    };
}

fn save_on_exit(tracker: Res<AchievementTracker>, mut save: ResMut<AchievementSave>) {
    save.total_kills += tracker.game_kills as u64;
    save.save();
}

fn track_kills(
    mut events: EventReader<ZombieKilledEvent>,
    time: Res<Time>,
    mut tracker: ResMut<AchievementTracker>,
) {
    let now = time.elapsed_seconds_f64();
    for ev in events.read() {
        tracker.game_kills += 1;
        tracker.kill_times.push_back(now);
        if ev.by_explosion {
            tracker.explosion_kills += 1;
        }
        if ev.kind == ZombieKind::Exploder {
            tracker.exploder_kill_times.push_back(now);
        }
    }
    while tracker
        .kill_times
        .front()
        .is_some_and(|&t| now - t > 100.0)
    {
        tracker.kill_times.pop_front();
    }
    while tracker
        .exploder_kill_times
        .front()
        .is_some_and(|&t| now - t > 10.0)
    {
        tracker.exploder_kill_times.pop_front();
    }
}

fn track_damage(
    mut events: EventReader<PlayerDamagedEvent>,
    mut tracker: ResMut<AchievementTracker>,
) {
    for _ in events.read() {
        tracker.took_damage_this_wave = true;
    }
}

fn try_unlock(
    id: AchievementId,
    save: &mut AchievementSave,
    tracker: &mut AchievementTracker,
) -> bool {
    if save.unlocked.contains(&id) {
        return false;
    }
    save.unlocked.insert(id);
    tracker.pending_toasts.push_back(id);
    save.save();
    true
}

#[allow(clippy::too_many_arguments)]
fn check_achievements(
    time: Res<Time>,
    wave: Res<WaveState>,
    zone_state: Res<ZoneState>,
    mut tracker: ResMut<AchievementTracker>,
    mut save: ResMut<AchievementSave>,
    mut sfx: EventWriter<SfxEvent>,
) {
    let now = time.elapsed_seconds_f64();
    let mut any_new = false;

    if tracker.game_kills >= 1
        && try_unlock(AchievementId::FirstBlood, &mut save, &mut tracker)
    {
        any_new = true;
    }

    if tracker.game_kills >= 100
        && try_unlock(AchievementId::Centurion, &mut save, &mut tracker)
    {
        any_new = true;
    }

    if tracker.kill_times.len() >= 100
        && try_unlock(AchievementId::Massacre, &mut save, &mut tracker)
    {
        any_new = true;
    }

    if tracker.exploder_kill_times.len() >= 5
        && try_unlock(AchievementId::ChainReaction, &mut save, &mut tracker)
    {
        any_new = true;
    }

    if save.total_kills + tracker.game_kills as u64 >= 1000
        && try_unlock(AchievementId::ZombieHunter, &mut save, &mut tracker)
    {
        any_new = true;
    }

    if wave.current_wave >= 10
        && try_unlock(AchievementId::WaveSurvivor10, &mut save, &mut tracker)
    {
        any_new = true;
    }

    if wave.current_wave >= 20
        && try_unlock(AchievementId::WaveSurvivor20, &mut save, &mut tracker)
    {
        any_new = true;
    }

    // Untouchable: wave just ended without taking damage
    if wave.in_break
        && !tracker.prev_in_break
        && wave.current_wave > 0
        && !tracker.took_damage_this_wave
        && try_unlock(AchievementId::Untouchable, &mut save, &mut tracker)
    {
        any_new = true;
    }
    // Reset damage flag when new wave starts
    if !wave.in_break && tracker.prev_in_break {
        tracker.took_damage_this_wave = false;
    }
    tracker.prev_in_break = wave.in_break;

    // FullArsenal: all 7 shop weapons purchased (bits 1-7)
    if tracker.weapons_bought & 0b1111_1110 == 0b1111_1110
        && try_unlock(AchievementId::FullArsenal, &mut save, &mut tracker)
    {
        any_new = true;
    }

    if zone_state.unlocked.iter().all(|&u| u)
        && try_unlock(AchievementId::Explorer, &mut save, &mut tracker)
    {
        any_new = true;
    }

    if tracker.explosion_kills >= 20
        && try_unlock(AchievementId::Demolition, &mut save, &mut tracker)
    {
        any_new = true;
    }

    if wave.current_wave >= 5 {
        let elapsed = now - tracker.game_start_time;
        if elapsed < 180.0
            && try_unlock(AchievementId::Speedrunner, &mut save, &mut tracker)
        {
            any_new = true;
        }
    }

    if any_new {
        sfx.send(SfxEvent::MenuSelect);
    }
}

// ── Toast UI ──────────────────────────────────────────────────────

fn update_toasts(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<UiAssets>,
    mut tracker: ResMut<AchievementTracker>,
    mut toasts: Query<(Entity, &mut AchievementToast)>,
) {
    let dt = time.delta_seconds();

    if toasts.is_empty() {
        if let Some(id) = tracker.pending_toasts.pop_front() {
            spawn_toast(&mut commands, &assets, id);
        }
    }

    for (entity, mut toast) in &mut toasts {
        toast.timer += dt;
        if toast.timer >= TOAST_DURATION {
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn spawn_toast(commands: &mut Commands, assets: &UiAssets, id: AchievementId) {
    let font = assets.font.clone();
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(120.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                ..default()
            },
            AchievementToast { timer: 0.0 },
        ))
        .with_children(|root| {
            root.spawn(NodeBundle {
                style: Style {
                    padding: UiRect::new(
                        Val::Px(24.0),
                        Val::Px(24.0),
                        Val::Px(10.0),
                        Val::Px(10.0),
                    ),
                    border: UiRect::all(Val::Px(2.0)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.12, 0.1, 0.02, 0.92)),
                border_color: BorderColor(Color::srgb(0.82, 0.72, 0.28)),
                ..default()
            })
            .with_children(|panel| {
                panel.spawn(TextBundle::from_section(
                    "ACHIEVEMENT UNLOCKED!",
                    TextStyle {
                        font: font.clone(),
                        font_size: 10.0,
                        color: Color::srgb(0.82, 0.72, 0.28),
                    },
                ));
                panel.spawn(TextBundle::from_section(
                    id.name(),
                    TextStyle {
                        font: font.clone(),
                        font_size: 16.0,
                        color: Color::srgb(1.0, 0.95, 0.85),
                    },
                ));
                panel.spawn(TextBundle::from_section(
                    id.description(),
                    TextStyle {
                        font,
                        font_size: 9.0,
                        color: Color::srgb(0.65, 0.65, 0.65),
                    },
                ));
            });
        });
}

// ── Achievement Menu ──────────────────────────────────────────────

fn spawn_achievement_menu(
    mut commands: Commands,
    assets: Res<UiAssets>,
    save: Res<AchievementSave>,
) {
    let font = assets.font.clone();
    let unlocked_count = save.unlocked.len();
    let total = ALL_ACHIEVEMENTS.len();

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
                background_color: BackgroundColor(Color::srgb(0.012, 0.016, 0.022)),
                ..default()
            },
            AchievementMenuRoot,
        ))
        .with_children(|root| {
            root.spawn(NodeBundle {
                style: Style {
                    width: Val::Px(700.0),
                    max_height: Val::Percent(90.0),
                    padding: UiRect::all(Val::Px(28.0)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(10.0),
                    border: UiRect::all(Val::Px(3.0)),
                    overflow: Overflow::clip_y(),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.035, 0.04, 0.05, 0.94)),
                border_color: BorderColor(Color::srgb(0.22, 0.28, 0.32)),
                ..default()
            })
            .with_children(|panel| {
                panel.spawn(TextBundle::from_section(
                    "ACHIEVEMENTS",
                    TextStyle {
                        font: font.clone(),
                        font_size: 36.0,
                        color: Color::srgb(0.82, 0.72, 0.28),
                    },
                ));
                panel.spawn(TextBundle::from_section(
                    format!("{unlocked_count} / {total}"),
                    TextStyle {
                        font: font.clone(),
                        font_size: 14.0,
                        color: Color::srgb(0.55, 0.58, 0.62),
                    },
                ));
                panel.spawn(NodeBundle {
                    style: Style {
                        width: Val::Px(360.0),
                        height: Val::Px(1.0),
                        margin: UiRect::vertical(Val::Px(8.0)),
                        ..default()
                    },
                    background_color: BackgroundColor(Color::srgba(0.25, 0.28, 0.32, 0.65)),
                    ..default()
                });

                panel
                    .spawn(NodeBundle {
                        style: Style {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(6.0),
                            width: Val::Percent(100.0),
                            ..default()
                        },
                        ..default()
                    })
                    .with_children(|list| {
                        for &id in &ALL_ACHIEVEMENTS {
                            let is_unlocked = save.unlocked.contains(&id);
                            let icon = if is_unlocked { "[X]" } else { "[ ]" };
                            let name_color = if is_unlocked {
                                Color::srgb(0.82, 0.72, 0.28)
                            } else {
                                Color::srgb(0.35, 0.35, 0.4)
                            };
                            let desc_color = if is_unlocked {
                                Color::srgb(0.55, 0.58, 0.62)
                            } else {
                                Color::srgb(0.28, 0.28, 0.32)
                            };

                            list.spawn(NodeBundle {
                                style: Style {
                                    flex_direction: FlexDirection::Row,
                                    align_items: AlignItems::Center,
                                    column_gap: Val::Px(12.0),
                                    padding: UiRect::new(
                                        Val::Px(12.0),
                                        Val::Px(12.0),
                                        Val::Px(6.0),
                                        Val::Px(6.0),
                                    ),
                                    ..default()
                                },
                                background_color: BackgroundColor(Color::srgba(
                                    0.06, 0.06, 0.08, 0.6,
                                )),
                                ..default()
                            })
                            .with_children(|row| {
                                row.spawn(TextBundle::from_section(
                                    icon,
                                    TextStyle {
                                        font: font.clone(),
                                        font_size: 14.0,
                                        color: name_color,
                                    },
                                ));
                                row.spawn(NodeBundle {
                                    style: Style {
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(2.0),
                                        ..default()
                                    },
                                    ..default()
                                })
                                .with_children(|info| {
                                    info.spawn(TextBundle::from_section(
                                        id.name(),
                                        TextStyle {
                                            font: font.clone(),
                                            font_size: 14.0,
                                            color: name_color,
                                        },
                                    ));
                                    info.spawn(TextBundle::from_section(
                                        id.description(),
                                        TextStyle {
                                            font: font.clone(),
                                            font_size: 9.0,
                                            color: desc_color,
                                        },
                                    ));
                                });
                            });
                        }
                    });

                panel.spawn(NodeBundle {
                    style: Style {
                        width: Val::Px(360.0),
                        height: Val::Px(1.0),
                        margin: UiRect::vertical(Val::Px(8.0)),
                        ..default()
                    },
                    background_color: BackgroundColor(Color::srgba(0.25, 0.28, 0.32, 0.65)),
                    ..default()
                });
                panel.spawn(TextBundle::from_section(
                    "ESC - BACK",
                    TextStyle {
                        font,
                        font_size: 11.0,
                        color: Color::srgb(0.32, 0.34, 0.38),
                    },
                ));
            });
        });
}

fn despawn_achievement_menu(
    mut commands: Commands,
    q: Query<Entity, With<AchievementMenuRoot>>,
) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn achievement_menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut sfx: EventWriter<SfxEvent>,
) {
    if keys.just_pressed(KeyCode::Escape)
        || keys.just_pressed(KeyCode::Space)
        || keys.just_pressed(KeyCode::Enter)
    {
        sfx.send(SfxEvent::MenuCancel);
        next_state.set(GameState::Menu);
    }
}

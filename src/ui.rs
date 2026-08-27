use std::collections::VecDeque;

use bevy::prelude::*;

use crate::map::{building_floor_count, segment_name, PlayerFloorState, SegmentUnlockHint};
use crate::map_data::BUILDINGS;
use crate::net::{NetContext, NetMode, PlayerNicknames};
use crate::player::{
    player_palette_color, Player, PlayerDamagedEvent, PLAYER_ARMOR_MAX, PLAYER_MAX_HP,
};
use crate::settings::GraphicsSettings;
use crate::wave::WaveState;
use crate::zombie::{SpawnZombieEvent, ZombieKilledEvent, ZombieKind};
use crate::weapon::{PickupPromptHint, ThrowableAssets, ThrowableKind, Weapon, WeaponAssets};
use crate::zombie::Zombie;
use crate::{GameState, Score, UiAssets};

#[derive(Component)]
pub struct HudRoot;

#[derive(Component)]
pub struct HpBarFill;

#[derive(Component)]
pub struct HpText;

#[derive(Component)]
pub struct ArmorBarFill;

#[derive(Component)]
pub struct ArmorText;

#[derive(Component)]
pub struct FloorIndicatorRoot;

#[derive(Component)]
pub struct FloorIndicatorText;

#[derive(Component)]
pub struct PickupPromptRoot;

#[derive(Component)]
pub struct PickupPromptText;

#[derive(Component)]
pub struct WaveIntroRoot;

#[derive(Component)]
pub struct WaveIntroText;

/// Tracks the last wave we showed an intro for so we re-fire on each
/// new wave start (and not every frame the wave is active).
#[derive(Resource, Default)]
pub struct WaveIntroState {
    pub last_wave: u32,
    pub flash_remaining: f32,
}

const WAVE_INTRO_DURATION: f32 = 2.4;

#[derive(Component)]
pub struct ComboCounterRoot;

#[derive(Component)]
pub struct ComboCounterText;

#[derive(Component)]
pub struct BossAlertRoot;

#[derive(Component)]
pub struct BossAlertText;

/// Tracks the boss-alert flash so it can be triggered from the spawn
/// listener and animated by `update_boss_alert`.  Holds the remaining
/// flash time in seconds.
#[derive(Resource, Default)]
pub struct BossAlertTimer {
    pub remaining: f32,
}

const BOSS_ALERT_DURATION: f32 = 2.8;

/// Tracks the running kill chain for the combo HUD.  Resets after
/// `COMBO_TIMEOUT` seconds without a fresh kill.
#[derive(Resource, Default)]
pub struct ComboState {
    pub count: u32,
    pub time_since_kill: f32,
    pub punch_time: f32, // counts up since last kill, used for the bump anim
}

const COMBO_TIMEOUT: f32 = 2.6;
/// Duration of the combo-text punch animation in seconds (overshoot then
/// settle).  Read by `update_combo` to drive the scale curve.
const COMBO_PUNCH_DURATION: f32 = 0.25;
/// Peak overshoot scale on a fresh kill — 1.0 + this = max scale.
const COMBO_PUNCH_OVERSHOOT: f32 = 0.4;

/// Full-screen overlay used for the red hit-flash and the low-HP pulsing
/// vignette.  Both share the same node — alpha gets composed each frame
/// from `HitFlashTimer` (transient) and the static low-HP factor.
#[derive(Component)]
pub struct DamageOverlay;

/// Decaying flash timer in seconds that drives the red hit-overlay alpha
/// after a `PlayerDamagedEvent`.  Set in `update_damage_overlay`.
#[derive(Resource, Default)]
pub struct HitFlashTimer {
    pub remaining: f32,
}

const HIT_FLASH_DURATION: f32 = 0.3;

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
pub struct SegmentPromptRoot;

#[derive(Component)]
pub struct SegmentPromptText;

#[derive(Component)]
pub struct PlayerListRoot;

#[derive(Component)]
pub struct PlayerListSlot {
    pub slot: u8,
}

#[derive(Component)]
pub struct PlayerListNickText {
    pub slot: u8,
}

#[derive(Component)]
pub struct PlayerListHpFill {
    pub slot: u8,
}

#[derive(Component)]
pub struct SlotIcon(pub u8);

#[derive(Component)]
pub struct SlotBorder(pub u8);

/// Small per-slot text overlay showing ammo (slots 0/1) or count
/// (slot 2, throwables).  Lets the player see all slot states at a
/// glance instead of having to switch weapons to check.
#[derive(Component)]
pub struct SlotAmmoText(pub u8);

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HitFlashTimer>()
            .init_resource::<WaveIntroState>()
            .init_resource::<ComboState>()
            .init_resource::<BossAlertTimer>()
            .add_systems(OnEnter(GameState::Playing), (spawn_hud, reset_combo))
            .add_systems(
                OnExit(GameState::Playing),
                (despawn_hud, reset_wave_intro, reset_combo),
            )
            .add_systems(
                Update,
                (
                    update_hud,
                    update_slot_icons,
                    update_slot_ammo_text,
                    update_floor_indicator,
                    update_pickup_prompt,
                    update_wave_intro,
                    update_combo,
                    update_boss_alert,
                    update_damage_overlay,
                    update_fps_counter,
                    update_segment_prompt,
                    update_player_list,
                )
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
                    bottom: Val::Px(24.0),
                    left: Val::Px(24.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
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
                        column_gap: Val::Px(5.0),
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
                                    width: Val::Px(51.0),
                                    height: Val::Px(51.0),
                                    border: UiRect::all(Val::Px(2.0)),
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
                                        width: Val::Px(36.0),
                                        height: Val::Px(36.0),
                                        ..default()
                                    },
                                    image: UiImage::new(img),
                                    visibility: vis,
                                    ..default()
                                },
                                SlotIcon(i),
                            ));
                            // Tiny ammo / count overlay anchored bottom-right
                            // of the slot.  `update_slot_ammo_text` keeps it
                            // in sync with `Player.ammo[i]` / throwable_count.
                            slot.spawn((
                                TextBundle::from_section(
                                    "",
                                    TextStyle {
                                        font: font.clone(),
                                        font_size: 11.0,
                                        color: Color::srgba(1.0, 0.95, 0.6, 0.95),
                                    },
                                )
                                .with_style(Style {
                                    position_type: PositionType::Absolute,
                                    bottom: Val::Px(0.0),
                                    right: Val::Px(2.0),
                                    ..default()
                                }),
                                SlotAmmoText(i),
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
                        font_size: 15.0,
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
                        font_size: 14.0,
                        color: Color::srgba(0.65, 0.65, 0.6, 0.8),
                    },
                ),
                AmmoText,
            ));
            // Armor bar row — sits above HP, light/whitish so it reads as
            // "shield over flesh".  Width starts at 0 (no armor) and fills
            // proportionally to PLAYER_ARMOR_MAX.
            parent
                .spawn(NodeBundle {
                    style: Style {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(9.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(NodeBundle {
                        style: Style {
                            width: Val::Px(210.0),
                            height: Val::Px(6.0),
                            ..default()
                        },
                        background_color: BackgroundColor(Color::srgba(
                            0.30, 0.32, 0.34, 0.45,
                        )),
                        ..default()
                    })
                    .with_children(|bar| {
                        bar.spawn((
                            NodeBundle {
                                style: Style {
                                    width: Val::Percent(0.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                background_color: BackgroundColor(Color::srgba(
                                    0.95, 0.96, 0.99, 0.95,
                                )),
                                ..default()
                            },
                            ArmorBarFill,
                        ));
                    });
                    row.spawn((
                        TextBundle::from_section(
                            "0",
                            TextStyle {
                                font: font.clone(),
                                font_size: 12.0,
                                color: Color::srgba(0.92, 0.93, 0.96, 0.85),
                            },
                        ),
                        ArmorText,
                    ));
                });
            // HP bar row
            parent
                .spawn(NodeBundle {
                    style: Style {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(9.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(NodeBundle {
                        style: Style {
                            width: Val::Px(210.0),
                            height: Val::Px(8.0),
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
                                font_size: 14.0,
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
                    top: Val::Px(15.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(3.0),
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
                        font_size: 24.0,
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
                        font_size: 12.0,
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
                    top: Val::Px(15.0),
                    right: Val::Px(24.0),
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
                        font_size: 20.0,
                        color: Color::srgba(0.55, 0.95, 0.4, 0.9),
                    },
                ),
                ScoreValueText,
            ));
        });

    // ── Segment unlock prompt (bottom-center, hidden until in range) ──
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(225.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                },
                visibility: Visibility::Hidden,
                ..default()
            },
            HudRoot,
            SegmentPromptRoot,
        ))
        .with_children(|panel| {
            panel.spawn(NodeBundle {
                style: Style {
                    padding: UiRect::axes(Val::Px(27.0), Val::Px(12.0)),
                    border: UiRect::all(Val::Px(3.0)),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.06, 0.07, 0.10, 0.86)),
                border_color: BorderColor(Color::srgb(0.55, 0.46, 0.18)),
                ..default()
            })
            .with_children(|node| {
                node.spawn((
                    TextBundle::from_section(
                        "",
                        TextStyle {
                            font: font.clone(),
                            font_size: 21.0,
                            color: Color::srgba(0.92, 0.82, 0.32, 1.0),
                        },
                    ),
                    SegmentPromptText,
                ));
            });
        });

    // ── Top-left: multiplayer player list (visible only in Host/Client) ──
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(15.0),
                    left: Val::Px(24.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    width: Val::Px(270.0),
                    ..default()
                },
                visibility: Visibility::Hidden,
                ..default()
            },
            HudRoot,
            PlayerListRoot,
        ))
        .with_children(|panel| {
            for slot in 0u8..4 {
                panel
                    .spawn((
                        NodeBundle {
                            style: Style {
                                width: Val::Percent(100.0),
                                height: Val::Px(33.0),
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(9.0),
                                padding: UiRect::axes(Val::Px(9.0), Val::Px(3.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            background_color: BackgroundColor(Color::srgba(
                                0.05, 0.06, 0.08, 0.78,
                            )),
                            border_color: BorderColor(Color::srgba(0.18, 0.22, 0.25, 0.85)),
                            visibility: Visibility::Hidden,
                            ..default()
                        },
                        PlayerListSlot { slot },
                    ))
                    .with_children(|row| {
                        // Color stripe
                        row.spawn(NodeBundle {
                            style: Style {
                                width: Val::Px(12.0),
                                height: Val::Px(21.0),
                                ..default()
                            },
                            background_color: BackgroundColor(player_palette_color(slot)),
                            ..default()
                        });
                        // Nickname text (fixed width so HP bar aligns)
                        row.spawn(NodeBundle {
                            style: Style {
                                width: Val::Px(120.0),
                                overflow: Overflow::clip_x(),
                                ..default()
                            },
                            ..default()
                        })
                        .with_children(|cell| {
                            cell.spawn((
                                TextBundle::from_section(
                                    "",
                                    TextStyle {
                                        font: font.clone(),
                                        font_size: 17.0,
                                        color: Color::srgba(0.85, 0.85, 0.88, 0.95),
                                    },
                                ),
                                PlayerListNickText { slot },
                            ));
                        });
                        // HP bar (track + fill)
                        row.spawn(NodeBundle {
                            style: Style {
                                width: Val::Px(90.0),
                                height: Val::Px(12.0),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            background_color: BackgroundColor(Color::srgba(
                                0.04, 0.05, 0.06, 0.95,
                            )),
                            border_color: BorderColor(Color::srgba(0.22, 0.24, 0.26, 0.9)),
                            ..default()
                        })
                        .with_children(|track| {
                            track.spawn((
                                NodeBundle {
                                    style: Style {
                                        width: Val::Percent(100.0),
                                        height: Val::Percent(100.0),
                                        ..default()
                                    },
                                    background_color: BackgroundColor(Color::srgba(
                                        0.55, 0.85, 0.4, 0.9,
                                    )),
                                    ..default()
                                },
                                PlayerListHpFill { slot },
                            ));
                        });
                    });
            }
        });

    // Damage overlay (full-screen red layer) — alpha is driven each frame
    // by `update_damage_overlay`.  Composed from the hit-flash timer plus a
    // pulsing low-HP factor.
    commands.spawn((
        NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                ..default()
            },
            background_color: BackgroundColor(Color::srgba(0.85, 0.05, 0.06, 0.0)),
            z_index: ZIndex::Global(50),
            ..default()
        },
        HudRoot,
        DamageOverlay,
    ));

    // Floor indicator (bottom-center) — shows when player is inside a
    // multi-floor building.  Hidden otherwise.
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(60.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                },
                visibility: Visibility::Hidden,
                ..default()
            },
            HudRoot,
            FloorIndicatorRoot,
        ))
        .with_children(|node| {
            node.spawn(NodeBundle {
                style: Style {
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.78)),
                border_color: BorderColor(Color::srgba(0.95, 0.85, 0.4, 0.85)),
                ..default()
            })
            .with_children(|panel| {
                panel.spawn((
                    TextBundle::from_section(
                        "PIETRO 0/0",
                        TextStyle {
                            font: font.clone(),
                            font_size: 14.0,
                            color: Color::srgba(0.98, 0.92, 0.7, 0.95),
                        },
                    ),
                    FloorIndicatorText,
                ));
            });
        });

    // Kill-streak combo counter — top-right under the wave HUD.  Hidden
    // until the player chains kills.
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(80.0),
                    right: Val::Px(28.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexEnd,
                    ..default()
                },
                z_index: ZIndex::Global(38),
                visibility: Visibility::Hidden,
                ..default()
            },
            HudRoot,
            ComboCounterRoot,
        ))
        .with_children(|node| {
            node.spawn((
                TextBundle::from_section(
                    "x0 KILL",
                    TextStyle {
                        font: font.clone(),
                        font_size: 22.0,
                        color: Color::srgba(1.0, 0.95, 0.55, 1.0),
                    },
                ),
                ComboCounterText,
            ));
        });

    // Boss alert — red flashing border + "GIANT INCOMING" centre text,
    // fires when a Giant spawns.  Hidden by default.
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
                border_color: BorderColor(Color::srgba(0.95, 0.10, 0.12, 0.0)),
                z_index: ZIndex::Global(45),
                visibility: Visibility::Hidden,
                ..default()
            },
            HudRoot,
            BossAlertRoot,
        ))
        .with_children(|node| {
            // Inner backing panel so the text reads against any background.
            node.spawn(NodeBundle {
                style: Style {
                    padding: UiRect::axes(Val::Px(28.0), Val::Px(14.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    margin: UiRect::top(Val::Px(80.0)),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.10, 0.04, 0.05, 0.85)),
                border_color: BorderColor(Color::srgba(0.95, 0.10, 0.12, 0.95)),
                ..default()
            })
            .with_children(|panel| {
                panel.spawn((
                    TextBundle::from_section(
                        "GIANT INCOMING",
                        TextStyle {
                            font: font.clone(),
                            font_size: 30.0,
                            color: Color::srgba(1.0, 0.18, 0.18, 1.0),
                        },
                    ),
                    BossAlertText,
                ));
            });
        });

    // Wave intro splash — big text centred on screen, fades in/out when a
    // new wave starts.  Hidden by default; `update_wave_intro` flips it.
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                z_index: ZIndex::Global(40),
                visibility: Visibility::Hidden,
                ..default()
            },
            HudRoot,
            WaveIntroRoot,
        ))
        .with_children(|node| {
            node.spawn((
                TextBundle::from_section(
                    "WAVE 1",
                    TextStyle {
                        font: font.clone(),
                        font_size: 64.0,
                        color: Color::srgba(1.0, 0.92, 0.45, 1.0),
                    },
                ),
                WaveIntroText,
            ));
        });

    // Pickup prompt (centred, slightly above the floor indicator).  Stays
    // hidden until the player walks onto a weapon pickup with slot 2 full.
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(110.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                },
                visibility: Visibility::Hidden,
                ..default()
            },
            HudRoot,
            PickupPromptRoot,
        ))
        .with_children(|node| {
            node.spawn(NodeBundle {
                style: Style {
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.05, 0.07, 0.10, 0.82)),
                border_color: BorderColor(Color::srgba(0.95, 0.85, 0.4, 0.92)),
                ..default()
            })
            .with_children(|panel| {
                panel.spawn((
                    TextBundle::from_section(
                        "[E] PRESS TO PICK UP",
                        TextStyle {
                            font: font.clone(),
                            font_size: 16.0,
                            color: Color::srgba(0.98, 0.92, 0.7, 0.95),
                        },
                    ),
                    PickupPromptText,
                ));
            });
        });

    // FPS counter (bottom-right, hidden by default)
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(21.0),
                    right: Val::Px(21.0),
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
                        font_size: 14.0,
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

/// Cached scalar HUD state — the formatter is the expensive bit
/// (`format!` builds a fresh `String` every time), so we only re-format
/// when the underlying value actually changed.  Each field is `Option`
/// so the first frame of a play session always re-renders.
#[derive(Default)]
struct HudCache {
    hp: Option<i32>,
    armor: Option<i32>,
    score: Option<u32>,
    wave: Option<u32>,
    in_break: Option<bool>,
    /// `(active_slot, weapon_id_or_throwable)` so a slot switch invalidates.
    weapon: Option<(u8, u8)>,
    ammo: Option<(u32, u32, u32, bool)>, // (ammo, mag, reserve, reloading)
    /// Tenths-of-seconds for the break countdown — keeps formatting at ~10 Hz
    /// rather than per-frame even though the float value drifts every tick.
    break_tenths: Option<u32>,
    zombies_left: Option<usize>,
    throwable_count: Option<u32>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_hud(
    ctx: Res<NetContext>,
    players: Query<&Player>,
    score: Res<Score>,
    wave: Res<WaveState>,
    zombies: Query<(), With<Zombie>>,
    mut cache: Local<HudCache>,
    mut bars: ParamSet<(
        Query<&mut Style, With<HpBarFill>>,
        Query<&mut Style, With<ArmorBarFill>>,
    )>,
    mut hp_text: Query<
        &mut Text,
        (
            With<HpText>,
            Without<WaveTitleText>,
            Without<WaveStatusText>,
            Without<ScoreValueText>,
            Without<WeaponText>,
            Without<AmmoText>,
            Without<ArmorText>,
        ),
    >,
    mut armor_text: Query<
        &mut Text,
        (
            With<ArmorText>,
            Without<HpText>,
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
            Without<ArmorText>,
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
            Without<ArmorText>,
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
            Without<ArmorText>,
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
            Without<ArmorText>,
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
            Without<ArmorText>,
        ),
    >,
) {
    let Some(player) = players.iter().find(|p| p.id == ctx.my_id) else {
        return;
    };
    let hp_pct = (player.hp.max(0) as f32 / PLAYER_MAX_HP as f32 * 100.0).clamp(0.0, 100.0);
    let armor_pct = (player.armor.max(0) as f32 / PLAYER_ARMOR_MAX as f32 * 100.0)
        .clamp(0.0, 100.0);
    // Compare before writing — an unchanged bar must not dirty `Style`,
    // since any `Mut` deref marks it changed and re-runs UI layout.
    let hp_width = Val::Percent(hp_pct);
    if let Ok(mut style) = bars.p0().get_single_mut() {
        if style.width != hp_width {
            style.width = hp_width;
        }
    }
    let armor_width = Val::Percent(armor_pct);
    if let Ok(mut style) = bars.p1().get_single_mut() {
        if style.width != armor_width {
            style.width = armor_width;
        }
    }
    let hp_now = player.hp.max(0);
    if cache.hp != Some(hp_now) {
        cache.hp = Some(hp_now);
        if let Ok(mut text) = hp_text.get_single_mut() {
            text.sections[0].value = format!("{}", hp_now);
        }
    }
    let armor_now = player.armor.max(0);
    if cache.armor != Some(armor_now) {
        cache.armor = Some(armor_now);
        if let Ok(mut text) = armor_text.get_single_mut() {
            text.sections[0].value = format!("{}", armor_now);
        }
    }
    // Weapon / slot display
    let weapon_key = if player.active_slot <= 1 {
        (player.active_slot, player.active_weapon().as_u8())
    } else {
        // Distinct namespace for throwables so we don't clash with weapon ids.
        (2, 0xF0 | player.throwable_kind as u8)
    };
    if cache.weapon != Some(weapon_key) {
        cache.weapon = Some(weapon_key);
        if let Ok(mut text) = weapon_text.get_single_mut() {
            let slot = player.active_slot;
            if slot <= 1 {
                text.sections[0].value =
                    format!("[{}] {}", slot + 1, player.active_weapon().label());
            } else {
                text.sections[0].value = format!("[3] {}", player.throwable_kind.label());
            }
        }
    }
    // Ammo display
    let ammo_now = if player.active_slot <= 1 {
        let slot = player.active_slot as usize;
        let weapon = player.active_weapon();
        (
            player.ammo[slot],
            weapon.magazine_size(),
            player.reserve_ammo[slot],
            player.reload_timer > 0.0,
        )
    } else {
        // Throwable slot — encode count in the reserve field, force "non-reloading".
        (player.throwable_count, 0, 0, false)
    };
    let throwable_now = if player.active_slot > 1 {
        Some(player.throwable_count)
    } else {
        None
    };
    if cache.ammo != Some(ammo_now) || cache.throwable_count != throwable_now {
        cache.ammo = Some(ammo_now);
        cache.throwable_count = throwable_now;
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
    }
    if cache.wave != Some(wave.current_wave) || cache.in_break != Some(wave.in_break) {
        cache.wave = Some(wave.current_wave);
        cache.in_break = Some(wave.in_break);
        if let Ok(mut text) = wave_title.get_single_mut() {
            text.sections[0].value = if wave.in_break && wave.current_wave == 0 {
                "GET READY".to_string()
            } else if wave.in_break {
                format!("WAVE {} IN...", wave.current_wave + 1)
            } else {
                format!("WAVE {}", wave.current_wave)
            };
        }
    }
    if wave.in_break {
        // Throttle countdown formatting to ~10 Hz by quantising to tenths.
        let tenths = (wave.break_timer.remaining_secs() * 10.0).max(0.0) as u32;
        if cache.break_tenths != Some(tenths) {
            cache.break_tenths = Some(tenths);
            cache.zombies_left = None;
            if let Ok(mut text) = wave_status.get_single_mut() {
                text.sections[0].value =
                    format!("{:.1}s", wave.break_timer.remaining_secs());
            }
        }
    } else {
        let alive = zombies.iter().count();
        let left = wave.zombies_to_spawn as usize + alive;
        if cache.zombies_left != Some(left) {
            cache.zombies_left = Some(left);
            cache.break_tenths = None;
            if let Ok(mut text) = wave_status.get_single_mut() {
                text.sections[0].value = format!("ZOMBIES: {left}");
            }
        }
    }
    if cache.score != Some(score.0) {
        cache.score = Some(score.0);
        if let Ok(mut text) = score_text.get_single_mut() {
            text.sections[0].value = format!("${}", format_compact_score(score.0));
        }
    }
}

/// Compact score so a long match doesn't blow past the HUD width.
/// `< 10K` → raw number, `< 1M` → `12.3K`, otherwise `9.9M`.
fn format_compact_score(score: u32) -> String {
    if score < 10_000 {
        format!("{score}")
    } else if score < 1_000_000 {
        format!("{:.1}K", score as f32 / 1_000.0)
    } else {
        format!("{:.1}M", score as f32 / 1_000_000.0)
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

fn update_slot_ammo_text(
    ctx: Res<NetContext>,
    players: Query<&Player>,
    mut texts: Query<(&SlotAmmoText, &mut Text)>,
) {
    let Some(player) = players.iter().find(|p| p.id == ctx.my_id) else {
        return;
    };
    for (slot, mut text) in &mut texts {
        let value: String = match slot.0 {
            0 | 1 => {
                let idx = slot.0 as usize;
                match player.slots[idx] {
                    Some(w) if w.has_infinite_ammo() => "∞".to_string(),
                    Some(_) => format!("{}", player.ammo[idx]),
                    None => String::new(),
                }
            }
            2 => {
                if player.throwable_count > 0 {
                    format!("x{}", player.throwable_count)
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        };
        if text.sections[0].value != value {
            text.sections[0].value = value;
        }
    }
}

fn reset_wave_intro(mut state: ResMut<WaveIntroState>) {
    *state = WaveIntroState::default();
}

fn reset_combo(mut state: ResMut<ComboState>) {
    *state = ComboState::default();
}

/// Fires the boss-alert UI flash when a Giant spawns.  The border + text
/// pulse for a few seconds then auto-hide.
#[allow(clippy::type_complexity)]
fn update_boss_alert(
    time: Res<Time>,
    mut events: EventReader<SpawnZombieEvent>,
    mut alert: ResMut<BossAlertTimer>,
    mut root: Query<
        (&mut Visibility, &mut BorderColor),
        (With<BossAlertRoot>, Without<BossAlertText>),
    >,
    mut text_q: Query<&mut Text, With<BossAlertText>>,
) {
    for ev in events.read() {
        if matches!(ev.kind, ZombieKind::Giant) {
            alert.remaining = BOSS_ALERT_DURATION;
        }
    }
    alert.remaining = (alert.remaining - time.delta_seconds()).max(0.0);
    let active = alert.remaining > 0.0;
    if let Ok((mut vis, mut border)) = root.get_single_mut() {
        *vis = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if active {
            // Fast pulse for that classic alarm feel.
            let pulse = (time.elapsed_seconds() * 7.0).sin() * 0.5 + 0.5;
            border.0 = Color::srgba(1.0, 0.12, 0.18, 0.5 + pulse * 0.4);
        }
    }
    if active {
        if let Ok(mut text) = text_q.get_single_mut() {
            // Punch the text alpha at the same pulse so the warning reads
            // even if the player is looking away from the centre.
            let pulse = (time.elapsed_seconds() * 7.0).sin() * 0.5 + 0.5;
            for sec in &mut text.sections {
                sec.style.color.set_alpha(0.65 + pulse * 0.35);
            }
        }
    }
}

/// Tracks consecutive kills and pumps the on-screen combo counter.  Tiered
/// label + colour so longer chains read as escalating: "KILL" → "STREAK" →
/// "RAMPAGE" → "UNSTOPPABLE".  Punch animation scales the text briefly
/// every fresh kill.
#[allow(clippy::type_complexity)]
fn update_combo(
    time: Res<Time>,
    mut state: ResMut<ComboState>,
    mut events: EventReader<ZombieKilledEvent>,
    mut root: Query<&mut Visibility, (With<ComboCounterRoot>, Without<ComboCounterText>)>,
    mut text_q: Query<(&mut Text, &mut Transform), With<ComboCounterText>>,
    // Last `count` we built the label string for; lets us skip the per-frame
    // `format!` + clone and only rewrite the text value when the streak ticks up.
    mut last_count: Local<u32>,
) {
    let dt = time.delta_seconds();
    state.time_since_kill += dt;
    state.punch_time += dt;

    for _ in events.read() {
        if state.time_since_kill > COMBO_TIMEOUT {
            state.count = 1;
        } else {
            state.count += 1;
        }
        state.time_since_kill = 0.0;
        // Reset punch_time so the bump animation re-plays on every kill —
        // `state.punch_time` is read further down as the animation phase.
        state.punch_time = 0.0;
    }

    if state.time_since_kill > COMBO_TIMEOUT {
        state.count = 0;
    }

    let active = state.count >= 2;
    if let Ok(mut vis) = root.get_single_mut() {
        *vis = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }

    if !active {
        // Force a label rebuild the next time a streak starts.
        *last_count = 0;
        return;
    }

    // Tiered colour — a cheap alloc-free match, recomputed each frame so the
    // fade alpha below stays live.
    let color = match state.count {
        2..=4 => Color::srgba(1.0, 0.96, 0.62, 1.0),
        5..=9 => Color::srgba(1.0, 0.78, 0.32, 1.0),
        10..=19 => Color::srgba(1.0, 0.45, 0.20, 1.0),
        _ => Color::srgba(1.0, 0.18, 0.18, 1.0),
    };

    // The text value only changes when the streak count does, so build the
    // string (the one heap alloc here) only then — not every frame.
    let new_label = if state.count != *last_count {
        Some(match state.count {
            2..=4 => format!("x{} KILL", state.count),
            5..=9 => format!("x{} STREAK", state.count),
            10..=19 => format!("x{} RAMPAGE", state.count),
            _ => format!("x{} UNSTOPPABLE", state.count),
        })
    } else {
        None
    };
    *last_count = state.count;

    // Fade out as the streak window runs out.
    let fade = ((COMBO_TIMEOUT - state.time_since_kill) / COMBO_TIMEOUT).clamp(0.0, 1.0);
    // Scale punch — quick overshoot then settle to 1.0.
    let punch_phase = (state.punch_time / COMBO_PUNCH_DURATION).clamp(0.0, 1.0);
    let scale = 1.0 + (1.0 - punch_phase) * COMBO_PUNCH_OVERSHOOT;

    if let Ok((mut text, mut transform)) = text_q.get_single_mut() {
        for sec in &mut text.sections {
            if let Some(ref label) = new_label {
                sec.value = label.clone();
            }
            let mut c = color;
            c.set_alpha(fade);
            sec.style.color = c;
        }
        transform.scale = Vec3::new(scale, scale, 1.0);
    }
}

/// Big "WAVE N" splash centred on screen, fired once per new wave start.
/// Triggers on `wave.in_break == false` AND `wave.current_wave > last`.
#[allow(clippy::type_complexity)]
fn update_wave_intro(
    time: Res<Time>,
    wave: Res<WaveState>,
    mut state: ResMut<WaveIntroState>,
    mut root: Query<&mut Visibility, (With<WaveIntroRoot>, Without<WaveIntroText>)>,
    mut text_q: Query<&mut Text, With<WaveIntroText>>,
) {
    if !wave.in_break && wave.current_wave > state.last_wave {
        state.last_wave = wave.current_wave;
        state.flash_remaining = WAVE_INTRO_DURATION;
        if let Ok(mut text) = text_q.get_single_mut() {
            for sec in &mut text.sections {
                sec.value = format!("WAVE {}", wave.current_wave);
            }
        }
    }
    state.flash_remaining = (state.flash_remaining - time.delta_seconds()).max(0.0);
    let active = state.flash_remaining > 0.0;
    if let Ok(mut vis) = root.get_single_mut() {
        *vis = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if active {
        let pct = (state.flash_remaining / WAVE_INTRO_DURATION).clamp(0.0, 1.0);
        // Smooth fade-in for first 30% of life then steady, fade-out at end.
        let alpha = if pct > 0.7 {
            ((1.0 - pct) / 0.3).clamp(0.0, 1.0)
        } else if pct < 0.3 {
            (pct / 0.3).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if let Ok(mut text) = text_q.get_single_mut() {
            for sec in &mut text.sections {
                sec.style.color.set_alpha(alpha);
            }
        }
    }
}

/// Drives the full-screen red overlay alpha every frame.  Composed from:
///   - a hit-flash that spikes on `PlayerDamagedEvent` and decays
///   - a pulsing low-HP layer that grows as HP drops below 30%.
/// Both layers max-blend so neither cancels the other.
fn update_damage_overlay(
    time: Res<Time>,
    ctx: Res<NetContext>,
    players: Query<&Player>,
    mut events: EventReader<PlayerDamagedEvent>,
    mut flash_timer: ResMut<HitFlashTimer>,
    mut overlays: Query<&mut BackgroundColor, With<DamageOverlay>>,
) {
    // Bump the flash on every relevant damage event for the local player.
    for ev in events.read() {
        if ev.target_id == ctx.my_id {
            flash_timer.remaining = HIT_FLASH_DURATION;
        }
    }
    flash_timer.remaining = (flash_timer.remaining - time.delta_seconds()).max(0.0);
    // Subtle hit flash — peaks at 0.28 alpha then fades quickly so the
    // player feels the hit without the screen turning into a red wall.
    let flash_alpha = (flash_timer.remaining / HIT_FLASH_DURATION).clamp(0.0, 1.0) * 0.28;

    // Low-HP pulse — kicks in below 30% HP, gentle pulse so the warning
    // is felt rather than blasted at the player.
    let local_hp = players
        .iter()
        .find(|p| p.id == ctx.my_id)
        .map(|p| p.hp)
        .unwrap_or(PLAYER_MAX_HP);
    let hp_pct = (local_hp.max(0) as f32 / PLAYER_MAX_HP as f32).clamp(0.0, 1.0);
    let low_hp_factor = ((0.30 - hp_pct) / 0.30).max(0.0);
    let pulse = (time.elapsed_seconds() * 3.0).sin() * 0.5 + 0.5;
    let low_hp_alpha = low_hp_factor * (0.06 + pulse * 0.10);

    let alpha = flash_alpha.max(low_hp_alpha).min(0.34);
    if let Ok(mut bg) = overlays.get_single_mut() {
        bg.0 = Color::srgba(0.92, 0.10, 0.10, alpha);
    }
}

/// Shows the "[E] PICK UP" prompt whenever the local player is overlapping
/// a weapon pickup that would replace a held weapon.  Hidden when slot 2
/// is empty (auto-pickup case) or when there's no nearby pickup.
#[allow(clippy::type_complexity)]
fn update_pickup_prompt(
    hint: Res<PickupPromptHint>,
    mut last: Local<Option<Weapon>>,
    mut root: Query<&mut Visibility, (With<PickupPromptRoot>, Without<PickupPromptText>)>,
    mut text_q: Query<&mut Text, With<PickupPromptText>>,
) {
    let Ok(mut vis) = root.get_single_mut() else {
        return;
    };
    match hint.weapon {
        Some(w) => {
            *vis = Visibility::Inherited;
            // Only re-format when the hinted weapon changed — the string is
            // identical frame after frame while the prompt is up, and
            // writing `Text` re-runs glyph layout even for an equal value.
            if *last != Some(w) {
                if let Ok(mut text) = text_q.get_single_mut() {
                    text.sections[0].value = format!("[E] PICK UP - {}", w.label());
                }
            }
        }
        None => {
            *vis = Visibility::Hidden;
        }
    }
    *last = hint.weapon;
}

/// Toggles the floor indicator visibility based on whether the local
/// player is inside a multi-floor building, and shows the current
/// `floor + 1 / total` count plus the localised hint.
#[allow(clippy::type_complexity)]
fn update_floor_indicator(
    floor_state: Res<PlayerFloorState>,
    mut last: Local<Option<(usize, u8)>>,
    mut root: Query<&mut Visibility, (With<FloorIndicatorRoot>, Without<FloorIndicatorText>)>,
    mut text_q: Query<&mut Text, With<FloorIndicatorText>>,
) {
    let Ok(mut vis) = root.get_single_mut() else {
        return;
    };
    match floor_state.building {
        Some(b_idx) => {
            *vis = Visibility::Inherited;
            // Only re-format when (building, floor) changed — the label is
            // stable while the player stays on the same storey.
            let key = Some((b_idx, floor_state.floor));
            if *last != key {
                let kind = BUILDINGS[b_idx].kind;
                let total = building_floor_count(kind);
                // Translate the numeric floor index into a human label so the
                // ground floor reads "PARTER" (typical Polish convention).
                let label = match floor_state.floor {
                    0 => "PARTER".to_string(),
                    f if f as u8 + 1 == total => "DACH".to_string(),
                    f => format!("{} PIETRO", f),
                };
                if let Ok(mut text) = text_q.get_single_mut() {
                    text.sections[0].value = format!(
                        "{}  ({}/{})  - E aby zmienic",
                        label,
                        floor_state.floor + 1,
                        total,
                    );
                }
            }
            *last = key;
        }
        None => {
            *vis = Visibility::Hidden;
            *last = None;
        }
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

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_player_list(
    net: Res<NetMode>,
    nicknames: Res<PlayerNicknames>,
    players: Query<&Player>,
    mut ids: Local<Vec<u8>>,
    mut root: Query<
        &mut Visibility,
        (
            With<PlayerListRoot>,
            Without<PlayerListSlot>,
            Without<PlayerListNickText>,
            Without<PlayerListHpFill>,
        ),
    >,
    mut slots: Query<
        (&PlayerListSlot, &mut Visibility),
        (
            Without<PlayerListRoot>,
            Without<PlayerListNickText>,
            Without<PlayerListHpFill>,
        ),
    >,
    mut nick_texts: Query<(&PlayerListNickText, &mut Text)>,
    mut hp_fills: Query<(&PlayerListHpFill, &mut Style)>,
) {
    let multiplayer = matches!(*net, NetMode::Host | NetMode::Client);
    // All writes below are compare-gated: a `Mut` deref marks the component
    // changed even for an equal value, which would keep text glyph layout
    // and UI layout hot every frame for content that almost never changes.
    if let Ok(mut vis) = root.get_single_mut() {
        let want = if multiplayer {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
    if !multiplayer {
        return;
    }

    // Build a quick lookup: slot index → matching player id, looking by sort
    // order of player ids so layout is stable across frames.  The buffer is
    // a `Local` so the per-frame rebuild doesn't allocate.
    ids.clear();
    ids.extend(players.iter().map(|p| p.id));
    ids.sort_unstable();

    for (slot_marker, mut vis) in slots.iter_mut() {
        let want = if (slot_marker.slot as usize) < ids.len() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
    // Update text + HP fill per slot.
    for (n, mut text) in nick_texts.iter_mut() {
        if let Some(&id) = ids.get(n.slot as usize) {
            // Compare against the current value first so the clone/format
            // only happens when the nickname actually changed.
            if let Some(nick) = nicknames.0.get(&id) {
                if text.sections[0].value != *nick {
                    text.sections[0].value = nick.clone();
                }
            } else {
                let fallback = format!("P{}", id);
                if text.sections[0].value != fallback {
                    text.sections[0].value = fallback;
                }
            }
        } else if !text.sections[0].value.is_empty() {
            text.sections[0].value.clear();
        }
    }
    for (hp_fill, mut style) in hp_fills.iter_mut() {
        let want = if let Some(&id) = ids.get(hp_fill.slot as usize) {
            let hp = players
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.hp.max(0))
                .unwrap_or(0);
            let frac = (hp as f32 / PLAYER_MAX_HP as f32).clamp(0.0, 1.0);
            Val::Percent(frac * 100.0)
        } else {
            Val::Percent(0.0)
        };
        if style.width != want {
            style.width = want;
        }
    }
}

fn update_segment_prompt(
    hint: Res<SegmentUnlockHint>,
    mut last: Local<Option<(u8, u32, bool)>>,
    mut roots: Query<&mut Visibility, With<SegmentPromptRoot>>,
    mut texts: Query<&mut Text, With<SegmentPromptText>>,
) {
    let Ok(mut vis) = roots.get_single_mut() else {
        return;
    };
    let Ok(mut text) = texts.get_single_mut() else {
        return;
    };
    match hint.segment_idx {
        Some(idx) => {
            *vis = Visibility::Inherited;
            // Only re-format when the target/cost/affordability changed —
            // the string and color are stable frame to frame otherwise.
            let key = Some((idx, hint.cost, hint.affordable));
            if *last != key {
                let label = segment_name(idx);
                let section = &mut text.sections[0];
                if hint.affordable {
                    section.value = format!("[E]  ODBLOKUJ {label}  -  ${}", hint.cost);
                    section.style.color = Color::srgba(0.55, 0.95, 0.4, 1.0);
                } else {
                    section.value = format!("BRAK $$$  -  {label}  KOSZT  ${}", hint.cost);
                    section.style.color = Color::srgba(0.92, 0.45, 0.32, 1.0);
                }
            }
            *last = key;
        }
        None => {
            *vis = Visibility::Hidden;
            *last = None;
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

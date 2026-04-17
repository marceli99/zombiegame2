use bevy::prelude::*;

use crate::achievements::AchievementTracker;
use crate::audio::SfxEvent;
use crate::map::{
    unlock_nav_rows, MapObstacles, NavGrid, Obstacle, ObstacleShape, BARRIER_NORTH_Y,
    BARRIER_SOUTH_Y, BARRIER_UNDERGROUND_Y, MAP_HEIGHT, MAP_WIDTH, ZONE1_ROW_MAX, ZONE1_ROW_MIN,
    ZONE2_ROW_MAX, ZONE2_ROW_MIN, ZONE3_ROW_MAX, ZONE3_ROW_MIN,
};
use crate::net::NetContext;
use crate::pixelart::{Canvas, Rgba};
use crate::player::Player;
use crate::weapon::Weapon;
use crate::{gameplay_active, GameState, Score, UiAssets};
use rand::Rng;

const BARRIER_HALF_H: f32 = 6.0;
const BARRIER_INTERACT_RANGE: f32 = 80.0;
const SHOP_INTERACT_RANGE: f32 = 50.0;

const CLEARING_FADE_DURATION: f32 = 0.8;
const CLEARING_MAX_DELAY: f32 = 0.5;
const LEAF_BURST_PER_TREE: usize = 4;
const FOG_ALPHA: f32 = 0.50;
const FOG_FADE_DURATION: f32 = 1.5;

#[derive(Resource)]
pub struct ZoneState {
    pub unlocked: [bool; 4],
}

impl Default for ZoneState {
    fn default() -> Self {
        Self {
            unlocked: [true, false, false, false],
        }
    }
}

#[derive(Component)]
pub struct ZoneBarrier {
    pub zone_id: u8,
    pub cost: u32,
}

#[derive(Component)]
pub struct WeaponShop {
    pub weapon: Weapon,
    pub cost: u32,
}

#[derive(Component)]
struct InteractionPrompt;

#[derive(Component)]
struct PromptText;

#[derive(Component)]
struct BarrierDecor {
    zone_id: u8,
}

#[derive(Component)]
struct ZoneFog {
    zone_id: u8,
}

#[derive(Component)]
struct ZoneFogFading {
    elapsed: f32,
}

#[derive(Component)]
struct BarrierClearing {
    delay: f32,
    elapsed: f32,
    fade_duration: f32,
    spawned_particles: bool,
}

#[derive(Component)]
struct UnlockParticle {
    velocity: Vec2,
    lifetime: f32,
    max_lifetime: f32,
    base_color: [f32; 3],
}

#[derive(Resource)]
struct ZoneAssets {
    dense_tree: Handle<Image>,
    thick_bush: Handle<Image>,
    rubble: Handle<Image>,
    shop: Handle<Image>,
    leaf: Handle<Image>,
    dust: Handle<Image>,
    fog: Handle<Image>,
}

pub struct ZonesPlugin;

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZoneState>()
            .add_systems(Startup, setup_zone_assets)
            .add_systems(
                OnEnter(GameState::Playing),
                (
                    reset_zones,
                    (spawn_barriers, spawn_shops, spawn_interaction_prompt),
                )
                    .chain(),
            )
            .add_systems(
                OnExit(GameState::Playing),
                (despawn_barriers_and_shops, despawn_prompt),
            )
            .add_systems(
                Update,
                (
                    barrier_interaction.run_if(gameplay_active),
                    shop_interaction.run_if(gameplay_active),
                    update_interaction_prompt,
                    animate_barrier_clearing,
                    animate_unlock_particles,
                    animate_fog_fading,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_zone_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(ZoneAssets {
        dense_tree: images.add(build_dense_tree_image()),
        thick_bush: images.add(build_thick_bush_image()),
        rubble: images.add(build_rubble_image()),
        shop: images.add(build_shop_image()),
        leaf: images.add(build_leaf_image()),
        dust: images.add(build_dust_image()),
        fog: images.add(build_fog_image()),
    });
}

fn reset_zones(mut zone_state: ResMut<ZoneState>, mut nav: ResMut<NavGrid>) {
    *zone_state = ZoneState::default();
    *nav = NavGrid::default();
}

fn spawn_barriers(
    mut commands: Commands,
    assets: Res<ZoneAssets>,
    zone_state: Res<ZoneState>,
    mut obstacles: ResMut<MapObstacles>,
) {
    let barrier_defs: [(u8, f32, u32); 3] = [
        (1, BARRIER_NORTH_Y, 1500),
        (2, BARRIER_SOUTH_Y, 2000),
        (3, BARRIER_UNDERGROUND_Y, 2500),
    ];

    let mut rng = rand::thread_rng();

    for (zone_id, y, cost) in barrier_defs {
        if zone_state.unlocked[zone_id as usize] {
            continue;
        }
        let pos = Vec2::new(0.0, y);

        // Invisible barrier entity for collision & interaction
        commands.spawn((
            SpatialBundle::from_transform(Transform::from_xyz(pos.x, pos.y, 0.0)),
            ZoneBarrier { zone_id, cost },
        ));
        obstacles.list.push(Obstacle {
            pos,
            shape: ObstacleShape::Rect(Vec2::new(MAP_WIDTH / 2.0, BARRIER_HALF_H)),
        });

        // Natural decorations along the barrier line
        let hw = MAP_WIDTH / 2.0 - 20.0;
        if zone_id <= 2 {
            // Surface zones: dense treeline with undergrowth
            let tree_count = 24;
            let spacing = (hw * 2.0) / tree_count as f32;
            for i in 0..tree_count {
                let x = -hw + spacing * (i as f32 + 0.5) + rng.gen_range(-10.0..10.0);
                let ty = y + rng.gen_range(-14.0..14.0);
                let size = rng.gen_range(48.0..62.0);
                let rot = rng.gen_range(-0.12..0.12);
                commands.spawn((
                    SpriteBundle {
                        texture: assets.dense_tree.clone(),
                        sprite: Sprite {
                            custom_size: Some(Vec2::splat(size)),
                            ..default()
                        },
                        transform: Transform::from_xyz(x, ty, -1.0)
                            .with_rotation(Quat::from_rotation_z(rot)),
                        ..default()
                    },
                    BarrierDecor { zone_id },
                ));
            }
            // Fill gaps with bushes
            for _ in 0..18 {
                let x = rng.gen_range(-hw..hw);
                let by = y + rng.gen_range(-20.0..20.0);
                let size = rng.gen_range(18.0..30.0);
                commands.spawn((
                    SpriteBundle {
                        texture: assets.thick_bush.clone(),
                        sprite: Sprite {
                            custom_size: Some(Vec2::new(size * 1.15, size * 0.85)),
                            ..default()
                        },
                        transform: Transform::from_xyz(x, by, -0.9),
                        ..default()
                    },
                    BarrierDecor { zone_id },
                ));
            }
        } else {
            // Underground: rubble and debris
            let count = 20;
            let spacing = (hw * 2.0) / count as f32;
            for i in 0..count {
                let x = -hw + spacing * (i as f32 + 0.5) + rng.gen_range(-14.0..14.0);
                let ry = y + rng.gen_range(-12.0..12.0);
                let size_x = rng.gen_range(28.0..48.0);
                let size_y = rng.gen_range(20.0..36.0);
                let rot = rng.gen_range(-0.25..0.25);
                commands.spawn((
                    SpriteBundle {
                        texture: assets.rubble.clone(),
                        sprite: Sprite {
                            custom_size: Some(Vec2::new(size_x, size_y)),
                            ..default()
                        },
                        transform: Transform::from_xyz(x, ry, -0.8)
                            .with_rotation(Quat::from_rotation_z(rot)),
                        ..default()
                    },
                    BarrierDecor { zone_id },
                ));
            }
        }

        // Dark fog overlay covering the locked zone
        let half_h = MAP_HEIGHT / 2.0;
        let (fog_bot, fog_top) = match zone_id {
            1 => (BARRIER_NORTH_Y, half_h),
            2 => (BARRIER_UNDERGROUND_Y, BARRIER_SOUTH_Y),
            3 => (-half_h, BARRIER_UNDERGROUND_Y),
            _ => continue,
        };
        let fog_h = fog_top - fog_bot + 20.0;
        let fog_cy = (fog_bot + fog_top) / 2.0;
        commands.spawn((
            SpriteBundle {
                texture: assets.fog.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::new(MAP_WIDTH + 40.0, fog_h)),
                    color: Color::srgba(0.0, 0.02, 0.05, FOG_ALPHA),
                    ..default()
                },
                transform: Transform::from_xyz(0.0, fog_cy, 4.0),
                ..default()
            },
            ZoneFog { zone_id },
        ));
    }
}

fn spawn_shops(mut commands: Commands, assets: Res<ZoneAssets>) {
    let shop_defs: &[(Weapon, Vec2, u32)] = &[
        // Zone 0
        (Weapon::Smg, Vec2::new(-350.0, -250.0), 750),
        (Weapon::Shotgun, Vec2::new(450.0, 250.0), 1200),
        // Zone 1 (north)
        (Weapon::Rifle, Vec2::new(-400.0, 900.0), 2000),
        (Weapon::Minigun, Vec2::new(600.0, 850.0), 3500),
        // Zone 2 (south)
        (Weapon::Sniper, Vec2::new(-300.0, -750.0), 3000),
        (Weapon::RocketLauncher, Vec2::new(500.0, -680.0), 4000),
        // Zone 3 (underground)
        (Weapon::Flamethrower, Vec2::new(0.0, -1020.0), 2500),
    ];

    for &(weapon, pos, cost) in shop_defs {
        commands.spawn((
            SpriteBundle {
                texture: assets.shop.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::new(36.0, 48.0)),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, 3.0),
                ..default()
            },
            WeaponShop { weapon, cost },
        ));
    }
}

fn spawn_interaction_prompt(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(80.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                ..default()
            },
            InteractionPrompt,
        ))
        .with_children(|root| {
            root.spawn((
                TextBundle::from_section(
                    "",
                    TextStyle {
                        font: assets.font.clone(),
                        font_size: 14.0,
                        color: Color::srgb(1.0, 0.95, 0.7),
                    },
                ),
                PromptText,
            ));
        });
}

#[allow(clippy::too_many_arguments)]
fn barrier_interaction(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    ctx: Res<NetContext>,
    players: Query<(&Transform, &Player)>,
    barriers: Query<(Entity, &Transform, &ZoneBarrier)>,
    decor: Query<(Entity, &BarrierDecor), Without<BarrierClearing>>,
    fogs: Query<(Entity, &ZoneFog)>,
    mut score: ResMut<Score>,
    mut zone_state: ResMut<ZoneState>,
    mut nav: ResMut<NavGrid>,
    mut obstacles: ResMut<MapObstacles>,
    mut sfx: EventWriter<SfxEvent>,
) {
    if !keys.just_pressed(KeyCode::KeyE) {
        return;
    }
    let Some(player_pos) = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .map(|(t, _)| t.translation.truncate())
    else {
        return;
    };

    for (entity, t, barrier) in &barriers {
        let bp = t.translation.truncate();
        if (player_pos.y - bp.y).abs() < BARRIER_INTERACT_RANGE && score.0 >= barrier.cost {
            score.0 -= barrier.cost;
            let zone_id = barrier.zone_id;
            zone_state.unlocked[zone_id as usize] = true;

            obstacles.remove_at(bp);

            let (row_min, row_max) = match zone_id {
                1 => (ZONE1_ROW_MIN, ZONE1_ROW_MAX),
                2 => (ZONE2_ROW_MIN, ZONE2_ROW_MAX),
                3 => (ZONE3_ROW_MIN, ZONE3_ROW_MAX),
                _ => continue,
            };
            unlock_nav_rows(&mut nav, row_min, row_max);

            // Despawn invisible collision barrier
            commands.entity(entity).despawn_recursive();

            // Mark all decorations for this zone for clearing animation
            let mut rng = rand::thread_rng();
            for (decor_entity, dec) in &decor {
                if dec.zone_id == zone_id {
                    commands.entity(decor_entity).insert(BarrierClearing {
                        delay: rng.gen_range(0.0..CLEARING_MAX_DELAY),
                        elapsed: 0.0,
                        fade_duration: rng.gen_range(0.5..CLEARING_FADE_DURATION),
                        spawned_particles: false,
                    });
                }
            }

            // Start fading the fog overlay for this zone
            for (fog_entity, fog) in &fogs {
                if fog.zone_id == zone_id {
                    commands
                        .entity(fog_entity)
                        .insert(ZoneFogFading { elapsed: 0.0 });
                }
            }

            sfx.send(SfxEvent::Explosion);
            break;
        }
    }
}

fn shop_interaction(
    keys: Res<ButtonInput<KeyCode>>,
    ctx: Res<NetContext>,
    mut players: Query<(&Transform, &mut Player)>,
    shops: Query<(&Transform, &WeaponShop)>,
    mut score: ResMut<Score>,
    mut sfx: EventWriter<SfxEvent>,
    mut tracker: ResMut<AchievementTracker>,
) {
    if !keys.just_pressed(KeyCode::KeyE) {
        return;
    }
    let Some((pt, mut player)) = players.iter_mut().find(|(_, p)| p.id == ctx.my_id) else {
        return;
    };
    let player_pos = pt.translation.truncate();

    for (t, shop) in &shops {
        let sp = t.translation.truncate();
        if player_pos.distance(sp) < SHOP_INTERACT_RANGE && score.0 >= shop.cost {
            let slot = if player.active_slot <= 1 {
                player.active_slot as usize
            } else {
                0
            };
            if player.slots[slot] == Some(shop.weapon) {
                continue;
            }
            score.0 -= shop.cost;
            player.slots[slot] = Some(shop.weapon);
            player.ammo[slot] = shop.weapon.magazine_size();
            player.reserve_ammo[slot] = shop.weapon.reserve_ammo();
            player.reload_timer = 0.0;
            player.active_slot = slot as u8;
            tracker.weapons_bought |= 1 << shop.weapon.as_u8();
            sfx.send(SfxEvent::MenuSelect);
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_interaction_prompt(
    ctx: Res<NetContext>,
    players: Query<(&Transform, &Player)>,
    barriers: Query<(&Transform, &ZoneBarrier)>,
    shops: Query<(&Transform, &WeaponShop)>,
    score: Res<Score>,
    mut prompt_vis: Query<&mut Visibility, With<InteractionPrompt>>,
    mut prompt_text: Query<&mut Text, With<PromptText>>,
) {
    let Some(player_pos) = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .map(|(t, _)| t.translation.truncate())
    else {
        return;
    };

    let Ok(mut vis) = prompt_vis.get_single_mut() else {
        return;
    };
    let Ok(mut text) = prompt_text.get_single_mut() else {
        return;
    };

    // Check barriers
    for (t, barrier) in &barriers {
        let bp = t.translation.truncate();
        if (player_pos.y - bp.y).abs() < BARRIER_INTERACT_RANGE {
            let can_afford = score.0 >= barrier.cost;
            let zone_name = match barrier.zone_id {
                1 => "NORTH",
                2 => "SOUTH",
                3 => "UNDERGROUND",
                _ => "???",
            };
            text.sections[0].value = if can_afford {
                format!("[E] UNLOCK {} (${})", zone_name, barrier.cost)
            } else {
                format!("UNLOCK {} - NEED ${}", zone_name, barrier.cost)
            };
            text.sections[0].style.color = if can_afford {
                Color::srgb(0.2, 1.0, 0.3)
            } else {
                Color::srgb(1.0, 0.3, 0.2)
            };
            *vis = Visibility::Visible;
            return;
        }
    }

    // Check shops
    for (t, shop) in &shops {
        let sp = t.translation.truncate();
        if player_pos.distance(sp) < SHOP_INTERACT_RANGE {
            let can_afford = score.0 >= shop.cost;
            text.sections[0].value = if can_afford {
                format!("[E] BUY {} (${})", shop.weapon.label(), shop.cost)
            } else {
                format!("{} - NEED ${}", shop.weapon.label(), shop.cost)
            };
            text.sections[0].style.color = if can_afford {
                Color::srgb(0.2, 1.0, 0.3)
            } else {
                Color::srgb(1.0, 0.3, 0.2)
            };
            *vis = Visibility::Visible;
            return;
        }
    }

    *vis = Visibility::Hidden;
}

fn despawn_barriers_and_shops(
    mut commands: Commands,
    barriers: Query<(Entity, &Transform), With<ZoneBarrier>>,
    shops: Query<Entity, With<WeaponShop>>,
    decor: Query<Entity, With<BarrierDecor>>,
    fogs: Query<Entity, With<ZoneFog>>,
    particles: Query<Entity, With<UnlockParticle>>,
    mut obstacles: ResMut<MapObstacles>,
) {
    for (e, t) in &barriers {
        obstacles.remove_at(t.translation.truncate());
        commands.entity(e).despawn_recursive();
    }
    for e in &shops {
        commands.entity(e).despawn_recursive();
    }
    for e in &decor {
        commands.entity(e).despawn_recursive();
    }
    for e in &fogs {
        commands.entity(e).despawn_recursive();
    }
    for e in &particles {
        commands.entity(e).despawn_recursive();
    }
}

fn despawn_prompt(mut commands: Commands, q: Query<Entity, With<InteractionPrompt>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn build_dense_tree_image() -> Image {
    let dark: Rgba = [18, 42, 18, 255];
    let mid: Rgba = [28, 62, 28, 255];
    let light: Rgba = [38, 78, 32, 255];
    let highlight: Rgba = [50, 95, 38, 255];
    let trunk: Rgba = [42, 30, 16, 255];

    let mut c = Canvas::new(24, 24);
    c.fill_circle(12, 12, 11, dark);
    c.fill_circle(11, 11, 9, mid);
    c.fill_circle(9, 9, 6, light);
    c.fill_circle(14, 10, 5, light);
    c.fill_circle(8, 8, 3, highlight);
    c.fill_circle(15, 9, 2, highlight);
    c.fill_circle(12, 13, 2, trunk);
    c.into_image()
}

fn build_thick_bush_image() -> Image {
    let dark: Rgba = [22, 48, 20, 255];
    let mid: Rgba = [32, 68, 28, 255];
    let light: Rgba = [48, 82, 32, 255];

    let mut c = Canvas::new(16, 14);
    c.fill_circle(8, 7, 6, dark);
    c.fill_circle(5, 6, 4, mid);
    c.fill_circle(11, 7, 4, mid);
    c.fill_circle(7, 5, 3, light);
    c.fill_circle(11, 6, 2, light);
    c.into_image()
}

fn build_rubble_image() -> Image {
    let dark_stone: Rgba = [42, 40, 38, 255];
    let stone: Rgba = [72, 68, 62, 255];
    let light_stone: Rgba = [98, 92, 84, 255];
    let dirt: Rgba = [52, 38, 22, 255];
    let crack: Rgba = [25, 22, 18, 255];

    let mut c = Canvas::new(20, 16);
    c.fill_rect(1, 3, 18, 11, dirt);
    c.fill_circle(6, 8, 4, dark_stone);
    c.fill_circle(14, 7, 3, dark_stone);
    c.fill_circle(10, 11, 3, dark_stone);
    c.fill_circle(5, 7, 2, stone);
    c.fill_circle(13, 6, 2, stone);
    c.fill_circle(9, 10, 2, stone);
    c.put(4, 6, light_stone);
    c.put(12, 5, light_stone);
    c.put(8, 9, light_stone);
    c.put(7, 8, crack);
    c.put(14, 8, crack);
    c.into_image()
}

fn build_shop_image() -> Image {
    let outline: Rgba = [20, 20, 25, 255];
    let body: Rgba = [50, 55, 65, 255];
    let body_light: Rgba = [70, 75, 85, 255];
    let screen: Rgba = [30, 100, 30, 255];
    let screen_light: Rgba = [50, 160, 50, 255];
    let slot: Rgba = [15, 15, 18, 255];
    let trim: Rgba = [180, 160, 40, 255];
    let button: Rgba = [180, 40, 40, 255];

    let mut c = Canvas::new(18, 24);

    c.fill_rect(0, 0, 18, 24, outline);
    c.fill_rect(1, 1, 16, 22, body);
    c.fill_rect(1, 1, 16, 3, body_light);

    c.fill_rect(1, 0, 16, 1, trim);
    c.fill_rect(1, 23, 16, 1, trim);

    c.fill_rect(3, 3, 12, 8, outline);
    c.fill_rect(4, 4, 10, 6, screen);
    c.fill_rect(5, 5, 8, 4, screen_light);

    c.fill_rect(5, 13, 8, 4, slot);
    c.fill_rect(6, 14, 6, 2, body_light);

    c.fill_rect(13, 13, 3, 3, button);

    c.fill_rect(3, 18, 2, 3, slot);

    c.into_image()
}

fn build_leaf_image() -> Image {
    let green: Rgba = [60, 120, 40, 255];
    let mut c = Canvas::new(3, 3);
    c.put(1, 0, green);
    c.put(0, 1, green);
    c.put(1, 1, green);
    c.put(2, 1, green);
    c.put(1, 2, green);
    c.into_image()
}

fn build_dust_image() -> Image {
    let dust: Rgba = [140, 130, 110, 255];
    let mut c = Canvas::new(2, 2);
    c.fill_rect(0, 0, 2, 2, dust);
    c.into_image()
}

fn build_fog_image() -> Image {
    let mut c = Canvas::new(1, 1);
    c.put(0, 0, [255, 255, 255, 255]);
    c.into_image()
}

fn animate_fog_fading(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut Sprite, &mut ZoneFogFading)>,
) {
    let dt = time.delta_seconds();
    for (entity, mut sprite, mut fading) in &mut query {
        fading.elapsed += dt;
        let t = (fading.elapsed / FOG_FADE_DURATION).min(1.0);
        let alpha = FOG_ALPHA * (1.0 - t);
        sprite.color = Color::srgba(0.0, 0.02, 0.05, alpha);
        if t >= 1.0 {
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn animate_barrier_clearing(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(
        Entity,
        &mut Transform,
        &mut Sprite,
        &mut BarrierClearing,
        &BarrierDecor,
    )>,
    assets: Res<ZoneAssets>,
) {
    let dt = time.delta_seconds();
    let mut rng = rand::thread_rng();

    for (entity, mut transform, mut sprite, mut clearing, decor) in &mut query {
        clearing.elapsed += dt;

        if clearing.elapsed < clearing.delay {
            continue;
        }

        let active_time = clearing.elapsed - clearing.delay;
        let t = (active_time / clearing.fade_duration).min(1.0);

        // Spawn particles at the start of fade
        if !clearing.spawned_particles {
            clearing.spawned_particles = true;
            let pos = transform.translation.truncate();
            let is_underground = decor.zone_id == 3;
            let particle_tex = if is_underground {
                &assets.dust
            } else {
                &assets.leaf
            };
            for _ in 0..LEAF_BURST_PER_TREE {
                spawn_particle(&mut commands, particle_tex, pos, is_underground, &mut rng);
            }
        }

        // Fade out + shrink
        let alpha = (1.0 - t).max(0.0);
        let shrink = 1.0 - t * 0.3;
        sprite.color = Color::srgba(1.0, 1.0, 1.0, alpha);
        transform.scale = Vec3::splat(shrink);

        if t >= 1.0 {
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn animate_unlock_particles(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut Transform, &mut Sprite, &mut UnlockParticle)>,
) {
    let dt = time.delta_seconds();

    for (entity, mut transform, mut sprite, mut particle) in &mut query {
        particle.lifetime += dt;
        if particle.lifetime >= particle.max_lifetime {
            commands.entity(entity).despawn_recursive();
            continue;
        }

        transform.translation.x += particle.velocity.x * dt;
        transform.translation.y += particle.velocity.y * dt;
        particle.velocity.y -= 80.0 * dt;
        particle.velocity.x += (particle.lifetime * 5.0).sin() * 30.0 * dt;

        let t = particle.lifetime / particle.max_lifetime;
        let alpha = (1.0 - t).max(0.0);
        let c = &particle.base_color;
        sprite.color = Color::srgba(c[0], c[1], c[2], alpha);
    }
}

fn spawn_particle(
    commands: &mut Commands,
    texture: &Handle<Image>,
    pos: Vec2,
    is_underground: bool,
    rng: &mut impl Rng,
) {
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    let speed = rng.gen_range(30.0..120.0);
    let velocity = Vec2::new(angle.cos() * speed, angle.sin() * speed + 30.0);
    let max_lifetime = rng.gen_range(0.6..1.2);
    let size = rng.gen_range(3.0..6.0);
    let base_color = if is_underground {
        let g: f32 = rng.gen_range(0.3..0.5);
        [g + 0.1, g, (g - 0.05).max(0.0)]
    } else {
        match rng.gen_range(0..3) {
            0 => [0.2, 0.5, 0.1],
            1 => [0.4, 0.3, 0.1],
            _ => [0.3, 0.45, 0.15],
        }
    };

    commands.spawn((
        SpriteBundle {
            texture: texture.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(size, size)),
                color: Color::srgba(base_color[0], base_color[1], base_color[2], 1.0),
                ..default()
            },
            transform: Transform::from_xyz(
                pos.x + rng.gen_range(-8.0..8.0),
                pos.y + rng.gen_range(-8.0..8.0),
                5.0,
            ),
            ..default()
        },
        UnlockParticle {
            velocity,
            lifetime: 0.0,
            max_lifetime,
            base_color,
        },
    ));
}

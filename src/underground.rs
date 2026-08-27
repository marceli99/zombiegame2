//! Metro / podziemia level.  A separate large playspace at low world Y,
//! reachable by the manhole prop on segment 2 (Downtown).  Exit back to
//! the surface lives at the far end of the station platform.
//!
//! Walls and decor are pushed into the regular `MapObstacles` so player /
//! zombie collision works without a dedicated underground physics system.
//! `map_obstacles::UNDERGROUND_EXTENT_Y` ensures the spatial grid covers
//! the negative-Y region these obstacles live in.

use bevy::prelude::*;

use crate::map::{MapObstacles, Obstacle, ObstacleShape};
use crate::net::{is_authoritative, LocalInput, NetContext, RemoteInputs};
use crate::pixelart::{Canvas, Rgba};
use crate::player::{LogicalPos, Player};
use crate::{gameplay_active, GameState};

// ── Layout constants (world coordinates) ────────────────────────────────

/// Underground bounding rectangle.
pub const UNDER_W: f32 = 5000.0;
pub const UNDER_H: f32 = 1400.0;
pub const UNDER_CX: f32 = -1000.0;
pub const UNDER_CY: f32 = -2500.0;
pub const UNDER_TOP: f32 = UNDER_CY + UNDER_H * 0.5;
pub const UNDER_BOTTOM: f32 = UNDER_CY - UNDER_H * 0.5;
pub const UNDER_LEFT: f32 = UNDER_CX - UNDER_W * 0.5;
pub const UNDER_RIGHT: f32 = UNDER_CX + UNDER_W * 0.5;

/// Surface manhole position (segment 2, just south of the road).
pub const MANHOLE_X: f32 = -1536.0;
pub const MANHOLE_Y: f32 = -200.0;
const MANHOLE_INTERACT_RADIUS: f32 = 60.0;

/// Underground exit-stair position (east end of the station platform).
const EXIT_X: f32 = UNDER_RIGHT - 250.0;
const EXIT_Y: f32 = UNDER_TOP - 90.0;
const EXIT_INTERACT_RADIUS: f32 = 80.0;

/// Where the player materialises after dropping through the manhole — top
/// of the platform, west of the visible exit stairs.
const ENTRY_DROP_X: f32 = MANHOLE_X;
const ENTRY_DROP_Y: f32 = UNDER_TOP - 180.0;

/// Where the player surfaces after using the station exit — same spot as
/// the manhole on the surface (single hub on segment 2).
const SURFACE_RETURN_X: f32 = MANHOLE_X + 80.0;
const SURFACE_RETURN_Y: f32 = MANHOLE_Y;

/// Wall thickness for the underground perimeter.
const WALL_THICK: f32 = 16.0;

/// Y of the platform-track edge (everything below this Y is the rails
/// pit).  Just visual — no separate collision; players can walk down.
const PLATFORM_EDGE_Y: f32 = UNDER_CY - 80.0;

// ── Components ─────────────────────────────────────────────────────────

#[derive(Component)]
pub struct Manhole;

#[derive(Component)]
pub struct SubwayExit;

/// Tag for every entity spawned by this plugin so they're easy to clean
/// up on `OnExit(GameState::Playing)`.
#[derive(Component)]
pub struct UndergroundEntity;

pub struct UndergroundPlugin;

impl Plugin for UndergroundPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), spawn_underground)
            .add_systems(OnExit(GameState::Playing), despawn_underground)
            .add_systems(
                FixedUpdate,
                manhole_teleport_system
                    .run_if(gameplay_active)
                    .run_if(is_authoritative),
            );
    }
}

// ── Spawn ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn spawn_underground(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut obstacles: ResMut<MapObstacles>,
    mut obstacles_registered: Local<bool>,
) {
    // The underground layout is static, but this system re-runs on every
    // OnEnter(Playing).  Register the collision obstacles only on the first
    // run — re-pushing them each session would append duplicates to
    // `MapObstacles` (and shift the obstacle indices `DestroyedExplodables`
    // ships over the wire).  Sprites are still spawned per session since
    // `despawn_underground` removes them on exit.
    let register_obstacles = !*obstacles_registered;
    *obstacles_registered = true;

    // Floor — single big sprite for the whole bounding rect.  Tile texture
    // looks like a metro station floor (concrete slabs).
    let floor_tex = images.add(build_floor_tile_image());
    commands.spawn((
        SpriteBundle {
            texture: floor_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(UNDER_W, UNDER_H)),
                ..default()
            },
            transform: Transform::from_xyz(UNDER_CX, UNDER_CY, -10.0),
            ..default()
        },
        UndergroundEntity,
    ));

    // Track strip across the bottom — darker, with rails drawn on it.
    let track_y = UNDER_CY - UNDER_H * 0.25;
    let track_h = UNDER_H * 0.5;
    let track_tex = images.add(build_track_image());
    commands.spawn((
        SpriteBundle {
            texture: track_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(UNDER_W, track_h)),
                ..default()
            },
            transform: Transform::from_xyz(UNDER_CX, track_y, -9.5),
            ..default()
        },
        UndergroundEntity,
    ));

    // Yellow safety stripe along the platform edge — pure decor, no collision.
    let stripe_tex = images.add(build_stripe_image());
    commands.spawn((
        SpriteBundle {
            texture: stripe_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(UNDER_W - 200.0, 8.0)),
                ..default()
            },
            transform: Transform::from_xyz(UNDER_CX, PLATFORM_EDGE_Y, -9.0),
            ..default()
        },
        UndergroundEntity,
    ));

    // Outer perimeter walls (4 rectangles).  Each is `WALL_THICK` thick
    // sitting just inside the bounding rect so the visible floor reaches
    // exactly to the wall.
    push_perimeter_walls(&mut commands, &mut images, &mut obstacles, register_obstacles);

    // Pillars along the platform — periodic columns supporting the
    // ceiling.  Every 800 px, 3 pillars total.
    let pillar_tex = images.add(build_pillar_image());
    for i in 0..6 {
        let x = UNDER_LEFT + 600.0 + i as f32 * 800.0;
        if x > UNDER_RIGHT - 400.0 {
            break;
        }
        let y = UNDER_TOP - 180.0;
        commands.spawn((
            SpriteBundle {
                texture: pillar_tex.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::new(40.0, 56.0)),
                    ..default()
                },
                transform: Transform::from_xyz(x, y, -2.5),
                ..default()
            },
            UndergroundEntity,
        ));
        if register_obstacles {
            let half = Vec2::new(18.0, 22.0);
            obstacles.list.push(Obstacle {
                pos: Vec2::new(x, y),
                shape: ObstacleShape::Rect(half),
            });
        }
    }

    // Benches on the platform — clusters of two between pillars.
    let bench_tex = images.add(build_bench_image());
    for i in 0..3 {
        let x = UNDER_LEFT + 1000.0 + i as f32 * 1500.0;
        if x > UNDER_RIGHT - 600.0 {
            break;
        }
        let y = UNDER_TOP - 280.0;
        for off in [-40.0, 40.0] {
            commands.spawn((
                SpriteBundle {
                    texture: bench_tex.clone(),
                    sprite: Sprite {
                        custom_size: Some(Vec2::new(56.0, 18.0)),
                        ..default()
                    },
                    transform: Transform::from_xyz(x + off, y, -3.0),
                    ..default()
                },
                UndergroundEntity,
            ));
            if register_obstacles {
                obstacles.list.push(Obstacle {
                    pos: Vec2::new(x + off, y),
                    shape: ObstacleShape::Rect(Vec2::new(24.0, 7.0)),
                });
            }
        }
    }

    // Train wreck on the western tracks — long horizontal sprite.
    let train_tex = images.add(build_train_image());
    let train_pos = Vec2::new(UNDER_LEFT + 600.0, UNDER_CY - 200.0);
    commands.spawn((
        SpriteBundle {
            texture: train_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(900.0, 130.0)),
                ..default()
            },
            transform: Transform::from_xyz(train_pos.x, train_pos.y, -3.5),
            ..default()
        },
        UndergroundEntity,
    ));
    if register_obstacles {
        obstacles.list.push(Obstacle {
            pos: train_pos,
            shape: ObstacleShape::Rect(Vec2::new(440.0, 60.0)),
        });
    }

    // Vending machine & ticket booth on the eastern platform.
    let vendor_tex = images.add(build_vendor_image());
    let vendor_pos = Vec2::new(UNDER_RIGHT - 600.0, UNDER_TOP - 130.0);
    commands.spawn((
        SpriteBundle {
            texture: vendor_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(48.0, 64.0)),
                ..default()
            },
            transform: Transform::from_xyz(vendor_pos.x, vendor_pos.y, -3.0),
            ..default()
        },
        UndergroundEntity,
    ));
    if register_obstacles {
        obstacles.list.push(Obstacle {
            pos: vendor_pos,
            shape: ObstacleShape::Rect(Vec2::new(20.0, 26.0)),
        });
    }

    // Subway exit stairs on the east end.
    let exit_tex = images.add(build_exit_stairs_image());
    commands.spawn((
        SpriteBundle {
            texture: exit_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(96.0, 96.0)),
                ..default()
            },
            transform: Transform::from_xyz(EXIT_X, EXIT_Y, -3.0),
            ..default()
        },
        SubwayExit,
        UndergroundEntity,
    ));

    // Manhole on the surface (segment 2).
    let manhole_tex = images.add(build_manhole_image());
    commands.spawn((
        SpriteBundle {
            texture: manhole_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(48.0, 48.0)),
                ..default()
            },
            transform: Transform::from_xyz(MANHOLE_X, MANHOLE_Y, -2.4),
            ..default()
        },
        Manhole,
        UndergroundEntity,
    ));

    // Hidden surface barrier — seals the strip just south of the surface
    // map so a player squeezing through one of the segment-S spawn gaps
    // can't wander down into the empty void above the metro.  The wall
    // spans the full map width with no gaps; zombies still spawn inside
    // the surface (at +1.5 tiles inward) so this doesn't affect spawning.
    if register_obstacles {
        let surface_floor_y = -crate::map::MAP_HEIGHT * 0.5 - 32.0;
        obstacles.list.push(Obstacle {
            pos: Vec2::new(0.0, surface_floor_y),
            shape: ObstacleShape::Rect(Vec2::new(crate::map::MAP_WIDTH * 0.5, 16.0)),
        });

        // Bookkeeping: rebuild the spatial grid now that all underground
        // obstacles have been pushed.
        obstacles.rebuild_grid();
    }
}

fn push_perimeter_walls(
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    obstacles: &mut ResMut<MapObstacles>,
    register_obstacles: bool,
) {
    let wall_tex = images.add(build_wall_image());
    // Overlap horizontal and vertical walls at the corners by `WALL_THICK`
    // — without the overlap a player squeezing into a corner can get
    // pushed out at the diagonal where the two segments meet, leaking onto
    // the black void.  Cheap, foolproof, and visually invisible.
    let half_w_ext = UNDER_W * 0.5 + WALL_THICK;
    let half_h_ext = UNDER_H * 0.5;
    let segments = [
        // top
        (
            Vec2::new(UNDER_CX, UNDER_TOP - WALL_THICK * 0.5),
            Vec2::new(half_w_ext, WALL_THICK * 0.5),
        ),
        // bottom
        (
            Vec2::new(UNDER_CX, UNDER_BOTTOM + WALL_THICK * 0.5),
            Vec2::new(half_w_ext, WALL_THICK * 0.5),
        ),
        // left
        (
            Vec2::new(UNDER_LEFT + WALL_THICK * 0.5, UNDER_CY),
            Vec2::new(WALL_THICK * 0.5, half_h_ext),
        ),
        // right
        (
            Vec2::new(UNDER_RIGHT - WALL_THICK * 0.5, UNDER_CY),
            Vec2::new(WALL_THICK * 0.5, half_h_ext),
        ),
    ];
    // Visual sprites still use the inner half so the wall texture stops at
    // the bounding rect — only the collision rect is extended.
    let visual_segments = [
        (
            Vec2::new(UNDER_CX, UNDER_TOP - WALL_THICK * 0.5),
            Vec2::new(UNDER_W * 0.5, WALL_THICK * 0.5),
        ),
        (
            Vec2::new(UNDER_CX, UNDER_BOTTOM + WALL_THICK * 0.5),
            Vec2::new(UNDER_W * 0.5, WALL_THICK * 0.5),
        ),
        (
            Vec2::new(UNDER_LEFT + WALL_THICK * 0.5, UNDER_CY),
            Vec2::new(WALL_THICK * 0.5, UNDER_H * 0.5),
        ),
        (
            Vec2::new(UNDER_RIGHT - WALL_THICK * 0.5, UNDER_CY),
            Vec2::new(WALL_THICK * 0.5, UNDER_H * 0.5),
        ),
    ];
    for (pos, half) in visual_segments {
        commands.spawn((
            SpriteBundle {
                texture: wall_tex.clone(),
                sprite: Sprite {
                    custom_size: Some(half * 2.0),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, -4.0),
                ..default()
            },
            UndergroundEntity,
        ));
    }
    if register_obstacles {
        for (pos, half) in segments {
            obstacles.list.push(Obstacle {
                pos,
                shape: ObstacleShape::Rect(half),
            });
        }
    }
}

fn despawn_underground(mut commands: Commands, q: Query<Entity, With<UndergroundEntity>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

// ── Teleport ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn manhole_teleport_system(
    mut local_input: ResMut<LocalInput>,
    mut remote_inputs: ResMut<RemoteInputs>,
    ctx: Res<NetContext>,
    mut players: Query<(&mut Transform, &Player, Option<&mut LogicalPos>), Without<Manhole>>,
    manholes: Query<&Transform, (With<Manhole>, Without<Player>, Without<SubwayExit>)>,
    exits: Query<&Transform, (With<SubwayExit>, Without<Player>, Without<Manhole>)>,
) {
    let manhole_pos = manholes
        .get_single()
        .ok()
        .map(|t| t.translation.truncate());
    let exit_pos = exits.get_single().ok().map(|t| t.translation.truncate());
    if manhole_pos.is_none() && exit_pos.is_none() {
        return;
    }

    for (mut t, player, lp) in players.iter_mut() {
        let interact = if player.id == ctx.my_id {
            local_input.0.interact
        } else {
            remote_inputs
                .0
                .get(&player.id)
                .map(|i| i.interact)
                .unwrap_or(false)
        };
        if !interact {
            continue;
        }
        let pp = t.translation.truncate();
        let mut teleported = false;

        if let Some(m) = manhole_pos {
            if pp.distance_squared(m) <= MANHOLE_INTERACT_RADIUS * MANHOLE_INTERACT_RADIUS {
                t.translation.x = ENTRY_DROP_X;
                t.translation.y = ENTRY_DROP_Y;
                teleported = true;
            }
        }
        if !teleported {
            if let Some(e) = exit_pos {
                if pp.distance_squared(e) <= EXIT_INTERACT_RADIUS * EXIT_INTERACT_RADIUS {
                    t.translation.x = SURFACE_RETURN_X;
                    t.translation.y = SURFACE_RETURN_Y;
                    teleported = true;
                }
            }
        }

        if teleported {
            // Sync logical pos so render-side interpolation doesn't lerp the
            // player from old → new position over a frame.
            if let Some(mut lp) = lp {
                lp.curr = t.translation.truncate();
                lp.prev = lp.curr;
            }
            // Consume the press so other interact-listeners (segment unlock)
            // don't double-fire.
            if player.id == ctx.my_id {
                local_input.0.interact = false;
            } else if let Some(input) = remote_inputs.0.get_mut(&player.id) {
                input.interact = false;
            }
        }
    }
}

// ── Sprites ────────────────────────────────────────────────────────────

fn build_floor_tile_image() -> Image {
    // Concrete stipple — no explicit grid lines (those would stretch into
    // huge bars when scaled to the full underground bounding rect).  The
    // noise reads as worn, dirty subway-station concrete.
    let base: Rgba = [82, 80, 88, 255];
    let dark: Rgba = [54, 52, 60, 255];
    let hi: Rgba = [108, 106, 114, 255];
    let stain: Rgba = [44, 38, 36, 255];
    let mut c = Canvas::new(96, 96);
    c.fill_rect(0, 0, 96, 96, base);
    for y in 0..96 {
        for x in 0..96 {
            let n = (x * 73 + y * 131 + x * y) & 0xFF;
            if n < 30 {
                c.put(x, y, dark);
            } else if n < 56 {
                c.put(x, y, hi);
            } else if n < 60 {
                c.put(x, y, stain);
            }
        }
    }
    c.into_image()
}

fn build_track_image() -> Image {
    let bed: Rgba = [42, 40, 46, 255];
    let gravel: Rgba = [62, 60, 64, 255];
    let sleeper: Rgba = [54, 38, 22, 255];
    let rail: Rgba = [180, 180, 188, 255];
    let rail_d: Rgba = [110, 110, 118, 255];
    let mut c = Canvas::new(96, 64);
    c.fill_rect(0, 0, 96, 64, bed);
    // Stipple gravel
    for y in 0..64 {
        for x in 0..96 {
            let n = (x * 31 + y * 41 + x * y) & 0xFF;
            if n < 30 {
                c.put(x, y, gravel);
            }
        }
    }
    // Sleepers (cross-ties) every 12 px
    for sx in (0..96).step_by(12) {
        c.fill_rect(sx, 18, 8, 4, sleeper);
        c.fill_rect(sx, 42, 8, 4, sleeper);
    }
    // Two rails — long horizontal lines with metallic highlight
    c.fill_rect(0, 22, 96, 2, rail);
    c.fill_rect(0, 24, 96, 1, rail_d);
    c.fill_rect(0, 38, 96, 1, rail_d);
    c.fill_rect(0, 39, 96, 2, rail);
    c.into_image()
}

fn build_stripe_image() -> Image {
    let yellow: Rgba = [228, 196, 60, 255];
    let dark: Rgba = [120, 100, 30, 255];
    let mut c = Canvas::new(64, 8);
    c.fill_rect(0, 0, 64, 8, yellow);
    c.fill_rect(0, 0, 64, 1, dark);
    c.fill_rect(0, 7, 64, 1, dark);
    // Black tick marks
    for sx in (4..60).step_by(8) {
        c.fill_rect(sx, 1, 2, 6, [40, 32, 10, 255]);
    }
    c.into_image()
}

fn build_wall_image() -> Image {
    let stone: Rgba = [70, 68, 76, 255];
    let stone_d: Rgba = [38, 36, 44, 255];
    let stone_h: Rgba = [110, 108, 116, 255];
    let mut c = Canvas::new(32, 16);
    c.fill_rect(0, 0, 32, 16, stone);
    c.fill_rect(0, 0, 32, 1, stone_h);
    c.fill_rect(0, 15, 32, 1, stone_d);
    // Horizontal mortar lines (rows of bricks)
    c.fill_rect(0, 5, 32, 1, stone_d);
    c.fill_rect(0, 10, 32, 1, stone_d);
    // Vertical staggered seams
    for x in [4, 12, 20, 28] {
        c.fill_rect(x, 0, 1, 5, stone_d);
        c.fill_rect(x + 4, 5, 1, 5, stone_d);
        c.fill_rect(x, 10, 1, 6, stone_d);
    }
    c.into_image()
}

fn build_pillar_image() -> Image {
    let metal: Rgba = [110, 108, 116, 255];
    let metal_d: Rgba = [60, 58, 66, 255];
    let metal_h: Rgba = [180, 178, 186, 255];
    let bolt: Rgba = [200, 196, 92, 255];
    let mut c = Canvas::new(20, 28);
    c.fill_rect(0, 0, 20, 28, [0, 0, 0, 0]);
    c.fill_rect(2, 0, 16, 28, metal);
    c.fill_rect(2, 0, 16, 1, metal_h);
    c.fill_rect(2, 27, 16, 1, metal_d);
    c.fill_rect(2, 0, 1, 28, metal_d);
    c.fill_rect(17, 0, 1, 28, metal_d);
    // Cross-bracing band across the middle
    c.fill_rect(2, 12, 16, 4, metal_d);
    c.fill_rect(2, 13, 16, 1, metal_h);
    // Bolt heads on the band
    c.put(5, 14, bolt);
    c.put(10, 14, bolt);
    c.put(14, 14, bolt);
    c.into_image()
}

fn build_bench_image() -> Image {
    let wood: Rgba = [98, 70, 44, 255];
    let wood_d: Rgba = [54, 38, 22, 255];
    let wood_h: Rgba = [148, 110, 70, 255];
    let metal: Rgba = [70, 70, 78, 255];
    let mut c = Canvas::new(28, 9);
    c.fill_rect(0, 0, 28, 9, [0, 0, 0, 0]);
    // Two horizontal slats
    c.fill_rect(2, 1, 24, 3, wood);
    c.fill_rect(2, 1, 24, 1, wood_h);
    c.fill_rect(2, 3, 24, 1, wood_d);
    c.fill_rect(2, 5, 24, 3, wood);
    c.fill_rect(2, 5, 24, 1, wood_h);
    c.fill_rect(2, 7, 24, 1, wood_d);
    // End legs
    c.fill_rect(0, 0, 2, 9, metal);
    c.fill_rect(26, 0, 2, 9, metal);
    c.into_image()
}

fn build_train_image() -> Image {
    let body: Rgba = [120, 50, 50, 255];
    let body_d: Rgba = [70, 26, 26, 255];
    let body_h: Rgba = [200, 80, 70, 255];
    let window: Rgba = [80, 90, 110, 255];
    let window_h: Rgba = [160, 180, 200, 255];
    let metal: Rgba = [70, 70, 78, 255];
    let mut c = Canvas::new(180, 26);
    c.fill_rect(0, 0, 180, 26, [0, 0, 0, 0]);
    // Main body
    c.fill_rect(4, 2, 172, 22, body);
    c.fill_rect(4, 2, 172, 2, body_h);
    c.fill_rect(4, 22, 172, 2, body_d);
    c.fill_rect(4, 2, 2, 22, body_d);
    c.fill_rect(174, 2, 2, 22, body_d);
    // Cab nose
    c.fill_rect(0, 6, 4, 14, body);
    c.fill_rect(0, 6, 4, 1, body_d);
    c.fill_rect(0, 19, 4, 1, body_d);
    // Headlight
    c.fill_rect(0, 11, 2, 4, [240, 230, 130, 255]);
    // Windows along the side
    for wx in (16..170).step_by(16) {
        c.fill_rect(wx, 6, 10, 8, window);
        c.fill_rect(wx, 6, 10, 1, window_h);
        c.fill_rect(wx, 13, 10, 1, body_d);
    }
    // Wheels
    for sx in [10, 40, 80, 120, 150] {
        c.fill_rect(sx, 23, 8, 3, metal);
    }
    // Damage scorch on the side
    c.fill_rect(60, 16, 24, 6, [40, 30, 22, 255]);
    c.fill_rect(62, 17, 6, 4, [180, 80, 30, 200]);
    c.into_image()
}

fn build_vendor_image() -> Image {
    let body: Rgba = [60, 130, 180, 255];
    let body_d: Rgba = [30, 70, 110, 255];
    let body_h: Rgba = [110, 170, 220, 255];
    let glass: Rgba = [190, 220, 240, 255];
    let frame: Rgba = [40, 40, 50, 255];
    let mut c = Canvas::new(24, 32);
    c.fill_rect(0, 0, 24, 32, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 24, 32, body);
    c.fill_rect(0, 0, 24, 1, body_h);
    c.fill_rect(0, 31, 24, 1, body_d);
    c.fill_rect(0, 0, 1, 32, body_d);
    c.fill_rect(23, 0, 1, 32, body_d);
    // Glass display
    c.fill_rect(2, 2, 20, 18, glass);
    c.fill_rect(2, 2, 20, 1, frame);
    c.fill_rect(2, 19, 20, 1, frame);
    c.fill_rect(2, 2, 1, 18, frame);
    c.fill_rect(21, 2, 1, 18, frame);
    // Items inside
    for col in 0..4 {
        for row in 0..3 {
            let x = 4 + col * 5;
            let y = 4 + row * 5;
            let col_a = match (col + row) % 3 {
                0 => [220, 60, 50, 255] as Rgba,
                1 => [60, 200, 100, 255],
                _ => [220, 200, 70, 255],
            };
            c.fill_rect(x, y, 3, 3, col_a);
        }
    }
    // Coin slot + button
    c.fill_rect(4, 22, 6, 1, frame);
    c.fill_rect(14, 22, 6, 4, [40, 40, 50, 255]);
    c.put(17, 24, [220, 60, 50, 255]);
    c.into_image()
}

fn build_exit_stairs_image() -> Image {
    let stone: Rgba = [120, 118, 126, 255];
    let stone_d: Rgba = [70, 68, 76, 255];
    let stone_h: Rgba = [180, 178, 186, 255];
    let arrow: Rgba = [220, 200, 92, 255];
    let mut c = Canvas::new(48, 48);
    c.fill_rect(0, 0, 48, 48, [0, 0, 0, 0]);
    // Outer landing
    c.fill_rect(0, 0, 48, 48, stone);
    c.fill_rect(0, 0, 48, 1, stone_h);
    c.fill_rect(0, 47, 48, 1, stone_d);
    c.fill_rect(0, 0, 1, 48, stone_d);
    c.fill_rect(47, 0, 1, 48, stone_d);
    // Stair steps — staircase rises north
    for i in 0..8 {
        let y = 4 + i * 4;
        c.fill_rect(8, y, 32, 4, if i % 2 == 0 { stone_d } else { stone });
        c.fill_rect(8, y, 32, 1, [40, 38, 44, 255]);
    }
    // Up arrow on the bottom face
    c.fill_rect(22, 38, 4, 8, arrow);
    c.fill_rect(18, 40, 12, 2, arrow);
    c.put(19, 39, arrow);
    c.put(28, 39, arrow);
    c.into_image()
}

fn build_manhole_image() -> Image {
    let frame: Rgba = [50, 48, 52, 255];
    let lid: Rgba = [78, 76, 80, 255];
    let lid_d: Rgba = [42, 40, 44, 255];
    let lid_h: Rgba = [110, 108, 112, 255];
    let mut c = Canvas::new(24, 24);
    c.fill_rect(0, 0, 24, 24, [0, 0, 0, 0]);
    let cx = 12i32;
    let cy = 12i32;
    let r = 11i32;
    // Outer frame ring
    for y in 0..24 {
        for x in 0..24 {
            let dx = x - cx;
            let dy = y - cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= r * r && d2 >= (r - 1) * (r - 1) {
                c.put(x, y, frame);
            }
        }
    }
    // Lid disk
    for y in 0..24 {
        for x in 0..24 {
            let dx = x - cx;
            let dy = y - cy;
            let d2 = dx * dx + dy * dy;
            if d2 < (r - 1) * (r - 1) {
                let shade = (dx + dy) as f32 / 12.0;
                let col = if shade < -0.4 {
                    lid_h
                } else if shade > 0.4 {
                    lid_d
                } else {
                    lid
                };
                c.put(x, y, col);
            }
        }
    }
    // Crosshatch grip pattern
    c.fill_rect(cx - 5, cy - 1, 10, 1, lid_d);
    c.fill_rect(cx - 5, cy + 1, 10, 1, lid_d);
    c.fill_rect(cx - 1, cy - 5, 1, 10, lid_d);
    c.fill_rect(cx + 1, cy - 5, 1, 10, lid_d);
    // Tiny "M" stamp for METRO
    c.put(cx - 3, cy + 4, [200, 196, 92, 255]);
    c.put(cx + 2, cy + 4, [200, 196, 92, 255]);
    c.put(cx, cy + 4, [200, 196, 92, 255]);
    c.into_image()
}

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::net::NetContext;
use crate::pixelart::{Canvas, Rgba};
use crate::player::Player;
use crate::settings::GraphicsSettings;
use crate::GameState;

pub const TILE_SIZE: f32 = 64.0;
pub const MAP_COLS: i32 = 51;
pub const MAP_ROWS: i32 = 37;
pub const MAP_WIDTH: f32 = MAP_COLS as f32 * TILE_SIZE;
pub const MAP_HEIGHT: f32 = MAP_ROWS as f32 * TILE_SIZE;

pub const ZONE0_ROW_MIN: i32 = 10;
pub const ZONE0_ROW_MAX: i32 = 26;
pub const ZONE1_ROW_MIN: i32 = 27;
pub const ZONE1_ROW_MAX: i32 = 36;
pub const ZONE2_ROW_MIN: i32 = 5;
pub const ZONE2_ROW_MAX: i32 = 9;
pub const ZONE3_ROW_MIN: i32 = 0;
pub const ZONE3_ROW_MAX: i32 = 4;

pub const BARRIER_NORTH_Y: f32 = 544.0;
pub const BARRIER_SOUTH_Y: f32 = -544.0;
pub const BARRIER_UNDERGROUND_Y: f32 = -864.0;

const ROAD_HALF_HEIGHT: f32 = 48.0;

#[derive(Clone, Copy)]
pub enum ObstacleShape {
    Circle(f32),
    Rect(Vec2),
}

#[derive(Clone, Copy)]
pub struct Obstacle {
    pub pos: Vec2,
    pub shape: ObstacleShape,
}

#[derive(Resource, Default)]
pub struct MapObstacles {
    pub list: Vec<Obstacle>,
}

impl MapObstacles {
    pub fn resolve(&self, pos: &mut Vec2, own_radius: f32) {
        for o in &self.list {
            match o.shape {
                ObstacleShape::Circle(r) => {
                    let delta = *pos - o.pos;
                    let min_dist = r + own_radius;
                    let dist_sq = delta.length_squared();
                    if dist_sq < min_dist * min_dist {
                        if dist_sq > 0.0001 {
                            let dist = dist_sq.sqrt();
                            *pos += delta / dist * (min_dist - dist);
                        } else {
                            *pos += Vec2::new(min_dist, 0.0);
                        }
                    }
                }
                ObstacleShape::Rect(half) => {
                    let delta = *pos - o.pos;
                    let clamped = Vec2::new(
                        delta.x.clamp(-half.x, half.x),
                        delta.y.clamp(-half.y, half.y),
                    );
                    let closest = o.pos + clamped;
                    let diff = *pos - closest;
                    let dist_sq = diff.length_squared();
                    if dist_sq < own_radius * own_radius {
                        if dist_sq > 0.0001 {
                            let dist = dist_sq.sqrt();
                            *pos = closest + diff / dist * own_radius;
                        } else {
                            let dx_left = delta.x + half.x;
                            let dx_right = half.x - delta.x;
                            let dy_bot = delta.y + half.y;
                            let dy_top = half.y - delta.y;
                            let min_x = dx_left.min(dx_right);
                            let min_y = dy_bot.min(dy_top);
                            if min_x < min_y {
                                if dx_left < dx_right {
                                    pos.x = o.pos.x - half.x - own_radius;
                                } else {
                                    pos.x = o.pos.x + half.x + own_radius;
                                }
                            } else if dy_bot < dy_top {
                                pos.y = o.pos.y - half.y - own_radius;
                            } else {
                                pos.y = o.pos.y + half.y + own_radius;
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn hits(&self, pos: Vec2, own_radius: f32) -> bool {
        for o in &self.list {
            match o.shape {
                ObstacleShape::Circle(r) => {
                    let min_dist = r + own_radius;
                    if pos.distance_squared(o.pos) < min_dist * min_dist {
                        return true;
                    }
                }
                ObstacleShape::Rect(half) => {
                    let delta = pos - o.pos;
                    let clamped = Vec2::new(
                        delta.x.clamp(-half.x, half.x),
                        delta.y.clamp(-half.y, half.y),
                    );
                    let closest = o.pos + clamped;
                    if pos.distance_squared(closest) < own_radius * own_radius {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn remove_at(&mut self, pos: Vec2) {
        self.list.retain(|o| o.pos.distance_squared(pos) > 4.0);
    }
}

#[derive(Component)]
struct RainDrop {
    velocity: Vec2,
}

#[derive(Component)]
struct Firefly {
    base_pos: Vec2,
    phase: f32,
    drift_speed: f32,
    drift_radius: f32,
}

#[derive(Component)]
struct FogWisp {
    speed: f32,
    fade_phase: f32,
}

#[derive(Component)]
struct CabinExterior {
    cabin_idx: usize,
}

#[derive(Component)]
struct CabinInterior {
    cabin_idx: usize,
}

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapObstacles>()
            .init_resource::<NavGrid>()
            .add_systems(Startup, (spawn_map, spawn_rain, spawn_ultra_effects))
            .add_systems(
                Update,
                (update_rain, toggle_cabin_interior, update_fireflies, update_fog_wisps)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

pub fn tile_center(col: i32, row: i32) -> Vec2 {
    Vec2::new(
        -MAP_WIDTH / 2.0 + (col as f32 + 0.5) * TILE_SIZE,
        -MAP_HEIGHT / 2.0 + (row as f32 + 0.5) * TILE_SIZE,
    )
}

#[derive(Resource)]
pub struct NavGrid {
    pub walkable: Vec<bool>,
    pub player_flow: HashMap<u8, Vec<u16>>,
    pub player_flow_tile: HashMap<u8, (i32, i32)>,
}

impl Default for NavGrid {
    fn default() -> Self {
        let total = (MAP_COLS * MAP_ROWS) as usize;
        let mut walkable = vec![false; total];
        for row in ZONE0_ROW_MIN..=ZONE0_ROW_MAX {
            for col in 0..MAP_COLS {
                walkable[(row * MAP_COLS + col) as usize] = is_walkable_tile(col, row);
            }
        }
        Self {
            walkable,
            player_flow: HashMap::new(),
            player_flow_tile: HashMap::new(),
        }
    }
}

pub fn unlock_nav_rows(nav: &mut NavGrid, row_min: i32, row_max: i32) {
    for row in row_min..=row_max {
        for col in 0..MAP_COLS {
            nav.walkable[(row * MAP_COLS + col) as usize] = is_walkable_tile(col, row);
        }
    }
    nav.player_flow.clear();
    nav.player_flow_tile.clear();
}

pub fn is_walkable_tile(col: i32, row: i32) -> bool {
    let center = tile_center(col, row);
    for cabin in CABINS {
        let d = center - cabin.pos;
        if d.x.abs() < cabin.half.x - 2.0 && d.y.abs() < cabin.half.y - 2.0 {
            return false;
        }
    }
    for wreck in WRECK_SPOTS {
        let d = center - *wreck;
        if d.x.abs() < WRECK_HALF.x - 4.0 && d.y.abs() < WRECK_HALF.y - 4.0 {
            return false;
        }
    }
    true
}

pub fn in_bounds(col: i32, row: i32) -> bool {
    (0..MAP_COLS).contains(&col) && (0..MAP_ROWS).contains(&row)
}

pub fn nav_idx(col: i32, row: i32) -> usize {
    (row * MAP_COLS + col) as usize
}

pub fn world_to_tile(pos: Vec2) -> (i32, i32) {
    let col = ((pos.x + MAP_WIDTH / 2.0) / TILE_SIZE).floor() as i32;
    let row = ((pos.y + MAP_HEIGHT / 2.0) / TILE_SIZE).floor() as i32;
    (col, row)
}

pub fn bfs_distance_field(walkable: &[bool], start: Vec2) -> Vec<u16> {
    let total = (MAP_COLS * MAP_ROWS) as usize;
    let mut dist = vec![u16::MAX; total];
    let (sc, sr) = world_to_tile(start);
    let (sc, sr) = snap_to_walkable(walkable, sc, sr);
    if !in_bounds(sc, sr) || !walkable[nav_idx(sc, sr)] {
        return dist;
    }
    dist[nav_idx(sc, sr)] = 0;
    let mut queue: VecDeque<(i32, i32)> = VecDeque::with_capacity(total);
    queue.push_back((sc, sr));
    let dirs: [(i32, i32); 8] = [
        (-1, 0), (1, 0), (0, -1), (0, 1),
        (-1, -1), (-1, 1), (1, -1), (1, 1),
    ];
    while let Some((c, r)) = queue.pop_front() {
        let d = dist[nav_idx(c, r)];
        for &(dc, dr) in &dirs {
            let (nc, nr) = (c + dc, r + dr);
            if !in_bounds(nc, nr) {
                continue;
            }
            let ni = nav_idx(nc, nr);
            if !walkable[ni] {
                continue;
            }
            if dc != 0 && dr != 0
                && (!walkable[nav_idx(c + dc, r)] || !walkable[nav_idx(c, r + dr)])
            {
                continue;
            }
            if dist[ni] > d + 1 {
                dist[ni] = d + 1;
                queue.push_back((nc, nr));
            }
        }
    }
    dist
}

fn snap_to_walkable(walkable: &[bool], col: i32, row: i32) -> (i32, i32) {
    if in_bounds(col, row) && walkable[nav_idx(col, row)] {
        return (col, row);
    }
    for ring in 1_i32..=6 {
        for dr in -ring..=ring {
            for dc in -ring..=ring {
                if dc.abs() != ring && dr.abs() != ring {
                    continue;
                }
                let (c, r) = (col + dc, row + dr);
                if in_bounds(c, r) && walkable[nav_idx(c, r)] {
                    return (c, r);
                }
            }
        }
    }
    (col, row)
}

struct CabinSpec {
    pos: Vec2,
    half: Vec2,
    door_side: DoorSide,
    kind: BuildingKind,
}

#[derive(Clone, Copy)]
enum DoorSide {
    South,
    North,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BuildingKind {
    Cabin,
    House,
    Store,
}

const CABIN_HALF: Vec2 = Vec2::new(96.0, 72.0);
const HOUSE_HALF: Vec2 = Vec2::new(104.0, 80.0);
const STORE_HALF: Vec2 = Vec2::new(120.0, 72.0);
const CABIN_DOOR_WIDTH: f32 = 28.0;
const CABIN_WALL_THICK: f32 = 7.0;

const CABINS: &[CabinSpec] = &[
    // Zone 0 (center)
    CabinSpec { pos: Vec2::new(-720.0, 156.0), half: HOUSE_HALF, door_side: DoorSide::South, kind: BuildingKind::House },
    CabinSpec { pos: Vec2::new(-200.0, 148.0), half: STORE_HALF, door_side: DoorSide::South, kind: BuildingKind::Store },
    CabinSpec { pos: Vec2::new(340.0, 148.0), half: CABIN_HALF, door_side: DoorSide::South, kind: BuildingKind::Cabin },
    CabinSpec { pos: Vec2::new(-440.0, -156.0), half: HOUSE_HALF, door_side: DoorSide::North, kind: BuildingKind::House },
    CabinSpec { pos: Vec2::new(160.0, -148.0), half: CABIN_HALF, door_side: DoorSide::North, kind: BuildingKind::Cabin },
    // Zone 1 (north)
    CabinSpec { pos: Vec2::new(-600.0, 740.0), half: HOUSE_HALF, door_side: DoorSide::South, kind: BuildingKind::House },
    CabinSpec { pos: Vec2::new(200.0, 780.0), half: STORE_HALF, door_side: DoorSide::South, kind: BuildingKind::Store },
    CabinSpec { pos: Vec2::new(800.0, 720.0), half: CABIN_HALF, door_side: DoorSide::South, kind: BuildingKind::Cabin },
    // Zone 2 (south surface)
    CabinSpec { pos: Vec2::new(-500.0, -680.0), half: CABIN_HALF, door_side: DoorSide::North, kind: BuildingKind::Cabin },
    CabinSpec { pos: Vec2::new(400.0, -700.0), half: HOUSE_HALF, door_side: DoorSide::North, kind: BuildingKind::House },
];

const WRECK_SPOTS: &[Vec2] = &[
    Vec2::new(-620.0, -20.0),
    Vec2::new(-140.0, 24.0),
    Vec2::new(260.0, -38.0),
    Vec2::new(720.0, 18.0),
];
const WRECK_HALF: Vec2 = Vec2::new(30.0, 13.0);

fn on_road(p: Vec2) -> bool {
    p.y.abs() < ROAD_HALF_HEIGHT
}

fn near_cabin(p: Vec2, padding: f32) -> bool {
    for cabin in CABINS {
        let dx = (p.x - cabin.pos.x).abs();
        let dy = (p.y - cabin.pos.y).abs();
        if dx < cabin.half.x + padding && dy < cabin.half.y + padding {
            return true;
        }
    }
    false
}

fn push_cabin_walls(obstacles: &mut MapObstacles, cabin: &CabinSpec) {
    let half = cabin.half;
    let t = CABIN_WALL_THICK * 0.5;
    let door_half = CABIN_DOOR_WIDTH * 0.5;

    let (door_y, solid_y) = match cabin.door_side {
        DoorSide::South => (-half.y + t, half.y - t),
        DoorSide::North => (half.y - t, -half.y + t),
    };

    obstacles.list.push(Obstacle {
        pos: cabin.pos + Vec2::new(0.0, solid_y),
        shape: ObstacleShape::Rect(Vec2::new(half.x, t)),
    });

    let side_len = half.x - door_half;
    if side_len > 0.0 {
        let off = door_half + side_len * 0.5;
        obstacles.list.push(Obstacle {
            pos: cabin.pos + Vec2::new(-off, door_y),
            shape: ObstacleShape::Rect(Vec2::new(side_len * 0.5, t)),
        });
        obstacles.list.push(Obstacle {
            pos: cabin.pos + Vec2::new(off, door_y),
            shape: ObstacleShape::Rect(Vec2::new(side_len * 0.5, t)),
        });
    }

    let side_h = half.y - t;
    obstacles.list.push(Obstacle {
        pos: cabin.pos + Vec2::new(-half.x + t, 0.0),
        shape: ObstacleShape::Rect(Vec2::new(t, side_h)),
    });
    obstacles.list.push(Obstacle {
        pos: cabin.pos + Vec2::new(half.x - t, 0.0),
        shape: ObstacleShape::Rect(Vec2::new(t, side_h)),
    });
}

fn toggle_cabin_interior(
    ctx: Res<NetContext>,
    players: Query<(&Transform, &Player)>,
    mut ext: Query<
        (&CabinExterior, &mut Visibility),
        Without<CabinInterior>,
    >,
    mut interior: Query<
        (&CabinInterior, &mut Visibility),
        Without<CabinExterior>,
    >,
) {
    let target = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .or_else(|| players.iter().next())
        .map(|(t, _)| t.translation.truncate());
    let Some(pos) = target else {
        return;
    };

    let mut inside: Option<usize> = None;
    for (i, cabin) in CABINS.iter().enumerate() {
        let d = pos - cabin.pos;
        let inner = cabin.half - Vec2::splat(CABIN_WALL_THICK);
        if d.x.abs() < inner.x && d.y.abs() < inner.y {
            inside = Some(i);
            break;
        }
    }

    for (e, mut vis) in &mut ext {
        let should_hide = inside == Some(e.cabin_idx);
        let want = if should_hide {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if *vis != want {
            *vis = want;
        }
    }
    for (i, mut vis) in &mut interior {
        let should_show = inside == Some(i.cabin_idx);
        let want = if should_show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
}

fn plaza_clear(p: Vec2) -> bool {
    p.length_squared() < 160.0 * 160.0
}

fn spawn_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut obstacles: ResMut<MapObstacles>,
    gfx: Res<GraphicsSettings>,
) {
    let preset = gfx.quality_preset();
    let tile_mesh = meshes.add(Rectangle::new(TILE_SIZE, TILE_SIZE));
    let leaf_mesh = meshes.add(Rectangle::new(3.0, 3.0));
    let twig_mesh = meshes.add(Rectangle::new(8.0, 1.5));
    let road_mesh = meshes.add(Rectangle::new(MAP_WIDTH, ROAD_HALF_HEIGHT * 2.0));
    let road_shoulder_mesh = meshes.add(Rectangle::new(MAP_WIDTH, 6.0));
    let road_edge_mesh = meshes.add(Rectangle::new(MAP_WIDTH, 1.5));
    let dash_mesh = meshes.add(Rectangle::new(16.0, 2.2));
    let tire_track_mesh = meshes.add(Rectangle::new(44.0, 1.6));
    let pothole_mesh = meshes.add(Ellipse::new(10.0, 6.0));

    let dirt_patch_mats = [
        materials.add(Color::srgb(0.16, 0.12, 0.07)),
        materials.add(Color::srgb(0.18, 0.14, 0.08)),
    ];
    let road_main_mat = materials.add(Color::srgb(0.05, 0.05, 0.058));
    let road_shoulder_mat = materials.add(Color::srgba(0.09, 0.075, 0.045, 0.9));
    let road_edge_mat = materials.add(Color::srgba(0.015, 0.015, 0.02, 0.95));
    let dash_mat = materials.add(Color::srgb(0.28, 0.22, 0.06));
    let tire_mat = materials.add(Color::srgba(0.018, 0.018, 0.022, 0.7));
    let pothole_mat = materials.add(Color::srgba(0.01, 0.01, 0.012, 0.9));

    let leaf_mats = [
        materials.add(Color::srgb(0.25, 0.18, 0.06)),
        materials.add(Color::srgb(0.18, 0.13, 0.04)),
        materials.add(Color::srgb(0.12, 0.2, 0.08)),
        materials.add(Color::srgb(0.3, 0.22, 0.08)),
    ];
    let twig_mat = materials.add(Color::srgb(0.12, 0.08, 0.03));
    let overcast_mat = materials.add(Color::srgba(0.04, 0.06, 0.1, 0.18));

    let pine_a_image = images.add(build_pine_image(0));
    let pine_b_image = images.add(build_pine_image(1));
    let pine_c_image = images.add(build_pine_image(2));
    let cabin_image = images.add(build_cabin_image());
    let house_image = images.add(build_house_image());
    let store_image = images.add(build_store_image());
    let log_image = images.add(build_log_image());
    let stump_image = images.add(build_stump_image());
    let stone_image = images.add(build_stone_image());
    let bush_image = images.add(build_bush_image());
    let fern_image = images.add(build_fern_image());
    let grass_image = images.add(build_grass_image());
    let wreck_image = images.add(build_wrecked_car_image());
    let ground_tiles: Vec<Handle<Image>> = (0..6)
        .map(|i| images.add(build_ground_tile_image(i)))
        .collect();
    let underground_tiles: Vec<Handle<Image>> = (0..4)
        .map(|i| images.add(build_underground_tile_image(i)))
        .collect();
    let pillar_image = images.add(build_pillar_image());

    let mut rng = StdRng::seed_from_u64(2027);

    for row in 0..MAP_ROWS {
        for col in 0..MAP_COLS {
            let center = tile_center(col, row);
            let texture = if row <= ZONE3_ROW_MAX {
                underground_tiles[rng.gen_range(0..underground_tiles.len())].clone()
            } else {
                ground_tiles[rng.gen_range(0..ground_tiles.len())].clone()
            };
            commands.spawn(SpriteBundle {
                texture,
                sprite: Sprite {
                    custom_size: Some(Vec2::splat(TILE_SIZE + 1.0)),
                    ..default()
                },
                transform: Transform::from_xyz(center.x, center.y, -10.0),
                ..default()
            });
        }
    }

    let half_w = MAP_WIDTH / 2.0 - 14.0;
    let half_h = MAP_HEIGHT / 2.0 - 14.0;

    for _ in 0..preset.dirt {
        let x = rng.gen_range(-half_w..half_w);
        let y = rng.gen_range(-half_h..half_h);
        let p = Vec2::new(x, y);
        if on_road(p) || p.y < BARRIER_UNDERGROUND_Y {
            continue;
        }
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(tile_mesh.clone()),
            material: dirt_patch_mats[rng.gen_range(0..dirt_patch_mats.len())].clone(),
            transform: Transform::from_xyz(x, y, -9.82)
                .with_scale(Vec3::new(
                    rng.gen_range(0.6..1.0),
                    rng.gen_range(0.6..1.0),
                    1.0,
                )),
            ..default()
        });
    }

    commands.spawn(MaterialMesh2dBundle {
        mesh: Mesh2dHandle(road_shoulder_mesh.clone()),
        material: road_shoulder_mat.clone(),
        transform: Transform::from_xyz(0.0, ROAD_HALF_HEIGHT + 2.0, -9.62),
        ..default()
    });
    commands.spawn(MaterialMesh2dBundle {
        mesh: Mesh2dHandle(road_shoulder_mesh),
        material: road_shoulder_mat,
        transform: Transform::from_xyz(0.0, -ROAD_HALF_HEIGHT - 2.0, -9.62),
        ..default()
    });
    commands.spawn(MaterialMesh2dBundle {
        mesh: Mesh2dHandle(road_mesh.clone()),
        material: road_main_mat,
        transform: Transform::from_xyz(0.0, 0.0, -9.6),
        ..default()
    });
    commands.spawn(MaterialMesh2dBundle {
        mesh: Mesh2dHandle(road_edge_mesh.clone()),
        material: road_edge_mat.clone(),
        transform: Transform::from_xyz(0.0, ROAD_HALF_HEIGHT - 1.0, -9.55),
        ..default()
    });
    commands.spawn(MaterialMesh2dBundle {
        mesh: Mesh2dHandle(road_edge_mesh),
        material: road_edge_mat,
        transform: Transform::from_xyz(0.0, -ROAD_HALF_HEIGHT + 1.0, -9.55),
        ..default()
    });
    let mut dx = -MAP_WIDTH / 2.0 + 40.0;
    while dx < MAP_WIDTH / 2.0 - 20.0 {
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(dash_mesh.clone()),
            material: dash_mat.clone(),
            transform: Transform::from_xyz(dx, 0.0, -9.5),
            ..default()
        });
        dx += 56.0;
    }
    for &track_y in &[-18.0_f32, 18.0] {
        let mut tx = -MAP_WIDTH / 2.0 + 60.0;
        while tx < MAP_WIDTH / 2.0 - 40.0 {
            commands.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(tire_track_mesh.clone()),
                material: tire_mat.clone(),
                transform: Transform::from_xyz(tx, track_y, -9.48),
                ..default()
            });
            tx += 90.0 + rng.gen_range(-20.0..20.0);
        }
    }
    for _ in 0..7 {
        let px = rng.gen_range(-MAP_WIDTH / 2.0 + 60.0..MAP_WIDTH / 2.0 - 60.0);
        let py = rng.gen_range(-ROAD_HALF_HEIGHT + 8.0..ROAD_HALF_HEIGHT - 8.0);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(pothole_mesh.clone()),
            material: pothole_mat.clone(),
            transform: Transform::from_xyz(px, py, -9.49)
                .with_scale(Vec3::new(
                    rng.gen_range(0.6..1.2),
                    rng.gen_range(0.6..1.0),
                    1.0,
                )),
            ..default()
        });
    }

    for _ in 0..preset.leaves {
        let x = rng.gen_range(-half_w..half_w);
        let y = rng.gen_range(-half_h..half_h);
        if y < BARRIER_UNDERGROUND_Y { continue; }
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(leaf_mesh.clone()),
            material: leaf_mats[rng.gen_range(0..leaf_mats.len())].clone(),
            transform: Transform::from_xyz(x, y, -9.3),
            ..default()
        });
    }
    for _ in 0..preset.twigs {
        let x = rng.gen_range(-half_w..half_w);
        let y = rng.gen_range(-half_h..half_h);
        if y < BARRIER_UNDERGROUND_Y { continue; }
        let rot = rng.gen_range(-1.5_f32..1.5);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(twig_mesh.clone()),
            material: twig_mat.clone(),
            transform: Transform::from_xyz(x, y, -9.25)
                .with_rotation(Quat::from_rotation_z(rot)),
            ..default()
        });
    }

    let cabin_interior_image = images.add(build_cabin_interior_image());
    let house_interior_image = images.add(build_house_interior_image());
    let store_interior_image = images.add(build_store_interior_image());
    for (i, cabin) in CABINS.iter().enumerate() {
        let (ext_tex, int_tex) = match cabin.kind {
            BuildingKind::Cabin => (cabin_image.clone(), cabin_interior_image.clone()),
            BuildingKind::House => (house_image.clone(), house_interior_image.clone()),
            BuildingKind::Store => (store_image.clone(), store_interior_image.clone()),
        };
        commands.spawn((
            SpriteBundle {
                texture: int_tex,
                sprite: Sprite {
                    custom_size: Some(cabin.half * 2.0),
                    ..default()
                },
                transform: Transform::from_xyz(cabin.pos.x, cabin.pos.y, -2.3),
                visibility: Visibility::Hidden,
                ..default()
            },
            CabinInterior { cabin_idx: i },
        ));
        commands.spawn((
            SpriteBundle {
                texture: ext_tex,
                sprite: Sprite {
                    custom_size: Some(cabin.half * 2.0),
                    ..default()
                },
                transform: Transform::from_xyz(cabin.pos.x, cabin.pos.y, -2.0),
                ..default()
            },
            CabinExterior { cabin_idx: i },
        ));
        push_cabin_walls(&mut obstacles, cabin);
    }

    let pine_textures = [pine_a_image.clone(), pine_b_image.clone(), pine_c_image.clone()];
    let tree_min_dist = 48.0;
    let mut attempts = 0;
    let mut placed_trees = 0;
    while placed_trees < preset.trees && attempts < 2600 {
        attempts += 1;
        let x = rng.gen_range(-half_w + 30.0..half_w - 30.0);
        let y = rng.gen_range(-half_h + 30.0..half_h - 30.0);
        let p = Vec2::new(x, y);
        if p.y < BARRIER_UNDERGROUND_Y {
            continue;
        }
        if plaza_clear(p) {
            continue;
        }
        if on_road(p) {
            continue;
        }
        if near_cabin(p, 24.0) {
            continue;
        }
        if obstacles.hits(p, tree_min_dist) {
            continue;
        }
        let texture = pine_textures[rng.gen_range(0..pine_textures.len())].clone();
        let size = rng.gen_range(46.0_f32..56.0);
        let rot = rng.gen_range(-0.18_f32..0.18);
        commands.spawn(SpriteBundle {
            texture,
            sprite: Sprite {
                custom_size: Some(Vec2::splat(size)),
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -1.5)
                .with_rotation(Quat::from_rotation_z(rot)),
            ..default()
        });
        obstacles.list.push(Obstacle {
            pos: p,
            shape: ObstacleShape::Circle(15.0),
        });
        placed_trees += 1;
    }

    for &p in WRECK_SPOTS {
        let rot = rng.gen_range(-0.6_f32..0.6);
        commands.spawn(SpriteBundle {
            texture: wreck_image.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(72.0, 34.0)),
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -3.0)
                .with_rotation(Quat::from_rotation_z(rot)),
            ..default()
        });
        obstacles.list.push(Obstacle {
            pos: p,
            shape: ObstacleShape::Rect(WRECK_HALF),
        });
    }

    let mut placed_props = 0;
    let mut prop_attempts = 0;
    while placed_props < preset.props && prop_attempts < 1000 {
        prop_attempts += 1;
        let x = rng.gen_range(-half_w..half_w);
        let y = rng.gen_range(-half_h..half_h);
        let p = Vec2::new(x, y);
        if p.y < BARRIER_UNDERGROUND_Y || plaza_clear(p) || on_road(p) || near_cabin(p, 6.0) {
            continue;
        }
        if obstacles.hits(p, 18.0) {
            continue;
        }
        let kind = rng.gen_range(0..5);
        match kind {
            0 => {
                let rot = rng.gen_range(-1.2_f32..1.2);
                commands.spawn(SpriteBundle {
                    texture: log_image.clone(),
                    sprite: Sprite {
                        custom_size: Some(Vec2::new(44.0, 14.0)),
                        ..default()
                    },
                    transform: Transform::from_xyz(p.x, p.y, -2.3)
                        .with_rotation(Quat::from_rotation_z(rot)),
                    ..default()
                });
                let (sin_r, cos_r) = rot.sin_cos();
                obstacles.list.push(Obstacle {
                    pos: p,
                    shape: ObstacleShape::Rect(Vec2::new(
                        20.0 * cos_r.abs() + 7.0 * sin_r.abs(),
                        7.0 * cos_r.abs() + 20.0 * sin_r.abs(),
                    )),
                });
            }
            1 => {
                commands.spawn(SpriteBundle {
                    texture: stump_image.clone(),
                    sprite: Sprite {
                        custom_size: Some(Vec2::new(22.0, 22.0)),
                        ..default()
                    },
                    transform: Transform::from_xyz(p.x, p.y, -2.2),
                    ..default()
                });
                obstacles.list.push(Obstacle {
                    pos: p,
                    shape: ObstacleShape::Circle(9.0),
                });
            }
            2 => {
                commands.spawn(SpriteBundle {
                    texture: stone_image.clone(),
                    sprite: Sprite {
                        custom_size: Some(Vec2::new(28.0, 20.0)),
                        ..default()
                    },
                    transform: Transform::from_xyz(p.x, p.y, -2.1),
                    ..default()
                });
                obstacles.list.push(Obstacle {
                    pos: p,
                    shape: ObstacleShape::Circle(10.0),
                });
            }
            3 => {
                commands.spawn(SpriteBundle {
                    texture: bush_image.clone(),
                    sprite: Sprite {
                        custom_size: Some(Vec2::new(26.0, 22.0)),
                        ..default()
                    },
                    transform: Transform::from_xyz(p.x, p.y, -2.0),
                    ..default()
                });
                obstacles.list.push(Obstacle {
                    pos: p,
                    shape: ObstacleShape::Circle(9.0),
                });
            }
            _ => {
                commands.spawn(SpriteBundle {
                    texture: fern_image.clone(),
                    sprite: Sprite {
                        custom_size: Some(Vec2::new(22.0, 22.0)),
                        ..default()
                    },
                    transform: Transform::from_xyz(p.x, p.y, -1.9),
                    ..default()
                });
            }
        }
        placed_props += 1;
    }

    for _ in 0..preset.grass {
        let x = rng.gen_range(-half_w..half_w);
        let y = rng.gen_range(-half_h..half_h);
        let p = Vec2::new(x, y);
        if p.y < BARRIER_UNDERGROUND_Y || on_road(p) || near_cabin(p, 2.0) {
            continue;
        }
        if obstacles.hits(p, 6.0) {
            continue;
        }
        let size_x = rng.gen_range(10.0_f32..16.0);
        let size_y = rng.gen_range(8.0_f32..12.0);
        let rot = rng.gen_range(-0.4_f32..0.4);
        commands.spawn(SpriteBundle {
            texture: grass_image.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(size_x, size_y)),
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -2.5)
                .with_rotation(Quat::from_rotation_z(rot)),
            ..default()
        });
    }

    for _ in 0..preset.bushes {
        let x = rng.gen_range(-half_w..half_w);
        let y = rng.gen_range(-half_h..half_h);
        let p = Vec2::new(x, y);
        if p.y < BARRIER_UNDERGROUND_Y || on_road(p) || near_cabin(p, 4.0) {
            continue;
        }
        if obstacles.hits(p, 10.0) {
            continue;
        }
        let size = rng.gen_range(16.0_f32..22.0);
        commands.spawn(SpriteBundle {
            texture: bush_image.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(size * 1.1, size * 0.9)),
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -2.15),
            ..default()
        });
    }

    // Underground pillars (zone 3)
    let ug_y_min = -MAP_HEIGHT / 2.0 + 40.0;
    let ug_y_max = BARRIER_UNDERGROUND_Y - 40.0;
    let mut placed_pillars = 0;
    let mut pillar_attempts = 0;
    while placed_pillars < 14 && pillar_attempts < 200 {
        pillar_attempts += 1;
        let x = rng.gen_range(-half_w + 60.0..half_w - 60.0);
        let y = rng.gen_range(ug_y_min..ug_y_max);
        let p = Vec2::new(x, y);
        if near_cabin(p, 30.0) || obstacles.hits(p, 30.0) {
            continue;
        }
        commands.spawn(SpriteBundle {
            texture: pillar_image.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::splat(24.0)),
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -1.5),
            ..default()
        });
        obstacles.list.push(Obstacle {
            pos: p,
            shape: ObstacleShape::Circle(10.0),
        });
        placed_pillars += 1;
    }

    // Map boundary walls
    let bw = 8.0;
    let hw = MAP_WIDTH / 2.0;
    let hh = MAP_HEIGHT / 2.0;
    obstacles.list.push(Obstacle { pos: Vec2::new(0.0, hh + bw), shape: ObstacleShape::Rect(Vec2::new(hw, bw)) });
    obstacles.list.push(Obstacle { pos: Vec2::new(0.0, -hh - bw), shape: ObstacleShape::Rect(Vec2::new(hw, bw)) });
    obstacles.list.push(Obstacle { pos: Vec2::new(-hw - bw, 0.0), shape: ObstacleShape::Rect(Vec2::new(bw, hh)) });
    obstacles.list.push(Obstacle { pos: Vec2::new(hw + bw, 0.0), shape: ObstacleShape::Rect(Vec2::new(bw, hh)) });

    commands.spawn(MaterialMesh2dBundle {
        mesh: Mesh2dHandle(meshes.add(Rectangle::new(MAP_WIDTH, MAP_HEIGHT))),
        material: overcast_mat,
        transform: Transform::from_xyz(0.0, 0.0, 30.0),
        ..default()
    });
}

fn spawn_rain(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    gfx: Res<GraphicsSettings>,
) {
    let rain_count = gfx.quality_preset().rain;
    let rain_mesh = meshes.add(Rectangle::new(0.9, 9.0));
    let rain_mat = materials.add(Color::srgba(0.72, 0.8, 0.92, 0.22));
    let mut rng = StdRng::seed_from_u64(99);
    let half_w = 900.0;
    let half_h = 550.0;
    for _ in 0..rain_count {
        let x = rng.gen_range(-half_w..half_w);
        let y = rng.gen_range(-half_h..half_h);
        commands.spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(rain_mesh.clone()),
                material: rain_mat.clone(),
                transform: Transform::from_xyz(x, y, 40.0)
                    .with_rotation(Quat::from_rotation_z(0.12)),
                ..default()
            },
            RainDrop {
                velocity: Vec2::new(-70.0, -720.0),
            },
        ));
    }

    // Ultra: second layer of fine, faster rain for depth
    if gfx.quality_idx >= 3 {
        let fine_mesh = meshes.add(Rectangle::new(0.5, 5.5));
        let fine_mat = materials.add(Color::srgba(0.65, 0.72, 0.85, 0.12));
        for _ in 0..50 {
            let x = rng.gen_range(-half_w..half_w);
            let y = rng.gen_range(-half_h..half_h);
            commands.spawn((
                MaterialMesh2dBundle {
                    mesh: Mesh2dHandle(fine_mesh.clone()),
                    material: fine_mat.clone(),
                    transform: Transform::from_xyz(x, y, 38.0)
                        .with_rotation(Quat::from_rotation_z(0.1)),
                    ..default()
                },
                RainDrop {
                    velocity: Vec2::new(-50.0, -920.0),
                },
            ));
        }
    }
}

fn update_rain(
    time: Res<Time>,
    mut drops: Query<(&mut Transform, &RainDrop), Without<Camera>>,
    camera: Query<&Transform, With<Camera>>,
) {
    let dt = time.delta_seconds();
    let Ok(cam) = camera.get_single() else {
        return;
    };
    let cx = cam.translation.x;
    let cy = cam.translation.y;
    let half_w = 900.0;
    let half_h = 560.0;
    let mut rng = rand::thread_rng();
    for (mut t, drop) in &mut drops {
        t.translation.x += drop.velocity.x * dt;
        t.translation.y += drop.velocity.y * dt;

        if t.translation.y < cy - half_h
            || t.translation.x < cx - half_w
            || t.translation.x > cx + half_w
        {
            t.translation.x = cx + rng.gen_range(-half_w..half_w);
            t.translation.y = cy + half_h + rng.gen_range(0.0..120.0);
        }
    }
}

// ── Ultra-quality visual effects ──────────────────────────────────

fn spawn_ultra_effects(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    gfx: Res<GraphicsSettings>,
) {
    if gfx.quality_idx < 3 {
        return;
    }

    // Fireflies / ambient dust motes
    let firefly_mesh = meshes.add(Circle::new(1.8));
    let firefly_mats = [
        materials.add(Color::srgba(0.85, 0.92, 0.3, 0.55)),
        materials.add(Color::srgba(0.7, 0.95, 0.45, 0.45)),
        materials.add(Color::srgba(0.95, 0.88, 0.25, 0.5)),
    ];
    let mut rng = StdRng::seed_from_u64(7777);
    let half_w = MAP_WIDTH / 2.0 - 40.0;
    let half_h = MAP_HEIGHT / 2.0 - 40.0;

    for _ in 0..45 {
        let x = rng.gen_range(-half_w..half_w);
        let y = rng.gen_range(-half_h..half_h);
        let mat = firefly_mats[rng.gen_range(0..firefly_mats.len())].clone();
        commands.spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(firefly_mesh.clone()),
                material: mat,
                transform: Transform::from_xyz(x, y, 25.0),
                ..default()
            },
            Firefly {
                base_pos: Vec2::new(x, y),
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
                drift_speed: rng.gen_range(0.3..0.8),
                drift_radius: rng.gen_range(12.0..30.0),
            },
        ));
    }

    // Ground fog wisps
    let fog_mesh = meshes.add(Ellipse::new(60.0, 12.0));
    let fog_mat = materials.add(Color::srgba(0.55, 0.6, 0.7, 0.08));
    for _ in 0..30 {
        let x = rng.gen_range(-half_w..half_w);
        let y = rng.gen_range(-half_h..half_h);
        commands.spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(fog_mesh.clone()),
                material: fog_mat.clone(),
                transform: Transform::from_xyz(x, y, 20.0)
                    .with_scale(Vec3::new(
                        rng.gen_range(0.6..1.5),
                        rng.gen_range(0.7..1.3),
                        1.0,
                    )),
                ..default()
            },
            FogWisp {
                speed: rng.gen_range(8.0..22.0),
                fade_phase: rng.gen_range(0.0..std::f32::consts::TAU),
            },
        ));
    }
}

fn update_fireflies(
    time: Res<Time>,
    mut fireflies: Query<(&mut Transform, &Firefly)>,
) {
    let t = time.elapsed_seconds();
    for (mut transform, ff) in &mut fireflies {
        let angle = ff.phase + t * ff.drift_speed;
        let secondary = ff.phase * 1.7 + t * ff.drift_speed * 0.6;
        transform.translation.x = ff.base_pos.x
            + angle.cos() * ff.drift_radius
            + secondary.sin() * ff.drift_radius * 0.4;
        transform.translation.y = ff.base_pos.y
            + angle.sin() * ff.drift_radius * 0.7
            + secondary.cos() * ff.drift_radius * 0.3;

        // Pulse scale for a gentle glow flicker
        let pulse = (t * ff.drift_speed * 2.5 + ff.phase).sin() * 0.3 + 0.85;
        transform.scale = Vec3::splat(pulse);
    }
}

fn update_fog_wisps(
    time: Res<Time>,
    mut wisps: Query<(&mut Transform, &FogWisp)>,
) {
    let t = time.elapsed_seconds();
    let dt = time.delta_seconds();
    for (mut transform, wisp) in &mut wisps {
        transform.translation.x += wisp.speed * dt;

        // Wrap around camera-independent — just wrap across map
        let half_w = MAP_WIDTH / 2.0 + 80.0;
        if transform.translation.x > half_w {
            transform.translation.x = -half_w;
        }

        // Gentle vertical oscillation
        let osc = (t * 0.4 + wisp.fade_phase).sin() * 0.8;
        transform.translation.y += osc * dt * 6.0;

        // Pulse opacity via scale
        let alpha = (t * 0.3 + wisp.fade_phase).sin() * 0.25 + 0.85;
        transform.scale.x = transform.scale.x.abs() * alpha.max(0.4);
    }
}

fn build_pine_image(variant: u8) -> Image {
    let outline: Rgba = [10, 22, 8, 255];
    let (leaf_dark, leaf_main, leaf_light, leaf_top, trunk) = match variant % 3 {
        0 => (
            [22, 55, 18, 255] as Rgba,
            [42, 94, 28, 255] as Rgba,
            [72, 132, 42, 255] as Rgba,
            [108, 172, 58, 255] as Rgba,
            [58, 34, 16, 255] as Rgba,
        ),
        1 => (
            [18, 46, 16, 255] as Rgba,
            [36, 82, 24, 255] as Rgba,
            [62, 118, 36, 255] as Rgba,
            [94, 156, 50, 255] as Rgba,
            [50, 30, 14, 255] as Rgba,
        ),
        _ => (
            [26, 62, 22, 255] as Rgba,
            [48, 104, 34, 255] as Rgba,
            [80, 144, 48, 255] as Rgba,
            [118, 184, 64, 255] as Rgba,
            [64, 38, 18, 255] as Rgba,
        ),
    };

    let mut c = Canvas::new(30, 30);

    c.fill_circle(15, 15, 13, outline);
    c.fill_circle(15, 15, 12, leaf_dark);
    c.fill_circle(14, 16, 10, leaf_main);
    c.fill_circle(12, 18, 7, leaf_light);
    c.fill_circle(11, 19, 4, leaf_top);

    c.put(15, 15, trunk);
    c.put(16, 15, trunk);
    c.put(15, 14, trunk);

    c.put(19, 20, leaf_light);
    c.put(21, 14, leaf_main);
    c.put(7, 13, leaf_dark);
    c.put(22, 10, leaf_dark);
    c.put(9, 22, leaf_main);
    c.put(17, 23, leaf_light);
    c.put(23, 17, leaf_dark);
    c.put(6, 17, leaf_dark);

    c.into_image()
}

fn build_cabin_image() -> Image {
    let transparent: Rgba = [0, 0, 0, 0];
    let outline: Rgba = [6, 4, 2, 255];
    let shingle_dark: Rgba = [18, 10, 4, 255];
    let shingle_main: Rgba = [38, 22, 8, 255];
    let shingle_light: Rgba = [56, 32, 12, 255];
    let shingle_hi: Rgba = [72, 42, 16, 255];
    let ridge: Rgba = [92, 58, 22, 255];
    let moss: Rgba = [28, 50, 20, 255];
    let moss_dark: Rgba = [14, 28, 12, 255];
    let chimney_out: Rgba = [10, 8, 8, 255];
    let chimney: Rgba = [44, 40, 40, 255];
    let chimney_hi: Rgba = [70, 64, 64, 255];
    let chimney_top: Rgba = [22, 20, 20, 255];
    let smoke_a: Rgba = [70, 66, 68, 180];
    let smoke_b: Rgba = [54, 50, 52, 140];
    let smoke_c: Rgba = [40, 38, 40, 90];
    let porch: Rgba = [26, 16, 6, 255];
    let step: Rgba = [54, 32, 12, 255];

    let w = 96;
    let h = 72;
    let mut c = Canvas::new(w, h);

    let margin_x = 4;
    let margin_y = 4;
    c.fill_rect(margin_x, margin_y, w - margin_x * 2, h - margin_y * 2, outline);
    c.fill_rect(
        margin_x + 1,
        margin_y + 1,
        w - margin_x * 2 - 2,
        h - margin_y * 2 - 2,
        shingle_main,
    );

    for y in (margin_y + 1)..(h - margin_y - 1) {
        for x in (margin_x + 1)..(w - margin_x - 1) {
            let d = (y - h / 2).abs();
            let mid = h / 2 - d;
            if mid < 4 && (x + y) % 2 == 0 {
                c.put(x, y, shingle_hi);
            } else if d > 20 {
                c.put(x, y, shingle_dark);
            } else if d > 14 && (x + y) % 3 == 0 {
                c.put(x, y, shingle_light);
            }
        }
    }

    for y in (margin_y + 3)..(h - margin_y - 3) {
        if (y - margin_y) % 3 == 0 {
            for x in (margin_x + 2)..(w - margin_x - 2) {
                if (x * 2 + y) % 5 == 0 {
                    c.put(x, y, shingle_dark);
                }
            }
        }
    }
    let ridge_y = h / 2;
    c.fill_rect(margin_x + 2, ridge_y - 1, w - margin_x * 2 - 4, 1, ridge);
    c.fill_rect(margin_x + 2, ridge_y, w - margin_x * 2 - 4, 1, shingle_hi);

    for (mx, my, sz) in [
        (12, 12, 4),
        (80, 14, 3),
        (20, 58, 4),
        (72, 56, 3),
        (48, 10, 2),
        (50, 62, 2),
    ] {
        c.fill_rect(mx, my, sz, 2, moss);
        c.put(mx + 1, my + 1, moss_dark);
        c.put(mx + 2, my, moss_dark);
    }

    let cx = 70;
    let cy = 10;
    c.fill_rect(cx, cy, 9, 11, chimney_out);
    c.fill_rect(cx + 1, cy + 1, 7, 9, chimney);
    c.fill_rect(cx + 1, cy + 1, 7, 2, chimney_top);
    c.fill_rect(cx + 6, cy + 3, 2, 6, chimney_hi);

    c.put(cx + 3, cy - 2, smoke_a);
    c.put(cx + 4, cy - 2, smoke_a);
    c.put(cx + 3, cy - 4, smoke_b);
    c.put(cx + 5, cy - 4, smoke_b);
    c.put(cx + 2, cy - 5, smoke_c);
    c.put(cx + 4, cy - 6, smoke_c);
    c.put(cx + 6, cy - 5, smoke_c);

    let porch_margin = 14;
    c.fill_rect(
        porch_margin,
        h - margin_y - 2,
        w - porch_margin * 2,
        1,
        porch,
    );
    c.fill_rect(porch_margin + 4, h - margin_y - 1, 6, 1, step);
    c.fill_rect(w - porch_margin - 10, h - margin_y - 1, 6, 1, step);

    c.put(margin_x, margin_y, transparent);
    c.put(w - margin_x - 1, margin_y, transparent);
    c.put(margin_x, h - margin_y - 1, transparent);
    c.put(w - margin_x - 1, h - margin_y - 1, transparent);

    c.into_image()
}

fn build_cabin_interior_image() -> Image {
    let transparent: Rgba = [0, 0, 0, 0];
    let wall_out: Rgba = [6, 4, 2, 255];
    let wall_mid: Rgba = [42, 26, 10, 255];
    let wall_hi: Rgba = [72, 46, 18, 255];
    let wall_shadow: Rgba = [22, 14, 6, 255];
    let floor_dark: Rgba = [52, 32, 14, 255];
    let floor_main: Rgba = [80, 52, 22, 255];
    let floor_hi: Rgba = [112, 76, 32, 255];
    let plank_gap: Rgba = [18, 10, 4, 255];
    let rug_a: Rgba = [120, 30, 30, 255];
    let rug_b: Rgba = [70, 18, 18, 255];
    let rug_trim: Rgba = [200, 160, 60, 255];
    let bed_frame: Rgba = [32, 20, 10, 255];
    let bed_sheet: Rgba = [210, 220, 220, 255];
    let bed_sheet_shadow: Rgba = [160, 170, 175, 255];
    let bed_pillow: Rgba = [240, 240, 240, 255];
    let bed_blanket: Rgba = [80, 40, 30, 255];
    let table: Rgba = [96, 58, 22, 255];
    let table_hi: Rgba = [140, 92, 36, 255];
    let table_dark: Rgba = [42, 24, 8, 255];
    let chair: Rgba = [70, 42, 16, 255];
    let fireplace_stone_d: Rgba = [36, 36, 42, 255];
    let fireplace_stone_m: Rgba = [64, 64, 72, 255];
    let fireplace_stone_l: Rgba = [100, 100, 110, 255];
    let fire_inner: Rgba = [250, 200, 60, 255];
    let fire_core: Rgba = [255, 240, 160, 255];
    let fire_outer: Rgba = [220, 80, 20, 255];
    let ember_dot: Rgba = [255, 150, 40, 255];
    let shelf: Rgba = [48, 30, 12, 255];
    let shelf_hi: Rgba = [92, 58, 22, 255];
    let jar_a: Rgba = [160, 180, 120, 255];
    let jar_b: Rgba = [120, 80, 40, 255];
    let book_a: Rgba = [130, 30, 30, 255];
    let book_b: Rgba = [30, 60, 120, 255];
    let book_c: Rgba = [50, 100, 60, 255];
    let crate_dark: Rgba = [46, 28, 10, 255];
    let crate_main: Rgba = [86, 54, 22, 255];
    let crate_hi: Rgba = [130, 84, 36, 255];
    let lantern_dark: Rgba = [12, 10, 8, 255];
    let lantern_glow: Rgba = [255, 200, 90, 255];

    let w = 96;
    let h = 72;
    let mut c = Canvas::new(w, h);

    let mx = 4;
    let my = 4;
    c.fill_rect(mx, my, w - mx * 2, h - my * 2, wall_out);
    c.fill_rect(mx + 1, my + 1, w - mx * 2 - 2, h - my * 2 - 2, wall_mid);
    c.fill_rect(mx + 2, my + 2, w - mx * 2 - 4, h - my * 2 - 4, wall_shadow);

    let fx0 = mx + 6;
    let fy0 = my + 6;
    let fw = w - mx * 2 - 12;
    let fh = h - my * 2 - 12;
    c.fill_rect(fx0, fy0, fw, fh, floor_main);
    for y in fy0..(fy0 + fh) {
        if (y - fy0) % 4 == 0 {
            c.fill_rect(fx0, y, fw, 1, plank_gap);
        } else if (y - fy0) % 4 == 1 {
            for x in fx0..(fx0 + fw) {
                if (x * 3 + y * 2) % 11 == 0 {
                    c.put(x, y, floor_hi);
                }
                if (x * 5 + y) % 13 == 0 {
                    c.put(x, y, floor_dark);
                }
            }
        }
    }

    c.fill_rect(mx + 1, my + 1, w - mx * 2 - 2, 1, wall_hi);
    c.fill_rect(mx + 1, my + 1, 1, h - my * 2 - 2, wall_hi);

    let rx = 36;
    let ry = 30;
    let rw = 22;
    let rh = 14;
    c.fill_rect(rx - 1, ry - 1, rw + 2, rh + 2, rug_trim);
    c.fill_rect(rx, ry, rw, rh, rug_a);
    for y in ry..(ry + rh) {
        for x in rx..(rx + rw) {
            if (x + y) % 2 == 0 {
                c.put(x, y, rug_b);
            }
        }
    }
    for x in rx..(rx + rw) {
        c.put(x, ry, rug_trim);
        c.put(x, ry + rh - 1, rug_trim);
    }
    for y in ry..(ry + rh) {
        c.put(rx, y, rug_trim);
        c.put(rx + rw - 1, y, rug_trim);
    }

    let bx = 8;
    let by = 10;
    let bw = 22;
    let bh = 14;
    c.fill_rect(bx, by, bw, bh, bed_frame);
    c.fill_rect(bx + 1, by + 1, bw - 2, bh - 2, bed_sheet);
    c.fill_rect(bx + 1, by + bh - 3, bw - 2, 2, bed_sheet_shadow);
    c.fill_rect(bx + 1, by + 1, bw - 2, 4, bed_pillow);
    c.fill_rect(bx + 1, by + 5, bw - 2, 1, bed_sheet_shadow);
    c.fill_rect(bx + 1, by + 9, bw - 2, 5, bed_blanket);
    c.put(bx + 3, by + 2, bed_sheet_shadow);
    c.put(bx + 18, by + 2, bed_sheet_shadow);

    let tx = 60;
    let ty = 46;
    let tw = 18;
    let th = 12;
    c.fill_rect(tx, ty, tw, th, table_dark);
    c.fill_rect(tx + 1, ty + 1, tw - 2, th - 2, table);
    c.fill_rect(tx + 1, ty + 1, tw - 2, 1, table_hi);
    c.put(tx, ty + th, table_dark);
    c.put(tx + tw - 1, ty + th, table_dark);
    c.put(tx + 4, ty + 4, lantern_dark);
    c.put(tx + 4, ty + 3, lantern_glow);
    c.put(tx + 5, ty + 3, lantern_glow);
    c.put(tx + 5, ty + 4, lantern_dark);
    c.put(tx + 11, ty + 5, book_a);
    c.put(tx + 12, ty + 5, book_a);
    c.put(tx + 11, ty + 6, book_b);
    c.put(tx + 13, ty + 7, book_c);

    for (cxp, cyp) in [(tx - 4, ty + 3), (tx + tw + 2, ty + 3)] {
        c.fill_rect(cxp, cyp, 4, 4, chair);
        c.fill_rect(cxp, cyp - 2, 4, 2, chair);
        c.put(cxp, cyp + 4, table_dark);
        c.put(cxp + 3, cyp + 4, table_dark);
    }

    let fpx = 42;
    let fpy = 6;
    let fpw = 20;
    let fph = 8;
    c.fill_rect(fpx, fpy, fpw, fph, fireplace_stone_d);
    for y in fpy..(fpy + fph) {
        for x in fpx..(fpx + fpw) {
            let sel = (x / 3 + y / 2) % 3;
            match sel {
                0 => c.put(x, y, fireplace_stone_d),
                1 => c.put(x, y, fireplace_stone_m),
                _ => c.put(x, y, fireplace_stone_l),
            }
            if (x + y) % 5 == 0 {
                c.put(x, y, fireplace_stone_d);
            }
        }
    }
    c.fill_rect(fpx + 4, fpy + 3, fpw - 8, 4, [0, 0, 0, 255]);
    c.fill_rect(fpx + 5, fpy + 4, fpw - 10, 2, fire_outer);
    c.fill_rect(fpx + 7, fpy + 4, fpw - 14, 2, fire_inner);
    c.put(fpx + fpw / 2, fpy + 4, fire_core);
    c.put(fpx + fpw / 2 - 1, fpy + 5, fire_core);
    c.put(fpx + 5, fpy + 6, ember_dot);
    c.put(fpx + fpw - 6, fpy + 6, ember_dot);

    let sx = 8;
    let sy = 30;
    let sw = 18;
    let sh = 4;
    c.fill_rect(sx, sy, sw, sh, shelf);
    c.fill_rect(sx, sy, sw, 1, shelf_hi);
    c.fill_rect(sx + 2, sy - 3, 2, 3, jar_a);
    c.put(sx + 2, sy - 3, lantern_glow);
    c.fill_rect(sx + 5, sy - 3, 2, 3, jar_b);
    c.fill_rect(sx + 9, sy - 4, 2, 4, book_a);
    c.fill_rect(sx + 11, sy - 4, 2, 4, book_b);
    c.fill_rect(sx + 13, sy - 4, 2, 4, book_c);

    let krx = 62;
    let kry = 16;
    c.fill_rect(krx, kry, 12, 10, crate_dark);
    c.fill_rect(krx + 1, kry + 1, 10, 8, crate_main);
    c.fill_rect(krx + 1, kry + 1, 10, 1, crate_hi);
    c.fill_rect(krx + 1, kry + 5, 10, 1, crate_dark);
    c.fill_rect(krx + 5, kry + 1, 2, 8, crate_dark);

    c.put(mx, my, transparent);
    c.put(w - mx - 1, my, transparent);
    c.put(mx, h - my - 1, transparent);
    c.put(w - mx - 1, h - my - 1, transparent);

    c.into_image()
}

fn build_log_image() -> Image {
    let outline: Rgba = [6, 4, 2, 255];
    let wood_dark: Rgba = [32, 20, 8, 255];
    let wood_main: Rgba = [64, 40, 18, 255];
    let wood_light: Rgba = [96, 60, 26, 255];
    let ring: Rgba = [108, 72, 32, 255];
    let moss: Rgba = [34, 52, 22, 255];

    let mut c = Canvas::new(36, 12);
    c.fill_rect(1, 1, 34, 10, outline);
    c.fill_rect(2, 2, 32, 8, wood_main);
    c.fill_rect(2, 2, 32, 2, wood_light);
    c.fill_rect(2, 8, 32, 2, wood_dark);
    for x in 4..32 {
        if x % 6 == 0 {
            c.put(x, 5, wood_dark);
            c.put(x + 1, 6, wood_light);
        }
    }
    c.fill_circle(3, 5, 2, outline);
    c.fill_circle(3, 5, 1, ring);
    c.fill_circle(32, 6, 2, outline);
    c.fill_circle(32, 6, 1, ring);
    c.put(10, 3, moss);
    c.put(11, 4, moss);
    c.put(22, 8, moss);
    c.put(23, 9, moss);
    c.put(17, 5, [18, 32, 14, 255]);

    c.into_image()
}

fn build_stump_image() -> Image {
    let outline: Rgba = [4, 4, 4, 255];
    let bark: Rgba = [26, 16, 6, 255];
    let wood: Rgba = [66, 40, 16, 255];
    let wood_light: Rgba = [100, 64, 24, 255];
    let ring: Rgba = [130, 86, 34, 255];
    let moss: Rgba = [32, 50, 20, 255];

    let mut c = Canvas::new(16, 16);
    c.fill_circle(8, 8, 7, outline);
    c.fill_circle(8, 8, 6, bark);
    c.fill_circle(8, 8, 5, wood);
    c.fill_circle(8, 8, 3, wood_light);
    c.fill_circle(8, 8, 2, ring);
    c.put(8, 8, wood);
    c.put(7, 7, ring);
    c.put(5, 10, moss);
    c.put(10, 5, moss);
    c.put(11, 11, moss);

    c.into_image()
}

fn build_stone_image() -> Image {
    let outline: Rgba = [4, 4, 6, 255];
    let dark: Rgba = [30, 32, 38, 255];
    let main: Rgba = [58, 60, 66, 255];
    let light: Rgba = [92, 92, 98, 255];
    let moss: Rgba = [34, 52, 22, 255];

    let mut c = Canvas::new(20, 14);
    c.fill_circle(10, 7, 7, outline);
    c.fill_circle(10, 7, 6, main);
    c.fill_circle(9, 5, 3, light);
    c.put(8, 4, light);
    c.fill_rect(6, 10, 8, 1, dark);
    c.put(13, 9, dark);
    c.put(5, 9, dark);
    c.put(12, 3, moss);
    c.put(5, 6, moss);
    c.put(14, 7, moss);

    c.into_image()
}

fn build_bush_image() -> Image {
    let outline: Rgba = [6, 10, 6, 255];
    let dark: Rgba = [20, 38, 18, 255];
    let mid: Rgba = [34, 58, 26, 255];
    let light: Rgba = [52, 84, 38, 255];
    let dead: Rgba = [52, 44, 22, 255];

    let mut c = Canvas::new(18, 14);
    c.fill_circle(5, 7, 4, outline);
    c.fill_circle(5, 7, 3, dark);
    c.fill_circle(4, 6, 2, mid);

    c.fill_circle(12, 7, 5, outline);
    c.fill_circle(12, 7, 4, dark);
    c.fill_circle(11, 6, 2, mid);
    c.put(10, 5, light);
    c.put(13, 6, light);

    c.put(8, 9, dead);
    c.put(15, 9, dead);
    c.put(3, 10, dead);

    c.into_image()
}

fn build_fern_image() -> Image {
    let outline: Rgba = [6, 12, 6, 255];
    let dark: Rgba = [22, 42, 22, 255];
    let mid: Rgba = [38, 66, 32, 255];
    let light: Rgba = [58, 92, 42, 255];

    let mut c = Canvas::new(16, 16);
    for (x, y) in [
        (8, 13), (8, 12), (8, 11), (8, 10), (8, 9), (8, 8), (8, 7), (8, 6),
    ] {
        c.put(x, y, outline);
    }
    for (x, y) in [
        (6, 12), (5, 11), (4, 10),
        (10, 12), (11, 11), (12, 10),
        (6, 9), (5, 8), (4, 7),
        (10, 9), (11, 8), (12, 7),
        (7, 6), (9, 6),
        (7, 4), (9, 4),
        (8, 3),
    ] {
        c.put(x, y, dark);
    }
    for (x, y) in [
        (5, 11), (11, 11), (5, 8), (11, 8), (7, 5), (9, 5), (8, 4),
    ] {
        c.put(x, y, mid);
    }
    for (x, y) in [(4, 10), (12, 10), (8, 3)] {
        c.put(x, y, light);
    }

    c.into_image()
}

fn build_wrecked_car_image() -> Image {
    let outline: Rgba = [6, 6, 8, 255];
    let body_dark: Rgba = [12, 12, 14, 255];
    let body_main: Rgba = [30, 28, 28, 255];
    let body_rust: Rgba = [62, 32, 14, 255];
    let body_rust_dark: Rgba = [38, 18, 6, 255];
    let window_shatter: Rgba = [28, 32, 38, 255];
    let tire: Rgba = [14, 12, 12, 255];
    let headlight: Rgba = [62, 60, 50, 255];
    let char: Rgba = [8, 8, 8, 255];
    let soot: Rgba = [18, 16, 16, 255];

    let mut c = Canvas::new(32, 14);

    c.fill_rect(1, 1, 30, 12, outline);
    c.fill_rect(2, 2, 28, 10, body_main);
    c.fill_rect(2, 2, 28, 1, body_dark);
    c.fill_rect(2, 11, 28, 1, body_dark);

    for (x, y) in [(5, 3), (9, 4), (13, 3), (18, 4), (22, 3), (26, 4)] {
        c.put(x, y, body_rust);
        c.put(x, y + 1, body_rust_dark);
    }
    for (x, y) in [(6, 10), (12, 10), (18, 10), (24, 10)] {
        c.put(x, y, body_rust);
    }

    c.fill_rect(6, 3, 4, 8, outline);
    c.fill_rect(7, 4, 2, 6, window_shatter);
    c.put(7, 5, char);
    c.put(8, 8, char);

    c.fill_rect(18, 3, 4, 8, outline);
    c.fill_rect(19, 4, 2, 6, window_shatter);
    c.put(19, 4, char);
    c.put(20, 7, char);

    c.fill_rect(10, 4, 8, 6, outline);
    c.fill_rect(11, 5, 6, 4, body_dark);
    c.put(12, 6, char);
    c.put(14, 7, char);
    c.put(16, 6, char);

    c.fill_rect(5, 0, 3, 3, outline);
    c.fill_rect(5, 11, 3, 3, outline);
    c.fill_rect(24, 0, 3, 3, outline);
    c.fill_rect(24, 11, 3, 3, outline);
    c.put(6, 1, tire);
    c.put(6, 12, tire);
    c.put(25, 1, tire);
    c.put(25, 12, tire);

    c.put(30, 4, headlight);
    c.put(30, 9, headlight);

    for (x, y) in [(4, 7), (14, 2), (22, 11), (28, 7)] {
        c.put(x, y, soot);
    }

    c.into_image()
}

fn build_house_image() -> Image {
    let transparent: Rgba = [0, 0, 0, 0];
    let outline: Rgba = [6, 4, 6, 255];
    let tile_dark: Rgba = [20, 28, 50, 255];
    let tile_main: Rgba = [44, 58, 96, 255];
    let tile_hi: Rgba = [72, 92, 138, 255];
    let tile_shine: Rgba = [108, 128, 178, 255];
    let ridge_dark: Rgba = [18, 22, 38, 255];
    let ridge_main: Rgba = [32, 42, 72, 255];
    let moss: Rgba = [34, 58, 24, 255];
    let chimney_out: Rgba = [10, 8, 10, 255];
    let chimney_brick_d: Rgba = [54, 22, 12, 255];
    let chimney_brick: Rgba = [102, 46, 28, 255];
    let chimney_brick_hi: Rgba = [146, 70, 42, 255];
    let chimney_top: Rgba = [30, 24, 22, 255];
    let smoke_a: Rgba = [76, 72, 78, 180];
    let smoke_b: Rgba = [52, 48, 54, 130];
    let dormer_out: Rgba = [10, 12, 20, 255];
    let dormer_frame: Rgba = [28, 32, 54, 255];
    let dormer_glass: Rgba = [140, 172, 196, 255];
    let dormer_shine: Rgba = [220, 232, 246, 255];
    let porch: Rgba = [26, 16, 6, 255];
    let step: Rgba = [60, 38, 16, 255];

    let w = 104;
    let h = 80;
    let mut c = Canvas::new(w, h);

    let mx = 4;
    let my = 4;
    c.fill_rect(mx, my, w - mx * 2, h - my * 2, outline);
    c.fill_rect(mx + 1, my + 1, w - mx * 2 - 2, h - my * 2 - 2, tile_main);

    for y in (my + 1)..(h - my - 1) {
        let row = (y - my - 1) / 3;
        let phase = if row % 2 == 0 { 0 } else { 3 };
        for x in (mx + 1)..(w - mx - 1) {
            let tile_x = (x - mx - 1 + phase) % 6;
            let tile_y = (y - my - 1) % 3;
            if tile_y == 0 || tile_x == 0 || tile_x == 5 {
                c.put(x, y, tile_dark);
            } else if tile_y == 1 && (tile_x == 2 || tile_x == 3) {
                c.put(x, y, tile_hi);
            }
        }
    }

    let ridge_y = h / 2;
    c.fill_rect(mx + 2, ridge_y - 1, w - mx * 2 - 4, 1, ridge_main);
    c.fill_rect(mx + 2, ridge_y, w - mx * 2 - 4, 1, ridge_dark);
    for x in (mx + 3)..(w - mx - 3) {
        if x % 4 == 0 {
            c.put(x, ridge_y - 1, tile_shine);
        }
    }

    for (mxp, myp) in [(14, 12), (82, 16), (20, 60), (76, 56), (48, 10)] {
        c.fill_rect(mxp, myp, 3, 2, moss);
        c.put(mxp + 1, myp + 1, ridge_dark);
    }

    let cx = 16;
    let cy = 10;
    c.fill_rect(cx, cy, 9, 12, chimney_out);
    c.fill_rect(cx + 1, cy + 1, 7, 10, chimney_brick);
    c.fill_rect(cx + 1, cy + 1, 7, 2, chimney_top);
    for by in 0..3 {
        let y = cy + 3 + by * 3;
        c.fill_rect(cx + 1, y, 7, 1, chimney_brick_d);
    }
    c.fill_rect(cx + 6, cy + 3, 2, 6, chimney_brick_hi);
    c.put(cx + 3, cy - 2, smoke_a);
    c.put(cx + 4, cy - 2, smoke_a);
    c.put(cx + 3, cy - 4, smoke_b);
    c.put(cx + 5, cy - 4, smoke_b);

    let dx = w / 2 - 8;
    let dy = my + 14;
    c.fill_rect(dx, dy, 16, 12, dormer_out);
    c.fill_rect(dx + 1, dy + 1, 14, 10, dormer_frame);
    c.fill_rect(dx + 2, dy + 2, 12, 8, dormer_glass);
    c.fill_rect(dx + 7, dy + 2, 2, 8, dormer_frame);
    c.fill_rect(dx + 2, dy + 5, 12, 2, dormer_frame);
    c.put(dx + 3, dy + 3, dormer_shine);
    c.put(dx + 4, dy + 3, dormer_shine);
    c.put(dx + 10, dy + 8, dormer_shine);

    let porch_margin = 18;
    c.fill_rect(porch_margin, h - my - 3, w - porch_margin * 2, 2, porch);
    c.fill_rect(porch_margin + 6, h - my - 1, 8, 1, step);
    c.fill_rect(w - porch_margin - 14, h - my - 1, 8, 1, step);

    c.put(mx, my, transparent);
    c.put(w - mx - 1, my, transparent);
    c.put(mx, h - my - 1, transparent);
    c.put(w - mx - 1, h - my - 1, transparent);

    c.into_image()
}

fn build_store_image() -> Image {
    let transparent: Rgba = [0, 0, 0, 0];
    let outline: Rgba = [8, 6, 4, 255];
    let roof_flat: Rgba = [48, 46, 44, 255];
    let roof_edge: Rgba = [22, 20, 18, 255];
    let roof_patch: Rgba = [36, 34, 32, 255];
    let roof_tar: Rgba = [14, 12, 12, 255];
    let sign_red: Rgba = [164, 28, 22, 255];
    let sign_white: Rgba = [232, 230, 222, 255];
    let sign_frame: Rgba = [24, 20, 14, 255];
    let vent_frame: Rgba = [18, 16, 14, 255];
    let vent_main: Rgba = [108, 110, 116, 255];
    let vent_hi: Rgba = [168, 172, 180, 255];
    let vent_dark: Rgba = [36, 36, 40, 255];

    let w = 120;
    let h = 72;
    let mut c = Canvas::new(w, h);

    let mx = 4;
    let my = 4;
    c.fill_rect(mx, my, w - mx * 2, h - my * 2, outline);
    c.fill_rect(mx + 1, my + 1, w - mx * 2 - 2, h - my * 2 - 2, roof_flat);

    for y in (my + 2)..(h - my - 2) {
        for x in (mx + 2)..(w - mx - 2) {
            let r = (x * 7 + y * 5) % 29;
            if r < 3 {
                c.put(x, y, roof_patch);
            } else if r == 5 {
                c.put(x, y, roof_tar);
            }
        }
    }

    c.fill_rect(mx + 1, my + 1, w - mx * 2 - 2, 2, roof_edge);
    c.fill_rect(mx + 1, h - my - 3, w - mx * 2 - 2, 2, roof_edge);
    c.fill_rect(mx + 1, my + 1, 2, h - my * 2 - 2, roof_edge);
    c.fill_rect(w - mx - 3, my + 1, 2, h - my * 2 - 2, roof_edge);

    let sx = w / 2 - 26;
    let sy = h / 2 - 10;
    c.fill_rect(sx, sy, 52, 20, sign_frame);
    c.fill_rect(sx + 2, sy + 2, 48, 16, sign_red);
    for i in 0..5 {
        let lx = sx + 6 + i * 9;
        c.fill_rect(lx, sy + 6, 6, 2, sign_white);
        c.fill_rect(lx + 2, sy + 8, 2, 4, sign_white);
    }
    c.fill_rect(sx + 2, sy + 2, 48, 1, sign_white);
    c.fill_rect(sx + 2, sy + 17, 48, 1, sign_frame);

    let vx = mx + 10;
    let vy = my + 8;
    c.fill_rect(vx, vy, 14, 12, vent_frame);
    c.fill_rect(vx + 1, vy + 1, 12, 10, vent_main);
    for row in 0..3 {
        let y = vy + 3 + row * 3;
        c.fill_rect(vx + 2, y, 10, 1, vent_dark);
    }
    c.fill_rect(vx + 1, vy + 1, 12, 1, vent_hi);

    let v2x = w - mx - 16;
    let v2y = h - my - 14;
    c.fill_rect(v2x, v2y, 10, 8, vent_frame);
    c.fill_rect(v2x + 1, v2y + 1, 8, 6, vent_main);
    c.fill_rect(v2x + 2, v2y + 2, 6, 1, vent_dark);
    c.fill_rect(v2x + 2, v2y + 4, 6, 1, vent_dark);
    c.put(v2x + 1, v2y + 1, vent_hi);

    c.put(mx, my, transparent);
    c.put(w - mx - 1, my, transparent);
    c.put(mx, h - my - 1, transparent);
    c.put(w - mx - 1, h - my - 1, transparent);

    c.into_image()
}

fn build_house_interior_image() -> Image {
    let transparent: Rgba = [0, 0, 0, 0];
    let wall_out: Rgba = [6, 4, 2, 255];
    let wall_main: Rgba = [52, 38, 24, 255];
    let wall_shadow: Rgba = [26, 18, 10, 255];
    let wall_hi: Rgba = [96, 72, 44, 255];
    let floor_dark: Rgba = [38, 26, 14, 255];
    let floor_main: Rgba = [72, 50, 26, 255];
    let floor_hi: Rgba = [112, 80, 40, 255];
    let plank_gap: Rgba = [18, 10, 4, 255];

    let rug_a: Rgba = [60, 100, 140, 255];
    let rug_b: Rgba = [40, 70, 100, 255];
    let rug_trim: Rgba = [220, 200, 140, 255];

    let couch_frame: Rgba = [40, 24, 12, 255];
    let couch_main: Rgba = [140, 60, 40, 255];
    let couch_hi: Rgba = [200, 100, 72, 255];
    let couch_shadow: Rgba = [80, 32, 20, 255];
    let cushion: Rgba = [220, 180, 140, 255];

    let ct_dark: Rgba = [28, 18, 8, 255];
    let ct_main: Rgba = [86, 54, 22, 255];
    let ct_hi: Rgba = [140, 92, 36, 255];

    let kitchen_dark: Rgba = [40, 42, 48, 255];
    let kitchen_main: Rgba = [96, 100, 112, 255];
    let kitchen_hi: Rgba = [156, 160, 172, 255];
    let stove_knob: Rgba = [220, 60, 40, 255];
    let sink: Rgba = [180, 200, 220, 255];
    let sink_dark: Rgba = [80, 112, 136, 255];
    let fridge: Rgba = [230, 230, 236, 255];
    let fridge_trim: Rgba = [160, 162, 168, 255];

    let shelf: Rgba = [54, 32, 14, 255];
    let shelf_hi: Rgba = [108, 66, 30, 255];
    let book_a: Rgba = [138, 34, 30, 255];
    let book_b: Rgba = [30, 74, 142, 255];
    let book_c: Rgba = [60, 110, 70, 255];
    let book_d: Rgba = [210, 180, 40, 255];
    let lamp_glow: Rgba = [255, 210, 110, 255];

    let w = 104;
    let h = 80;
    let mut c = Canvas::new(w, h);

    let mx = 4;
    let my = 4;
    c.fill_rect(mx, my, w - mx * 2, h - my * 2, wall_out);
    c.fill_rect(mx + 1, my + 1, w - mx * 2 - 2, h - my * 2 - 2, wall_main);
    c.fill_rect(mx + 2, my + 2, w - mx * 2 - 4, h - my * 2 - 4, wall_shadow);

    let fx0 = mx + 6;
    let fy0 = my + 6;
    let fw = w - mx * 2 - 12;
    let fh = h - my * 2 - 12;
    c.fill_rect(fx0, fy0, fw, fh, floor_main);
    for y in fy0..(fy0 + fh) {
        if (y - fy0) % 5 == 0 {
            c.fill_rect(fx0, y, fw, 1, plank_gap);
        } else if (y - fy0) % 5 == 1 {
            for x in fx0..(fx0 + fw) {
                if (x * 3 + y * 2) % 13 == 0 {
                    c.put(x, y, floor_hi);
                }
                if (x * 5 + y) % 17 == 0 {
                    c.put(x, y, floor_dark);
                }
            }
        }
    }

    c.fill_rect(mx + 1, my + 1, w - mx * 2 - 2, 1, wall_hi);
    c.fill_rect(mx + 1, my + 1, 1, h - my * 2 - 2, wall_hi);

    let rx = 34;
    let ry = 38;
    let rw = 36;
    let rh = 22;
    c.fill_rect(rx - 1, ry - 1, rw + 2, rh + 2, rug_trim);
    c.fill_rect(rx, ry, rw, rh, rug_a);
    for y in ry..(ry + rh) {
        for x in rx..(rx + rw) {
            if (x + y) % 2 == 0 && ((x * 3 + y) % 7) < 3 {
                c.put(x, y, rug_b);
            }
        }
    }
    for x in rx..(rx + rw) {
        c.put(x, ry, rug_trim);
        c.put(x, ry + rh - 1, rug_trim);
    }
    for y in ry..(ry + rh) {
        c.put(rx, y, rug_trim);
        c.put(rx + rw - 1, y, rug_trim);
    }

    let sx = 10;
    let sy = 32;
    let sw = 18;
    let sh = 30;
    c.fill_rect(sx, sy, sw, sh, couch_frame);
    c.fill_rect(sx + 1, sy + 1, sw - 2, sh - 2, couch_main);
    c.fill_rect(sx + 1, sy + 1, sw - 2, 2, couch_hi);
    c.fill_rect(sx + 1, sy + sh - 3, sw - 2, 2, couch_shadow);
    for i in 0..3 {
        let cy = sy + 4 + i * 8;
        c.fill_rect(sx + 3, cy, sw - 6, 5, cushion);
        c.fill_rect(sx + 3, cy, sw - 6, 1, couch_hi);
    }

    let tx = 36;
    let ty = 44;
    let tw = 24;
    let th = 12;
    c.fill_rect(tx, ty, tw, th, ct_dark);
    c.fill_rect(tx + 1, ty + 1, tw - 2, th - 2, ct_main);
    c.fill_rect(tx + 1, ty + 1, tw - 2, 1, ct_hi);
    c.put(tx + 6, ty + 5, [240, 240, 240, 255]);
    c.put(tx + 7, ty + 5, [240, 240, 240, 255]);
    c.put(tx + 6, ty + 6, [240, 240, 240, 255]);
    c.fill_rect(tx + 13, ty + 4, 4, 2, book_a);
    c.fill_rect(tx + 14, ty + 5, 4, 2, book_b);

    let kx = w - 34;
    let ky = 10;
    c.fill_rect(kx, ky, 26, 14, wall_out);
    c.fill_rect(kx + 1, ky + 1, 24, 12, kitchen_main);
    c.fill_rect(kx + 1, ky + 1, 24, 1, kitchen_hi);
    c.fill_rect(kx + 2, ky + 4, 8, 8, kitchen_dark);
    c.put(kx + 4, ky + 6, stove_knob);
    c.put(kx + 7, ky + 6, stove_knob);
    c.put(kx + 4, ky + 9, kitchen_hi);
    c.put(kx + 7, ky + 9, kitchen_hi);
    c.fill_rect(kx + 12, ky + 4, 8, 8, sink_dark);
    c.fill_rect(kx + 13, ky + 5, 6, 6, sink);
    c.put(kx + 16, ky + 7, kitchen_dark);
    let fxk = kx + 20;
    c.fill_rect(fxk, ky - 2, 6, 18, wall_out);
    c.fill_rect(fxk + 1, ky - 1, 4, 16, fridge);
    c.fill_rect(fxk + 1, ky + 5, 4, 1, fridge_trim);
    c.put(fxk + 4, ky + 2, fridge_trim);
    c.put(fxk + 4, ky + 8, fridge_trim);

    let shx = w - 28;
    let shy = h - 22;
    c.fill_rect(shx, shy, 20, 14, wall_out);
    c.fill_rect(shx + 1, shy + 1, 18, 12, shelf);
    c.fill_rect(shx + 1, shy + 1, 18, 1, shelf_hi);
    c.fill_rect(shx + 1, shy + 5, 18, 1, shelf_hi);
    c.fill_rect(shx + 1, shy + 9, 18, 1, shelf_hi);
    let books = [book_a, book_b, book_c, book_d, book_a, book_b];
    for (i, col) in books.iter().enumerate() {
        let bx = shx + 2 + (i as i32) * 3;
        c.fill_rect(bx, shy + 2, 2, 2, *col);
        c.fill_rect(bx, shy + 6, 2, 2, *col);
    }

    c.put(shx + 17, shy + 10, lamp_glow);
    c.put(shx + 17, shy + 11, lamp_glow);

    c.put(mx, my, transparent);
    c.put(w - mx - 1, my, transparent);
    c.put(mx, h - my - 1, transparent);
    c.put(w - mx - 1, h - my - 1, transparent);

    c.into_image()
}

fn build_store_interior_image() -> Image {
    let transparent: Rgba = [0, 0, 0, 0];
    let wall_out: Rgba = [6, 4, 2, 255];
    let wall_main: Rgba = [82, 40, 20, 255];
    let wall_shadow: Rgba = [40, 20, 10, 255];
    let wall_hi: Rgba = [140, 72, 36, 255];
    let floor_dark: Rgba = [60, 56, 50, 255];
    let floor_main: Rgba = [108, 102, 92, 255];
    let floor_hi: Rgba = [156, 150, 138, 255];
    let tile_gap: Rgba = [36, 32, 28, 255];

    let counter_dark: Rgba = [24, 16, 8, 255];
    let counter_main: Rgba = [82, 52, 24, 255];
    let counter_hi: Rgba = [140, 90, 40, 255];
    let counter_top: Rgba = [168, 110, 50, 255];

    let shelf_dark: Rgba = [18, 14, 10, 255];
    let shelf_main: Rgba = [56, 42, 22, 255];
    let shelf_hi: Rgba = [102, 74, 34, 255];

    let can_a: Rgba = [200, 60, 40, 255];
    let can_b: Rgba = [60, 120, 180, 255];
    let can_c: Rgba = [220, 180, 40, 255];
    let can_d: Rgba = [60, 160, 80, 255];
    let can_e: Rgba = [220, 120, 60, 255];
    let can_top: Rgba = [200, 200, 210, 255];

    let barrel_dark: Rgba = [28, 18, 8, 255];
    let barrel_main: Rgba = [92, 56, 24, 255];
    let barrel_hi: Rgba = [140, 88, 36, 255];
    let barrel_hoop: Rgba = [60, 50, 40, 255];

    let register_dark: Rgba = [20, 20, 24, 255];
    let register_main: Rgba = [180, 180, 190, 255];
    let register_key: Rgba = [60, 62, 70, 255];
    let register_screen: Rgba = [80, 180, 100, 255];

    let poster_a: Rgba = [200, 30, 40, 255];
    let poster_b: Rgba = [60, 80, 160, 255];

    let w = 120;
    let h = 72;
    let mut c = Canvas::new(w, h);

    let mx = 4;
    let my = 4;
    c.fill_rect(mx, my, w - mx * 2, h - my * 2, wall_out);
    c.fill_rect(mx + 1, my + 1, w - mx * 2 - 2, h - my * 2 - 2, wall_main);
    c.fill_rect(mx + 2, my + 2, w - mx * 2 - 4, h - my * 2 - 4, wall_shadow);

    let fx0 = mx + 6;
    let fy0 = my + 6;
    let fw = w - mx * 2 - 12;
    let fh = h - my * 2 - 12;
    c.fill_rect(fx0, fy0, fw, fh, floor_main);
    for y in fy0..(fy0 + fh) {
        for x in fx0..(fx0 + fw) {
            if (x - fx0) % 8 == 0 || (y - fy0) % 6 == 0 {
                c.put(x, y, tile_gap);
            }
            if ((x - fx0) % 8 == 2 && (y - fy0) % 6 == 2)
                || ((x - fx0) % 8 == 5 && (y - fy0) % 6 == 4)
            {
                c.put(x, y, floor_hi);
            }
            if (x - fx0) % 8 == 6 && (y - fy0) % 6 == 1 {
                c.put(x, y, floor_dark);
            }
        }
    }

    c.fill_rect(mx + 1, my + 1, w - mx * 2 - 2, 1, wall_hi);
    c.fill_rect(mx + 1, my + 1, 1, h - my * 2 - 2, wall_hi);

    c.fill_rect(16, my + 2, 8, 3, poster_a);
    c.put(17, my + 3, can_top);
    c.fill_rect(w - 24, my + 2, 8, 3, poster_b);
    c.put(w - 22, my + 3, can_top);

    let shelf_row_y = 14;
    let shelves: [(i32, i32); 3] = [
        (fx0 + 2, shelf_row_y),
        (fx0 + 38, shelf_row_y),
        (fx0 + 74, shelf_row_y),
    ];
    for (shx, shy) in shelves {
        let sw = 22;
        let sh = 9;
        c.fill_rect(shx, shy, sw, sh, shelf_dark);
        c.fill_rect(shx + 1, shy + 1, sw - 2, sh - 2, shelf_main);
        c.fill_rect(shx + 1, shy + 1, sw - 2, 1, shelf_hi);
        c.fill_rect(shx + 1, shy + 4, sw - 2, 1, shelf_hi);
        let can_colors = [can_a, can_b, can_c, can_d, can_e];
        for row in 0..2i32 {
            for i in 0..6i32 {
                let cx = shx + 2 + i * 3;
                let cy = shy + 1 + row * 4;
                let idx = ((i as usize) + (row as usize) * 3) % can_colors.len();
                let col = can_colors[idx];
                c.fill_rect(cx, cy, 2, 3, col);
                c.put(cx, cy, can_top);
                c.put(cx + 1, cy, can_top);
            }
        }
    }

    let cx0 = 30;
    let cy0 = 42;
    let cwidth = w - 60;
    let cheight = 8;
    c.fill_rect(cx0, cy0, cwidth, cheight, counter_dark);
    c.fill_rect(cx0 + 1, cy0 + 1, cwidth - 2, cheight - 2, counter_main);
    c.fill_rect(cx0 + 1, cy0 + 1, cwidth - 2, 1, counter_top);
    c.fill_rect(cx0 + 1, cy0 + 2, cwidth - 2, 1, counter_hi);
    let panels = (cwidth - 4) / 8;
    for i in 0..panels {
        let px = cx0 + 3 + i * 8;
        c.fill_rect(px, cy0 + 4, 6, 3, counter_dark);
    }

    let rgx = cx0 + cwidth / 2 - 6;
    let rgy = cy0 - 8;
    c.fill_rect(rgx, rgy, 12, 8, register_dark);
    c.fill_rect(rgx + 1, rgy + 1, 10, 6, register_main);
    c.fill_rect(rgx + 2, rgy + 2, 8, 2, register_screen);
    for row in 0..2 {
        for col_i in 0..4 {
            let kx = rgx + 2 + col_i * 2;
            let ky = rgy + 5 + row;
            c.put(kx, ky, register_key);
        }
    }

    for (bxb, byb) in [(fx0 + 4, h - 18), (w - 16, h - 18)] {
        c.fill_circle(bxb, byb, 5, barrel_dark);
        c.fill_circle(bxb, byb, 4, barrel_main);
        c.fill_circle(bxb - 1, byb - 1, 2, barrel_hi);
        c.fill_rect(bxb - 4, byb - 1, 8, 1, barrel_hoop);
        c.fill_rect(bxb - 4, byb + 2, 8, 1, barrel_hoop);
    }

    c.put(mx, my, transparent);
    c.put(w - mx - 1, my, transparent);
    c.put(mx, h - my - 1, transparent);
    c.put(w - mx - 1, h - my - 1, transparent);

    c.into_image()
}

fn build_ground_tile_image(variant: u8) -> Image {
    let mut c = Canvas::new(16, 16);

    let bases: [[u8; 4]; 6] = [
        [20, 28, 16, 255],
        [24, 22, 14, 255],
        [18, 26, 18, 255],
        [22, 20, 12, 255],
        [16, 24, 14, 255],
        [20, 22, 16, 255],
    ];
    let base = bases[(variant % 6) as usize];
    c.fill_rect(0, 0, 16, 16, base);

    let dark: Rgba = [
        base[0].saturating_sub(6),
        base[1].saturating_sub(8),
        base[2].saturating_sub(6),
        255,
    ];
    let light: Rgba = [base[0] + 10, base[1] + 14, base[2] + 8, 255];
    let root: Rgba = [32, 24, 14, 255];
    let moss: Rgba = [24, 38, 18, 255];
    let stone: Rgba = [32, 32, 36, 255];

    for y in 0..16 {
        for x in 0..16 {
            let hash = ((x * 7 + y * 13 + variant as i32 * 31) & 0xFF) as u8;
            match hash % 17 {
                0 | 1 => { c.put(x, y, dark); }
                2 => { c.put(x, y, light); }
                3 if variant.is_multiple_of(2) => { c.put(x, y, moss); }
                4 if variant.is_multiple_of(3) => { c.put(x, y, root); }
                5 if variant == 3 || variant == 5 => { c.put(x, y, stone); }
                _ => {}
            }
        }
    }

    match variant % 6 {
        0 => {
            c.fill_rect(3, 7, 10, 1, root);
            c.put(4, 6, root);
            c.put(11, 8, root);
        }
        1 => {
            c.fill_rect(5, 5, 4, 3, moss);
            c.put(6, 4, moss);
        }
        2 => {
            c.put(4, 4, stone);
            c.put(5, 4, stone);
            c.put(10, 11, stone);
            c.put(11, 11, stone);
            c.put(11, 10, stone);
        }
        3 => {
            let dirt: Rgba = [28, 22, 12, 255];
            c.fill_rect(6, 6, 5, 4, dirt);
            c.put(7, 5, dirt);
            c.put(9, 10, dirt);
        }
        4 => {
            let leaf_a: Rgba = [36, 26, 12, 255];
            let leaf_b: Rgba = [30, 22, 10, 255];
            for &(lx, ly) in &[(2, 3), (5, 8), (9, 2), (12, 10), (7, 13), (14, 5)] {
                c.put(lx, ly, leaf_a);
                c.put(lx + 1, ly, leaf_b);
            }
        }
        _ => {
            c.fill_rect(2, 10, 6, 1, root);
            c.put(3, 9, root);
            c.fill_rect(9, 3, 3, 2, moss);
        }
    }

    c.into_image()
}

fn build_grass_image() -> Image {
    let blade_dark: Rgba = [14, 32, 10, 255];
    let blade_main: Rgba = [28, 58, 18, 255];
    let blade_hi: Rgba = [52, 96, 30, 255];

    let w = 10;
    let h = 8;
    let mut c = Canvas::new(w, h);
    for y in 2..7 {
        c.put(2, y, blade_dark);
    }
    for y in 0..7 {
        c.put(5, y, blade_dark);
    }
    for y in 3..7 {
        c.put(8, y, blade_dark);
    }
    c.put(2, 3, blade_main);
    c.put(2, 5, blade_main);
    c.put(5, 2, blade_main);
    c.put(5, 4, blade_main);
    c.put(5, 0, blade_hi);
    c.put(5, 1, blade_hi);
    c.put(8, 4, blade_main);
    c.put(8, 6, blade_hi);
    c.into_image()
}

fn build_underground_tile_image(variant: i32) -> Image {
    let base: Rgba = match variant % 4 {
        0 => [42, 42, 46, 255],
        1 => [38, 38, 42, 255],
        2 => [44, 44, 48, 255],
        _ => [36, 36, 40, 255],
    };
    let crack: Rgba = [24, 24, 28, 255];
    let stain: Rgba = [32, 30, 28, 255];
    let highlight: Rgba = [52, 52, 56, 255];
    let mut c = Canvas::new(16, 16);
    c.fill_rect(0, 0, 16, 16, base);
    for y in 0..16 {
        for x in 0..16 {
            let hash = ((x * 11 + y * 7 + variant * 23) & 0xFF) as u8;
            if hash.is_multiple_of(19) {
                c.put(x, y, highlight);
            } else if hash.is_multiple_of(23) {
                c.put(x, y, stain);
            }
        }
    }
    match variant % 4 {
        0 => {
            c.put(3, 7, crack); c.put(4, 8, crack); c.put(5, 8, crack);
            c.put(12, 3, stain);
        }
        1 => {
            c.put(8, 4, crack); c.put(9, 5, crack);
            c.put(2, 12, stain); c.put(3, 12, stain);
        }
        2 => {
            c.put(6, 10, crack); c.put(7, 11, crack);
            c.put(13, 7, stain);
        }
        _ => {
            c.put(4, 2, crack); c.put(10, 14, stain);
            c.put(11, 14, stain); c.put(11, 13, crack);
        }
    }
    c.into_image()
}

fn build_pillar_image() -> Image {
    let outline: Rgba = [22, 22, 26, 255];
    let concrete: Rgba = [90, 90, 95, 255];
    let concrete_light: Rgba = [110, 110, 115, 255];
    let concrete_dark: Rgba = [60, 60, 65, 255];
    let crack: Rgba = [40, 40, 44, 255];
    let mut c = Canvas::new(12, 12);
    c.fill_circle(6, 6, 6, outline);
    c.fill_circle(6, 6, 5, concrete);
    c.fill_circle(5, 5, 3, concrete_light);
    c.put(7, 7, concrete_dark);
    c.put(8, 8, concrete_dark);
    c.put(4, 8, crack);
    c.put(3, 7, crack);
    c.into_image()
}


use std::sync::OnceLock;

use bevy::prelude::*;

use crate::map_data::{
    Building, BuildingType, GateKind, Prop, PropKind, RoofStyle, Theme,
    BUILDINGS, GATES, PROPS, SEGMENTS,
};
use crate::net::NetContext;
use crate::pixelart::{Canvas, Rgba};
use crate::player::Player;
use crate::settings::GraphicsSettings;
use crate::{gameplay_active, GameState};

// ════════════════════════════════════════════════════════════════════════
//  World layout — 5 themed segments in a horizontal strip.
//  240×48 tiles (7680×1536 px).  Each segment 48×48, road grid at rows
//  22-25 and cols 22-25 (local).  Player starts in segment 1 SW.
// ════════════════════════════════════════════════════════════════════════

pub const TILE_SIZE: f32 = 32.0;
pub const MAP_COLS: i32 = 240;
pub const MAP_ROWS: i32 = 48;
pub const MAP_WIDTH: f32 = MAP_COLS as f32 * TILE_SIZE; // 7680
pub const MAP_HEIGHT: f32 = MAP_ROWS as f32 * TILE_SIZE; // 1536

pub const SEG_TILES: i32 = 48;
pub const SEG_WIDTH: f32 = SEG_TILES as f32 * TILE_SIZE; // 1536
pub const ROAD_H_TOP: i32 = 22;
pub const ROAD_H_BOT: i32 = 25;
pub const ROAD_V_LEFT: i32 = 22;
pub const ROAD_V_RIGHT: i32 = 25;

pub const WALL_THICK: f32 = 16.0;
pub const BUILDING_WALL_THICK: f32 = 12.0;
pub const INTERNAL_WALL_THICK: f32 = 8.0;

/// Player spawn at segment 1 local tile (4, 32).
pub const PLAYER_SPAWN_X: f32 = -MAP_WIDTH * 0.5 + 4.5 * TILE_SIZE;
pub const PLAYER_SPAWN_Y: f32 = -MAP_HEIGHT * 0.5 + 32.5 * TILE_SIZE;

// (Legacy multi-floor / zone / barrier compat shims removed — the metro
// map is single-storey, all those constants were placeholders for code
// paths that no longer exist.)

// ════════════════════════════════════════════════════════════════════════
//  Wall side / spawn points
// ════════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum WallSide {
    N,
    S,
    E,
    W,
}

pub struct SpawnPointSpec {
    /// Display name for debug/dev tooling — not read at runtime, kept so
    /// the `SPAWN_POINTS` table reads as documentation.
    #[allow(dead_code)]
    pub label: &'static str,
    pub tile: (i32, i32),
    pub span_tiles: i32,
    pub side: WallSide,
    pub interior_only: bool,
    /// 1-5; zombies only spawn from points whose segment is unlocked.
    pub segment_idx: u8,
}

/// Two perimeter spawn points per segment (north + south edges) plus the
/// far west/east map edges.  Filtering by `segment_idx` against the
/// unlock state keeps zombies from emerging in fogged-out areas.
pub const SPAWN_POINTS: &[SpawnPointSpec] = &[
    SpawnPointSpec { label: "SEG1 W ROAD",   tile: (0,   23), span_tiles: 4, side: WallSide::W, interior_only: false, segment_idx: 1 },
    SpawnPointSpec { label: "SEG1 N",        tile: (24,  47), span_tiles: 3, side: WallSide::N, interior_only: false, segment_idx: 1 },
    SpawnPointSpec { label: "SEG1 S",        tile: (24,  0),  span_tiles: 3, side: WallSide::S, interior_only: false, segment_idx: 1 },
    SpawnPointSpec { label: "SEG2 N",        tile: (72,  47), span_tiles: 3, side: WallSide::N, interior_only: false, segment_idx: 2 },
    SpawnPointSpec { label: "SEG2 S",        tile: (72,  0),  span_tiles: 3, side: WallSide::S, interior_only: false, segment_idx: 2 },
    SpawnPointSpec { label: "SEG3 N",        tile: (120, 47), span_tiles: 3, side: WallSide::N, interior_only: false, segment_idx: 3 },
    SpawnPointSpec { label: "SEG3 S",        tile: (120, 0),  span_tiles: 3, side: WallSide::S, interior_only: false, segment_idx: 3 },
    SpawnPointSpec { label: "SEG4 N",        tile: (168, 47), span_tiles: 3, side: WallSide::N, interior_only: false, segment_idx: 4 },
    SpawnPointSpec { label: "SEG4 S",        tile: (168, 0),  span_tiles: 3, side: WallSide::S, interior_only: false, segment_idx: 4 },
    SpawnPointSpec { label: "SEG5 N",        tile: (216, 47), span_tiles: 3, side: WallSide::N, interior_only: false, segment_idx: 5 },
    SpawnPointSpec { label: "SEG5 S",        tile: (216, 0),  span_tiles: 3, side: WallSide::S, interior_only: false, segment_idx: 5 },
    SpawnPointSpec { label: "SEG5 E ROAD",   tile: (239, 23), span_tiles: 4, side: WallSide::E, interior_only: false, segment_idx: 5 },
];

pub fn spawn_point_world(spec: &SpawnPointSpec) -> Vec2 {
    tile_center(spec.tile.0, spec.tile.1)
}

// ════════════════════════════════════════════════════════════════════════
//  Obstacles + nav
// ════════════════════════════════════════════════════════════════════════

// Obstacles + spatial grid live in `map_obstacles` (split out 2026-05-03
// to keep this file from sprawling further).  Re-exported below so existing
// `use crate::map::{MapObstacles, Obstacle, ObstacleShape, ...}` imports
// keep working without churn.
pub use crate::map_obstacles::{MapObstacles, Obstacle, ObstacleShape};

#[inline]
pub fn tile_center(col: i32, row: i32) -> Vec2 {
    Vec2::new(
        -MAP_WIDTH * 0.5 + (col as f32 + 0.5) * TILE_SIZE,
        -MAP_HEIGHT * 0.5 + (row as f32 + 0.5) * TILE_SIZE,
    )
}

#[inline]
pub fn world_to_tile(pos: Vec2) -> (i32, i32) {
    let col = ((pos.x + MAP_WIDTH * 0.5) / TILE_SIZE).floor() as i32;
    let row = ((pos.y + MAP_HEIGHT * 0.5) / TILE_SIZE).floor() as i32;
    (col, row)
}

#[inline]
pub fn in_bounds(col: i32, row: i32) -> bool {
    (0..MAP_COLS).contains(&col) && (0..MAP_ROWS).contains(&row)
}

#[inline]
pub fn nav_idx(col: i32, row: i32) -> usize {
    (row * MAP_COLS + col) as usize
}

// NavGrid + BFS distance fields live in `map_nav` (split out 2026-05-03).
// Re-exported here so existing `use crate::map::{NavGrid, bfs_distance_field, ...}`
// keeps working.
pub use crate::map_nav::{bfs_distance_field, NavGrid};

/// Tile is walkable when it doesn't overlap any wall rect.  This is the
/// build-time predicate used by `NavGrid::default()` and `unlock_nav_rows`.
/// Lives here (not in `map_nav`) because it touches `wall_rects()`, which
/// is map-build state belonging in this file.
pub fn is_walkable_tile(col: i32, row: i32) -> bool {
    if !in_bounds(col, row) {
        return false;
    }
    let center = tile_center(col, row);
    let tile_half = TILE_SIZE * 0.5;
    for &(pos, half) in wall_rects().iter() {
        let d = center - pos;
        if d.x.abs() < tile_half + half.x && d.y.abs() < tile_half + half.y {
            return false;
        }
    }
    true
}

// ════════════════════════════════════════════════════════════════════════
//  Wall generation (perimeter + buildings)
// ════════════════════════════════════════════════════════════════════════

static WALL_RECTS: OnceLock<Vec<(Vec2, Vec2)>> = OnceLock::new();

pub fn wall_rects() -> &'static Vec<(Vec2, Vec2)> {
    WALL_RECTS.get_or_init(|| {
        let mut out: Vec<(Vec2, Vec2)> = Vec::new();
        let half_wt = WALL_THICK * 0.5;

        let (north_gaps, south_gaps, east_gaps, west_gaps) = collect_perimeter_gaps();
        let perim_n_y = MAP_HEIGHT * 0.5 + half_wt;
        let perim_s_y = -MAP_HEIGHT * 0.5 - half_wt;
        let perim_e_x = MAP_WIDTH * 0.5 + half_wt;
        let perim_w_x = -MAP_WIDTH * 0.5 - half_wt;

        push_horizontal_wall(&mut out, perim_n_y, MAP_WIDTH * 0.5, half_wt, &north_gaps);
        push_horizontal_wall(&mut out, perim_s_y, MAP_WIDTH * 0.5, half_wt, &south_gaps);
        push_vertical_wall(&mut out, perim_e_x, MAP_HEIGHT * 0.5, half_wt, &east_gaps);
        push_vertical_wall(&mut out, perim_w_x, MAP_HEIGHT * 0.5, half_wt, &west_gaps);

        // Building walls — 4 segments per building with a 1-tile gap on
        // the door side so the player can enter through the front.
        for b in BUILDINGS {
            push_building_walls(&mut out, b);
        }

        out
    })
}

#[allow(clippy::type_complexity)]
fn collect_perimeter_gaps() -> (
    Vec<(f32, f32)>,
    Vec<(f32, f32)>,
    Vec<(f32, f32)>,
    Vec<(f32, f32)>,
) {
    let mut north_gaps: Vec<(f32, f32)> = vec![];
    let mut south_gaps: Vec<(f32, f32)> = vec![];
    let mut east_gaps: Vec<(f32, f32)> = vec![];
    let mut west_gaps: Vec<(f32, f32)> = vec![];
    for sp in SPAWN_POINTS {
        if sp.span_tiles <= 0 || sp.interior_only {
            continue;
        }
        let center = tile_center(sp.tile.0, sp.tile.1);
        let half_span = sp.span_tiles as f32 * TILE_SIZE * 0.5;
        match sp.side {
            WallSide::N => north_gaps.push((center.x - half_span, center.x + half_span)),
            WallSide::S => south_gaps.push((center.x - half_span, center.x + half_span)),
            WallSide::E => east_gaps.push((center.y - half_span, center.y + half_span)),
            WallSide::W => west_gaps.push((center.y - half_span, center.y + half_span)),
        }
    }
    (north_gaps, south_gaps, east_gaps, west_gaps)
}

fn push_horizontal_wall(
    out: &mut Vec<(Vec2, Vec2)>,
    line_y: f32,
    half_w: f32,
    half_t: f32,
    gaps_x: &[(f32, f32)],
) {
    let mut sorted = gaps_x.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut cursor = -half_w;
    for &(s, e) in &sorted {
        if cursor < s {
            let mid = (cursor + s) * 0.5;
            let half = (s - cursor) * 0.5;
            out.push((Vec2::new(mid, line_y), Vec2::new(half, half_t)));
        }
        cursor = cursor.max(e);
    }
    if cursor < half_w {
        let mid = (cursor + half_w) * 0.5;
        let half = (half_w - cursor) * 0.5;
        out.push((Vec2::new(mid, line_y), Vec2::new(half, half_t)));
    }
}

fn push_vertical_wall(
    out: &mut Vec<(Vec2, Vec2)>,
    line_x: f32,
    half_h: f32,
    half_t: f32,
    gaps_y: &[(f32, f32)],
) {
    let mut sorted = gaps_y.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut cursor = -half_h;
    for &(s, e) in &sorted {
        if cursor < s {
            let mid = (cursor + s) * 0.5;
            let half = (s - cursor) * 0.5;
            out.push((Vec2::new(line_x, mid), Vec2::new(half_t, half)));
        }
        cursor = cursor.max(e);
    }
    if cursor < half_h {
        let mid = (cursor + half_h) * 0.5;
        let half = (half_h - cursor) * 0.5;
        out.push((Vec2::new(line_x, mid), Vec2::new(half_t, half)));
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Coordinate helpers (segment-local → world)
// ════════════════════════════════════════════════════════════════════════

pub fn segment_origin_x(seg_id: u8) -> i32 {
    (seg_id.saturating_sub(1) as i32) * SEG_TILES
}

/// World rect of a building (centre, half-extent) given segment-local coords.
pub fn building_world_rect(b: &Building) -> (Vec2, Vec2) {
    let global_x = segment_origin_x(b.seg_id) + b.x;
    let global_y = b.y;
    let cx = -MAP_WIDTH * 0.5 + (global_x as f32 + b.w as f32 * 0.5) * TILE_SIZE;
    let cy = -MAP_HEIGHT * 0.5 + (global_y as f32 + b.h as f32 * 0.5) * TILE_SIZE;
    let half = Vec2::new(b.w as f32 * TILE_SIZE * 0.5, b.h as f32 * TILE_SIZE * 0.5);
    (Vec2::new(cx, cy), half)
}

/// World position of the door tile centre.
pub fn building_door_world(b: &Building) -> Vec2 {
    let global_x = segment_origin_x(b.seg_id) + b.door.x;
    let global_y = b.door.y;
    tile_center(global_x, global_y)
}

/// Which wall the door sits on, derived from door tile coords vs footprint.
pub fn building_door_side(b: &Building) -> WallSide {
    if b.door.y == b.y {
        WallSide::S
    } else if b.door.y == b.y + b.h - 1 {
        WallSide::N
    } else if b.door.x == b.x {
        WallSide::W
    } else {
        WallSide::E
    }
}

/// Push the 4 wall segments of a building into `out`, leaving a 1-tile
/// gap on the door side.  Used by `wall_rects` and `spawn_map`.
fn push_building_walls(out: &mut Vec<(Vec2, Vec2)>, b: &Building) {
    let (center, half) = building_world_rect(b);
    let t = BUILDING_WALL_THICK * 0.5;
    let n_y = center.y + half.y - t;
    let s_y = center.y - half.y + t;
    let e_x = center.x + half.x - t;
    let w_x = center.x - half.x + t;

    let door_world = building_door_world(b);
    let door_half = TILE_SIZE * 0.5;
    let side = building_door_side(b);

    if matches!(side, WallSide::N) {
        push_horizontal_segment_with_gap(
            out, n_y, t,
            center.x - half.x, center.x + half.x,
            door_world.x - door_half, door_world.x + door_half,
        );
    } else {
        out.push((Vec2::new(center.x, n_y), Vec2::new(half.x, t)));
    }
    if matches!(side, WallSide::S) {
        push_horizontal_segment_with_gap(
            out, s_y, t,
            center.x - half.x, center.x + half.x,
            door_world.x - door_half, door_world.x + door_half,
        );
    } else {
        out.push((Vec2::new(center.x, s_y), Vec2::new(half.x, t)));
    }
    if matches!(side, WallSide::E) {
        push_vertical_segment_with_gap(
            out, e_x, t,
            center.y - half.y, center.y + half.y,
            door_world.y - door_half, door_world.y + door_half,
        );
    } else {
        out.push((Vec2::new(e_x, center.y), Vec2::new(t, half.y)));
    }
    if matches!(side, WallSide::W) {
        push_vertical_segment_with_gap(
            out, w_x, t,
            center.y - half.y, center.y + half.y,
            door_world.y - door_half, door_world.y + door_half,
        );
    } else {
        out.push((Vec2::new(w_x, center.y), Vec2::new(t, half.y)));
    }
}

fn push_horizontal_segment_with_gap(
    out: &mut Vec<(Vec2, Vec2)>,
    line_y: f32,
    half_t: f32,
    left_x: f32,
    right_x: f32,
    gap_l: f32,
    gap_r: f32,
) {
    if gap_l > left_x {
        let mid = (left_x + gap_l) * 0.5;
        let hx = (gap_l - left_x) * 0.5;
        out.push((Vec2::new(mid, line_y), Vec2::new(hx, half_t)));
    }
    if gap_r < right_x {
        let mid = (gap_r + right_x) * 0.5;
        let hx = (right_x - gap_r) * 0.5;
        out.push((Vec2::new(mid, line_y), Vec2::new(hx, half_t)));
    }
}

fn push_vertical_segment_with_gap(
    out: &mut Vec<(Vec2, Vec2)>,
    line_x: f32,
    half_t: f32,
    bot_y: f32,
    top_y: f32,
    gap_b: f32,
    gap_t: f32,
) {
    if gap_b > bot_y {
        let mid = (bot_y + gap_b) * 0.5;
        let hy = (gap_b - bot_y) * 0.5;
        out.push((Vec2::new(line_x, mid), Vec2::new(half_t, hy)));
    }
    if gap_t < top_y {
        let mid = (gap_t + top_y) * 0.5;
        let hy = (top_y - gap_t) * 0.5;
        out.push((Vec2::new(line_x, mid), Vec2::new(half_t, hy)));
    }
}

/// World position of a prop's footprint centre.
pub fn prop_world_center(p: &Prop) -> Vec2 {
    let global_x = segment_origin_x(p.seg_id) + p.x;
    let global_y = p.y;
    let cx = -MAP_WIDTH * 0.5 + (global_x as f32 + p.w as f32 * 0.5) * TILE_SIZE;
    let cy = -MAP_HEIGHT * 0.5 + (global_y as f32 + p.h as f32 * 0.5) * TILE_SIZE;
    Vec2::new(cx, cy)
}

// ════════════════════════════════════════════════════════════════════════
//  Explodables — destructible car wrecks + fuel barrels
// ════════════════════════════════════════════════════════════════════════

/// Component on entities that take bullet damage and explode when destroyed.
#[derive(Component, Debug, Clone, Copy)]
pub struct Explodable {
    pub hp: i32,
    pub radius: f32,
    pub player_damage: i32,
    pub zombie_damage: i32,
    pub kind: ExplodableVisualKind,
}

/// Stores the index in `MapObstacles.list` for an explodable's collision
/// rect.  When the explodable detonates we zero its shape so the wreckage
/// stops blocking movement.
#[derive(Component, Debug, Clone, Copy)]
pub struct ExplodableObstacleIdx(pub usize);

/// Set of obstacle indices for explodables that have been destroyed —
/// used to replicate destruction across the wire.  Host pushes here on
/// detonation; the snapshot ships the full set; clients walk it to find
/// matching `ExplodableObstacleIdx` entities to despawn + zero locally.
#[derive(Resource, Default)]
pub struct DestroyedExplodables {
    pub indices: std::collections::HashSet<u32>,
}

/// Despawn any explodable whose obstacle index appears in
/// `DestroyedExplodables` and zero its collision rect.  Runs every
/// `Update` but is essentially free — `is_changed` short-circuits the
/// no-op case, and the explodable Query only enumerates surviving
/// entities (so already-destroyed ones don't get processed twice).
fn apply_destroyed_explodables(
    mut commands: Commands,
    destroyed: Res<DestroyedExplodables>,
    explodables: Query<(Entity, &ExplodableObstacleIdx)>,
    mut obstacles: ResMut<MapObstacles>,
) {
    if !destroyed.is_changed() {
        return;
    }
    for (entity, idx) in explodables.iter() {
        if !destroyed.indices.contains(&(idx.0 as u32)) {
            continue;
        }
        if let Some(o) = obstacles.list.get_mut(idx.0) {
            o.shape = ObstacleShape::Circle(0.0);
        }
        commands.entity(entity).despawn_recursive();
    }
}

/// Streetlight flicker — modulates the lamp sprite color over time with a
/// per-lamp phase so the lights don't pulse in unison.
#[derive(Component, Debug, Clone, Copy)]
pub struct LampFlicker {
    pub phase: f32,
}

/// Glowing-window decal on apartment / tower facades.  Phase per window so
/// nearby windows don't pulse together — sells the post-apo flickering
/// power-grid look.
#[derive(Component, Debug, Clone, Copy)]
pub struct WindowGlow {
    pub phase: f32,
    pub base_alpha: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum ExplodableVisualKind {
    CarWreck,
    FuelBarrel,
}

#[derive(Clone, Copy, Debug)]
pub struct ExplodableSpec {
    pub seg_id: u8,
    pub kind: ExplodableVisualKind,
    /// Local tile column in the segment (0..48).
    pub local_col: i32,
    /// Local tile row (0..48).
    pub local_row: i32,
}

/// Per-segment ambient explodables: 2-3 destructible car wrecks + 1-2 fuel
/// barrels each, scattered so the player can chain detonations through
/// crowds of zombies.  Positions sit near the segment's road grid where it
/// makes diegetic sense (wrecks block side streets, barrels guard depots).
pub const EXPLODABLES: &[ExplodableSpec] = &[
    // ── Segment 1 — Suburbs (3 wrecks, 2 barrels) ──
    ExplodableSpec { seg_id: 1, kind: ExplodableVisualKind::CarWreck,  local_col: 18, local_row: 23 },
    ExplodableSpec { seg_id: 1, kind: ExplodableVisualKind::CarWreck,  local_col: 32, local_row: 24 },
    ExplodableSpec { seg_id: 1, kind: ExplodableVisualKind::CarWreck,  local_col: 22, local_row: 38 },
    ExplodableSpec { seg_id: 1, kind: ExplodableVisualKind::FuelBarrel, local_col: 8,  local_row: 42 },
    ExplodableSpec { seg_id: 1, kind: ExplodableVisualKind::FuelBarrel, local_col: 41, local_row: 36 },
    // ── Segment 2 — Downtown (3 wrecks, 2 barrels) ──
    ExplodableSpec { seg_id: 2, kind: ExplodableVisualKind::CarWreck,  local_col: 18, local_row: 23 },
    ExplodableSpec { seg_id: 2, kind: ExplodableVisualKind::CarWreck,  local_col: 30, local_row: 24 },
    ExplodableSpec { seg_id: 2, kind: ExplodableVisualKind::CarWreck,  local_col: 22, local_row: 18 },
    ExplodableSpec { seg_id: 2, kind: ExplodableVisualKind::FuelBarrel, local_col: 33, local_row: 16 },
    ExplodableSpec { seg_id: 2, kind: ExplodableVisualKind::FuelBarrel, local_col: 9,  local_row: 36 },
    // ── Segment 3 — Industrial (3 wrecks, 2 barrels — extra fuel here) ──
    ExplodableSpec { seg_id: 3, kind: ExplodableVisualKind::CarWreck,  local_col: 19, local_row: 23 },
    ExplodableSpec { seg_id: 3, kind: ExplodableVisualKind::CarWreck,  local_col: 30, local_row: 24 },
    ExplodableSpec { seg_id: 3, kind: ExplodableVisualKind::CarWreck,  local_col: 23, local_row: 11 },
    ExplodableSpec { seg_id: 3, kind: ExplodableVisualKind::FuelBarrel, local_col: 25, local_row: 33 },
    ExplodableSpec { seg_id: 3, kind: ExplodableVisualKind::FuelBarrel, local_col: 8,  local_row: 21 },
    // ── Segment 4 — Hospital & Park (2 wrecks, 1 barrel) ──
    ExplodableSpec { seg_id: 4, kind: ExplodableVisualKind::CarWreck,  local_col: 18, local_row: 23 },
    ExplodableSpec { seg_id: 4, kind: ExplodableVisualKind::CarWreck,  local_col: 31, local_row: 24 },
    ExplodableSpec { seg_id: 4, kind: ExplodableVisualKind::FuelBarrel, local_col: 21, local_row: 21 },
    // ── Segment 5 — Military (3 wrecks, 2 barrels — heavy presence) ──
    ExplodableSpec { seg_id: 5, kind: ExplodableVisualKind::CarWreck,  local_col: 19, local_row: 23 },
    ExplodableSpec { seg_id: 5, kind: ExplodableVisualKind::CarWreck,  local_col: 31, local_row: 24 },
    ExplodableSpec { seg_id: 5, kind: ExplodableVisualKind::CarWreck,  local_col: 24, local_row: 36 },
    ExplodableSpec { seg_id: 5, kind: ExplodableVisualKind::FuelBarrel, local_col: 21, local_row: 36 },
    ExplodableSpec { seg_id: 5, kind: ExplodableVisualKind::FuelBarrel, local_col: 33, local_row: 11 },
];

impl ExplodableVisualKind {
    pub fn default_spec(self) -> Explodable {
        match self {
            ExplodableVisualKind::CarWreck => Explodable {
                hp: 60,
                radius: 90.0,
                player_damage: 35,
                zombie_damage: 14,
                kind: self,
            },
            ExplodableVisualKind::FuelBarrel => Explodable {
                hp: 25,
                radius: 75.0,
                player_damage: 30,
                zombie_damage: 14,
                kind: self,
            },
        }
    }

    pub fn sprite_size(self) -> Vec2 {
        match self {
            ExplodableVisualKind::CarWreck => Vec2::new(64.0, 32.0),
            ExplodableVisualKind::FuelBarrel => Vec2::new(28.0, 28.0),
        }
    }

    pub fn collision_half(self) -> Vec2 {
        match self {
            ExplodableVisualKind::CarWreck => Vec2::new(28.0, 12.0),
            ExplodableVisualKind::FuelBarrel => Vec2::new(11.0, 11.0),
        }
    }
}

pub fn explodable_world_center(spec: &ExplodableSpec) -> Vec2 {
    let global_col = segment_origin_x(spec.seg_id) + spec.local_col;
    tile_center(global_col, spec.local_row)
}

/// Returns Explodable stats for vehicle prop kinds.  `None` for non-vehicle
/// props.  Bigger vehicles take more punishment but make a larger crater.
pub fn vehicle_explodable_for(kind: PropKind) -> Option<Explodable> {
    use PropKind as P;
    let (hp, radius, player_damage, zombie_damage) = match kind {
        // Civilian street wrecks — fragile, decent boom.
        P::Car | P::Wreck => (45, 80.0, 28, 13),
        // Heavy vehicles — more HP, bigger blast radius.
        P::Bus => (110, 120.0, 45, 18),
        P::Truck | P::MilTruck => (95, 110.0, 40, 16),
        P::Ambulance => (75, 95.0, 32, 14),
        P::Jeep => (55, 90.0, 30, 14),
        _ => return None,
    };
    Some(Explodable {
        hp,
        radius,
        player_damage,
        zombie_damage,
        kind: ExplodableVisualKind::CarWreck,
    })
}

// (Legacy `ShopKind`/`ShopSpec`/`ElevatorSpec`/`DoorSide`/`MarkerKind`/
// `SHOPS`/`ELEVATORS`/`shop_back_room_pos`/`shop_wall_rects` compat shims
// removed — the metro map doesn't have shops or elevators, and nothing
// outside this file ever read those types.)

// ════════════════════════════════════════════════════════════════════════
//  Segment unlock state (now: 5 segments, gates between them)
// ════════════════════════════════════════════════════════════════════════

#[derive(Resource)]
pub struct MapSegmentUnlockState {
    /// `unlocked[i]` true ⇒ segment `i+1` is open.  Segment 1 always true.
    pub unlocked: [bool; 5],
}

impl Default for MapSegmentUnlockState {
    fn default() -> Self {
        // Only segment 1 (the spawn area) is open at the start.  Segments 2-5
        // are gated behind the money-buy barriers — the player walks up to a
        // gate and pays its `cost` (see GATES / `unlock_segments_by_input`) to
        // drop the barrier and clear the fog over the next segment.
        let mut unlocked = [false; 5];
        unlocked[0] = true;
        Self { unlocked }
    }
}

impl MapSegmentUnlockState {
    pub fn is_unlocked(&self, seg_id: u8) -> bool {
        if seg_id == 0 || seg_id > 5 {
            return seg_id == 0;
        }
        self.unlocked
            .get((seg_id - 1) as usize)
            .copied()
            .unwrap_or(false)
    }
    pub fn unlock(&mut self, seg_id: u8) {
        if (1..=5).contains(&seg_id) {
            self.unlocked[(seg_id - 1) as usize] = true;
        }
    }
    pub fn as_mask(&self) -> u8 {
        let mut m = 0u8;
        for (i, &v) in self.unlocked.iter().enumerate() {
            if v {
                m |= 1 << i;
            }
        }
        m
    }
    pub fn apply_mask(&mut self, mask: u8) {
        for i in 0..5 {
            self.unlocked[i] = mask & (1 << i) != 0;
        }
        self.unlocked[0] = true;
    }
}

#[derive(Component)]
pub struct SegmentFog {
    pub idx: u8,
}

#[derive(Component)]
pub struct GateBarrier {
    pub from_seg: u8,
    pub to_seg: u8,
    pub obstacle_idx: usize,
}

/// Marker on a building's roof sprite — toggled to hide the roof when the
/// local player crosses the door threshold so the interior is revealed.
#[derive(Component)]
pub struct BuildingRoof {
    pub idx: usize,
}

/// Tags an entity as belonging to a specific floor of a specific building.
/// `floor = 0` is the ground floor (default visibility); higher floors are
/// shown only when the local player is on that floor of that building.
#[derive(Component, Debug, Clone, Copy)]
pub struct FloorEntity {
    pub building: usize,
    pub floor: u8,
}

/// Interactable staircase prop — pressing E on it cycles the local player
/// between Ground (floor 0) and Roof (floor 1) of `building`.
#[derive(Component, Debug, Clone, Copy)]
pub struct Staircase {
    pub building: usize,
}

/// Local-player floor tracker.  `building = None` means the player is
/// outside every building (default ground state).  When inside a building,
/// `floor` selects which storey to render.  Per-client only — multiplayer
/// peers see remote players on the ground floor regardless.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct PlayerFloorState {
    pub building: Option<usize>,
    pub floor: u8,
}

/// Decrementing seconds-since-last-stair-use.  Stops the stand-on-staircase
/// auto-trigger from cycling floors every tick; also bumped when the player
/// climbs via E so they can step off without instantly being teleported back
/// up.  Kept *separate* from `PlayerFloorState` on purpose: it ticks down
/// every frame, and folding it into `PlayerFloorState` would mark that
/// resource changed every frame, defeating the change-detection gate on the
/// floor/roof toggle systems (`toggle_roof_walls`, `toggle_floor_obstacles`,
/// `update_floor_entity_visibility`).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct StairCooldown(pub f32);

/// Per-building wall obstacle indices captured at spawn time.  When the
/// local player is on a building's roof, those walls are temporarily
/// passable so the player can walk off the edge (= fall).  Restored when
/// they go back down.
#[derive(Resource, Default)]
pub struct BuildingWallIndices {
    pub walls: std::collections::HashMap<usize, Vec<(usize, ObstacleShape)>>,
}

/// Per-floor obstacle indices for furniture inside multi-story buildings.
/// Each floor's obstacles are activated only when the local player is on
/// that floor of that building (or, for ground floor, when outside any
/// building).  Single-floor buildings put their furniture directly into
/// `MapObstacles` without floor-tagging.
#[derive(Resource, Default)]
pub struct FloorObstacleIndices {
    pub by: std::collections::HashMap<(usize, u8), Vec<(usize, ObstacleShape)>>,
}

/// Number of accessible floors for a building type.  Most are single-floor
/// solid blocks; apartment blocks get **5 floors** (lobby + 3 piętra +
/// roof) for proper "wielka płyta" feel, hospitals get 4 (lobby + 2 patient
/// floors + roof).
pub fn building_floor_count(kind: BuildingType) -> u8 {
    match kind {
        BuildingType::Apartment => 5,
        BuildingType::Hospital => 4,
        _ => 1,
    }
}

pub fn has_roof_access(kind: BuildingType) -> bool {
    building_floor_count(kind) > 1
}

/// Index of the highest floor (= the roof) for a building.
pub fn top_floor(kind: BuildingType) -> u8 {
    building_floor_count(kind).saturating_sub(1)
}

/// Internal wall layout for the residential ("wielka płyta") floor of a
/// multi-story building.  Returns segments as `(centre offset from building
/// centre, half-extent)`.  The layout forms a "+" cross with corridor gaps
/// at the centre, splitting the inner volume into four mieszkania plus a
/// circulation crossroad that connects to the staircase corner.
///
/// Returned offsets are relative to the building centre — the caller adds
/// `building_world_rect(b).0` to place each segment in world space.
pub fn residential_floor_walls(b: &Building) -> Vec<(Vec2, Vec2)> {
    let (_, half) = building_world_rect(b);
    // Inner half-extent = building half minus outer wall thickness.
    let inner_half = Vec2::new(
        (half.x - BUILDING_WALL_THICK).max(8.0),
        (half.y - BUILDING_WALL_THICK).max(8.0),
    );
    let half_t = INTERNAL_WALL_THICK * 0.5;
    // ~1.3 tiles of doorway centred at the cross — enough room for the
    // player to slip through but the walls still read as corridor dividers.
    let corridor_half = TILE_SIZE * 0.7;

    let mut out: Vec<(Vec2, Vec2)> = Vec::new();
    // Horizontal central wall — split by the doorway.
    let h_seg_half_x = ((inner_half.x - corridor_half) * 0.5).max(4.0);
    out.push((
        Vec2::new(-(corridor_half + h_seg_half_x), 0.0),
        Vec2::new(h_seg_half_x, half_t),
    ));
    out.push((
        Vec2::new(corridor_half + h_seg_half_x, 0.0),
        Vec2::new(h_seg_half_x, half_t),
    ));
    // Vertical central wall — same split.
    let v_seg_half_y = ((inner_half.y - corridor_half) * 0.5).max(4.0);
    out.push((
        Vec2::new(0.0, -(corridor_half + v_seg_half_y)),
        Vec2::new(half_t, v_seg_half_y),
    ));
    out.push((
        Vec2::new(0.0, corridor_half + v_seg_half_y),
        Vec2::new(half_t, v_seg_half_y),
    ));
    out
}

/// World-space position of a building's staircase (a corner of its inner
/// rect, away from the door so it doesn't block entry).
pub fn staircase_world_pos(b: &Building) -> Vec2 {
    let (center, half) = building_world_rect(b);
    let inset = BUILDING_WALL_THICK + TILE_SIZE * 0.6;
    let side = building_door_side(b);
    // Pick the corner OPPOSITE the door so the player walks across the
    // building to reach the stairs.
    match side {
        WallSide::S => Vec2::new(center.x + half.x - inset, center.y + half.y - inset),
        WallSide::N => Vec2::new(center.x + half.x - inset, center.y - half.y + inset),
        WallSide::W => Vec2::new(center.x + half.x - inset, center.y + half.y - inset),
        WallSide::E => Vec2::new(center.x - half.x + inset, center.y + half.y - inset),
    }
}

/// Furniture types placed inside buildings.  Each spawns a sprite + an
/// obstacle so players can navigate around them.  Reuses some of the
/// outdoor prop kinds (Crate, Barrels) and adds new interior-only ones.
///
/// Several variants (KitchenSink, BathSink, CoffeeTable, FloorLamp, Rug,
/// Painting, Trashcan, OfficeChair) have full sprite + collision support
/// but are reserved for future room layouts — `furniture_for_floor`
/// doesn't currently place them.  Once added there, the dead_code allow
/// can come off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum FurnKind {
    Bed,
    Couch,
    Tv,
    Counter,
    Desk,
    Cot,
    Shelf,
    Altar,
    Crate,
    Barrels,
    Gurney,
    Bench,
    // ── Kitchen ────────────────────────────────────────────────────────
    Fridge,
    Stove,
    KitchenSink,
    // ── Bathroom ───────────────────────────────────────────────────────
    Toilet,
    Bathtub,
    BathSink,
    // ── Bedroom ────────────────────────────────────────────────────────
    Dresser,
    Wardrobe,
    Nightstand,
    // ── Living / dining ────────────────────────────────────────────────
    CoffeeTable,
    Bookshelf,
    DiningTable,
    DiningChair,
    ArmChair,
    Fireplace,
    // ── Decoration (no collision) ──────────────────────────────────────
    FloorLamp,
    Rug,
    Plant,
    Painting,
    // ── Misc ───────────────────────────────────────────────────────────
    Trashcan,
    FilingCabinet,
    OfficeChair,
}

/// Per-archetype furniture layout per floor.  `floor=0` is ground (default
/// outside view).  For single-floor buildings only floor 0 has content.
/// For multi-story buildings (apartment, hospital) floors 1+ have distinct
/// content; the topmost floor is the rooftop and is filled by
/// `spawn_multi_floor_extras`, not here.
pub fn furniture_for_floor(
    kind: BuildingType,
    floor: u8,
) -> &'static [(FurnKind, f32, f32)] {
    use BuildingType as B;
    use FurnKind as F;
    match (kind, floor) {
        // ── Apartment block: lobby → residential (4 mieszkania) → roof ──
        (B::Apartment, 0) => &[
            // Lobby — mailbox counter + sitting area, sparse on purpose.
            (F::Counter, 0.0, 70.0),
            (F::Couch, -55.0, -10.0),
            (F::Couch, 55.0, -10.0),
            (F::Tv, 0.0, -55.0),
            (F::Plant, -90.0, 80.0),
        ],
        (B::Apartment, 1) => &[
            // 1. piętro — four mieszkania, only the essentials per quadrant.
            // NW: sypialnia
            (F::Bed, -60.0, 75.0),
            (F::Nightstand, -32.0, 75.0),
            (F::Dresser, -85.0, 38.0),
            // NE: kuchnia
            (F::Fridge, 92.0, 88.0),
            (F::Stove, 60.0, 92.0),
            (F::DiningTable, 65.0, 45.0),
            // SW: salon
            (F::Couch, -60.0, -45.0),
            (F::Tv, -90.0, -75.0),
            (F::Bookshelf, -85.0, -45.0),
            // SE: łazienka
            (F::Bathtub, 70.0, -90.0),
            (F::Toilet, 38.0, -45.0),
            (F::Wardrobe, 95.0, -50.0),
        ],
        (B::Apartment, 2) => &[
            // 2. piętro — mirrored layout vs floor 1.
            // NW: kuchnia
            (F::Fridge, -92.0, 88.0),
            (F::Stove, -60.0, 92.0),
            (F::DiningTable, -65.0, 45.0),
            // NE: sypialnia
            (F::Bed, 60.0, 75.0),
            (F::Nightstand, 32.0, 75.0),
            (F::Dresser, 85.0, 38.0),
            // SW: łazienka
            (F::Bathtub, -70.0, -90.0),
            (F::Toilet, -38.0, -45.0),
            (F::Wardrobe, -95.0, -50.0),
            // SE: salon
            (F::Couch, 60.0, -45.0),
            (F::Tv, 90.0, -75.0),
            (F::Bookshelf, 85.0, -45.0),
        ],
        (B::Apartment, 3) => &[
            // 3. piętro — penthouse: fireplace lounge above, bedroom + dining below.
            (F::Fireplace, 0.0, 95.0),
            (F::Couch, -50.0, 50.0),
            (F::ArmChair, 50.0, 50.0),
            (F::Bookshelf, -90.0, 70.0),
            (F::Bed, -55.0, -55.0),
            (F::Wardrobe, -95.0, -85.0),
            (F::DiningTable, 55.0, -55.0),
            (F::DiningChair, 80.0, -55.0),
        ],
        // ── Hospital: ER reception → patient rooms (4 sale) → helipad ───
        (B::Hospital, 0) => &[
            (F::Counter, 0.0, 35.0),
            (F::Gurney, -55.0, -10.0),
            (F::Gurney, 55.0, -10.0),
            (F::Bench, 0.0, -35.0),
        ],
        (B::Hospital, 1) => &[
            // 1. piętro — patient rooms divided by interior walls.
            (F::Gurney, -55.0, 45.0),
            (F::Gurney, 55.0, 45.0),
            (F::Gurney, -55.0, -45.0),
            (F::Gurney, 55.0, -45.0),
            (F::Counter, 0.0, 0.0),
        ],
        (B::Hospital, 2) => &[
            // 2. piętro — different ward layout.
            (F::Gurney, -55.0, 50.0),
            (F::Gurney, -50.0, 20.0),
            (F::Gurney, 55.0, 50.0),
            (F::Counter, 50.0, 18.0),
            (F::Gurney, -55.0, -50.0),
            (F::Gurney, 55.0, -50.0),
        ],
        // Top floors of multi-story buildings have no interior furniture
        // (rooftop content is spawned separately).
        (B::Apartment, _) | (B::Hospital, _) => &[],

        // ── Single-floor archetypes — only floor 0 has content ──────────
        (B::House, 0) => &[
            // Open-plan suburban house: bedroom NW, bathroom NE, salon SW,
            // kitchen+dining SE.  Just the essentials so the room reads
            // without feeling crammed.
            (F::Bed, -55.0, 50.0),
            (F::Wardrobe, -85.0, 50.0),
            (F::Toilet, 75.0, 65.0),
            (F::Bathtub, 60.0, 40.0),
            (F::Couch, -55.0, -35.0),
            (F::Tv, -85.0, -60.0),
            (F::Bookshelf, -78.0, -10.0),
            (F::Fridge, 80.0, -50.0),
            (F::Stove, 50.0, -55.0),
            (F::DiningTable, 55.0, -10.0),
            (F::DiningChair, 80.0, -10.0),
        ],
        (B::Shed, 0) => &[
            (F::Crate, -20.0, -10.0),
            (F::Barrels, 20.0, 10.0),
        ],
        (B::Garage, 0) => &[
            (F::Crate, -35.0, -20.0),
            (F::Barrels, 35.0, -20.0),
            (F::Shelf, 0.0, 30.0),
        ],
        (B::Shop, 0) | (B::Market, 0) => &[
            (F::Counter, 0.0, -25.0),
            (F::Shelf, -50.0, 30.0),
            (F::Shelf, 50.0, 30.0),
            (F::Crate, 0.0, 30.0),
        ],
        (B::Civic, 0) => &[
            (F::Desk, -45.0, 25.0),
            (F::Desk, 45.0, 25.0),
            (F::FilingCabinet, -55.0, -35.0),
            (F::FilingCabinet, 55.0, -35.0),
            (F::Bench, 0.0, -55.0),
        ],
        (B::Church, 0) => &[
            (F::Altar, 0.0, 60.0),
            (F::Bench, 0.0, 10.0),
            (F::Bench, 0.0, -30.0),
            (F::Bench, 0.0, -65.0),
        ],
        (B::Bank, 0) => &[
            (F::Counter, 0.0, -25.0),
            (F::Desk, -45.0, 35.0),
            (F::Desk, 45.0, 35.0),
            (F::FilingCabinet, -80.0, 0.0),
        ],
        (B::Tower, 0) => &[
            (F::Desk, -25.0, 0.0),
            (F::FilingCabinet, 30.0, 0.0),
        ],
        (B::Factory, 0) => &[
            (F::Crate, -60.0, -30.0),
            (F::Crate, 30.0, 0.0),
            (F::Barrels, 60.0, 30.0),
            (F::Barrels, -60.0, 30.0),
        ],
        (B::Warehouse, 0) => &[
            (F::Crate, -60.0, -50.0),
            (F::Crate, 60.0, -50.0),
            (F::Crate, 0.0, 50.0),
            (F::Barrels, -50.0, 50.0),
            (F::Barrels, 50.0, 50.0),
        ],
        (B::Depot, 0) => &[
            (F::Crate, -50.0, -25.0),
            (F::Barrels, 50.0, -25.0),
            (F::Shelf, 0.0, 35.0),
        ],
        (B::Morgue, 0) => &[
            (F::Gurney, -45.0, -30.0),
            (F::Gurney, 45.0, -30.0),
            (F::FilingCabinet, -65.0, 50.0),
            (F::FilingCabinet, 65.0, 50.0),
        ],
        (B::Park, 0) => &[
            (F::Bench, -45.0, 0.0),
            (F::Bench, 45.0, 0.0),
        ],
        (B::Bunker, 0) => &[
            (F::Cot, -50.0, -30.0),
            (F::Cot, 50.0, -30.0),
            (F::Crate, 0.0, 45.0),
        ],
        (B::Tent, 0) => &[
            (F::Cot, -28.0, -8.0),
            (F::Crate, 28.0, -8.0),
        ],
        (B::Gas, 0) => &[
            (F::Counter, 0.0, -20.0),
            (F::Shelf, -55.0, 25.0),
            (F::Shelf, 55.0, 25.0),
            (F::Fridge, -90.0, -25.0),
        ],
        _ => &[],
    }
}

/// Resource updated each frame with the nearest unlock-target.
#[derive(Resource, Default, Clone, Copy)]
pub struct SegmentUnlockHint {
    pub segment_idx: Option<u8>,
    pub cost: u32,
    pub affordable: bool,
}

pub fn segment_name(seg_id: u8) -> &'static str {
    SEGMENTS
        .iter()
        .find(|s| s.id == seg_id)
        .map(|s| s.pl_name)
        .unwrap_or("")
}

const SEGMENT_UNLOCK_RADIUS: f32 = 100.0;
const SEGMENT_FOG_Z: f32 = 8.5;
const GATE_BARRIER_Z: f32 = 9.0;

/// World-space position of the gate between `from_seg` and `from_seg+1`.
pub fn gate_world_pos(from_seg: u8) -> Vec2 {
    let boundary_col = from_seg as i32 * SEG_TILES;
    let cx = -MAP_WIDTH * 0.5 + boundary_col as f32 * TILE_SIZE;
    Vec2::new(cx, 0.0)
}

// ════════════════════════════════════════════════════════════════════════
//  Plugin
// ════════════════════════════════════════════════════════════════════════

#[derive(Component)]
pub struct MapDecor;

pub struct MapPlugin;

/// Marker set for systems that read & maybe consume `LocalInput.interact`
/// (gates, staircases, weapon-swap).  `clear_interact_flag` runs strictly
/// after this set so a leftover press never carries to the next tick.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct InteractConsumers;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapObstacles>()
            .init_resource::<NavGrid>()
            .init_resource::<MapSegmentUnlockState>()
            .init_resource::<SegmentUnlockHint>()
            .init_resource::<PlayerFloorState>()
            .init_resource::<StairCooldown>()
            .init_resource::<BuildingWallIndices>()
            .init_resource::<FloorObstacleIndices>()
            .init_resource::<DestroyedExplodables>()
            .add_systems(Startup, spawn_map)
            .add_systems(
                OnEnter(GameState::Playing),
                (reset_segment_state, reset_player_floor_state),
            )
            .add_systems(
                Update,
                (
                    apply_destroyed_explodables,
                    update_segment_fog_visibility,
                    refresh_segment_unlock_hint,
                    update_building_roof_visibility,
                    track_player_building_floor,
                    update_floor_entity_visibility,
                    toggle_roof_walls,
                    toggle_floor_obstacles,
                    check_roof_fall,
                )
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                (
                    animate_segment_fog,
                    pulse_gate_barriers,
                    animate_lamp_flicker,
                    animate_window_glow,
                )
                    // Purely cosmetic — freeze during a singleplayer pause so the
                    // phone isn't churning hundreds of sprite mutations while the
                    // player has deliberately stopped.  `gameplay_active` stays
                    // true in multiplayer (the world never pauses there).
                    .run_if(gameplay_active),
            )
            .add_systems(
                FixedUpdate,
                (unlock_segments_by_input, staircase_interact)
                    .in_set(InteractConsumers)
                    .run_if(gameplay_active),
            )
            .add_systems(
                FixedUpdate,
                clear_interact_flag
                    .after(InteractConsumers)
                    .run_if(gameplay_active),
            );
    }
}

/// Clears `LocalInput.interact` after every consumer has had a shot at it
/// in this FixedUpdate cycle.  Prevents a stale flag from a press that
/// landed nowhere useful from auto-triggering when the player later walks
/// past a gate, staircase, or pickup.
pub fn clear_interact_flag(mut local: ResMut<crate::net::LocalInput>) {
    local.0.interact = false;
}

/// Slow drift + breathing alpha on each locked-segment fog overlay so the
/// covered areas don't read as a static grey wall.  Each segment uses a
/// different phase derived from its id, so the breathing isn't synchronised
/// across the world.
fn animate_segment_fog(
    time: Res<Time>,
    mut q: Query<(&SegmentFog, &mut Sprite, &mut Transform)>,
) {
    let t = time.elapsed_seconds();
    // Keep the fog opaque on mobile (see spawn_segment_fog_and_gates) so it
    // stays a single non-blended layer; desktop breathes its alpha.
    let lean = cfg!(any(target_os = "android", target_os = "ios"));
    for (fog, mut sprite, mut transform) in &mut q {
        let phase = fog.idx as f32 * 1.37;
        let breathe = if lean {
            1.0
        } else {
            0.78 + (t * 0.55 + phase).sin() * 0.07
        };
        sprite.color.set_alpha(breathe);
        // Tiny x/y drift gives the cloud cover a sense of movement.
        let origin_x = -MAP_WIDTH * 0.5
            + segment_origin_x(fog.idx) as f32 * TILE_SIZE
            + SEG_WIDTH * 0.5;
        transform.translation.x = origin_x + (t * 12.0 + phase).sin() * 6.0;
        transform.translation.y = (t * 8.0 + phase * 0.7).cos() * 4.0;
    }
}

/// Pulses the window-glow alpha over time per-window.  Combines a slow
/// sine with a small high-freq jitter so most windows breathe steadily but
/// a few look like flickering bulbs.
fn animate_window_glow(time: Res<Time>, mut q: Query<(&WindowGlow, &mut Sprite)>) {
    let t = time.elapsed_seconds();
    for (glow, mut sprite) in &mut q {
        let slow = (t * 0.7 + glow.phase).sin() * 0.5 + 0.5;
        let jitter = ((t * 6.0 + glow.phase * 1.7).sin() * 0.5 + 0.5).powi(8) * 0.4;
        let mix = slow * 0.78 + jitter * 0.22;
        let alpha = (glow.base_alpha * (0.55 + mix * 0.55)).clamp(0.0, 1.0);
        sprite.color.set_alpha(alpha);
    }
}

/// Scatters a few glowing window panes on the outside of an apartment /
/// tower building.  Skips the wall side that holds the door so the entry
/// way reads cleanly.  Tagged `BuildingRoof` so they hide when the local
/// player steps inside the building (they belong to the exterior).
fn spawn_building_windows(
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    idx: usize,
    b: &Building,
) {
    use rand::Rng;
    let (center, half) = building_world_rect(b);
    let door_side = building_door_side(b);
    let win_tex = images.add(build_window_image());

    // Place rows of windows along each non-door wall.  Inset slightly so
    // they sit "inside" the wall thickness rather than on top of it.
    let wall_inset = BUILDING_WALL_THICK * 0.5 + 0.5;
    let win_w = TILE_SIZE * 0.55;
    let win_h = TILE_SIZE * 0.42;

    let mut rng = rand::thread_rng();

    let mut place = |pos: Vec2, phase_seed: f32, cmds: &mut Commands| {
        let base_alpha: f32 = if rng.gen_bool(0.85) { 0.82 } else { 0.42 };
        cmds.spawn((
            SpriteBundle {
                texture: win_tex.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::new(win_w, win_h)),
                    color: Color::srgba(1.0, 0.86, 0.42, base_alpha),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, -3.0),
                ..default()
            },
            WindowGlow {
                phase: phase_seed,
                base_alpha,
            },
            BuildingRoof { idx },
        ));
    };

    // Determine how many windows fit per side.  ~1 every 1.4 tiles.
    let cols = ((b.w as f32 / 1.4).floor() as i32).max(2);
    let rows = ((b.h as f32 / 1.4).floor() as i32).max(2);

    // South wall.
    if !matches!(door_side, WallSide::S) {
        let y = center.y - half.y + wall_inset + win_h * 0.5;
        for i in 0..cols {
            let t = (i as f32 + 0.5) / cols as f32;
            let x = center.x - half.x + t * (half.x * 2.0);
            place(Vec2::new(x, y), x * 0.07 + y * 0.13, commands);
        }
    }
    // North wall.
    if !matches!(door_side, WallSide::N) {
        let y = center.y + half.y - wall_inset - win_h * 0.5;
        for i in 0..cols {
            let t = (i as f32 + 0.5) / cols as f32;
            let x = center.x - half.x + t * (half.x * 2.0);
            place(Vec2::new(x, y), x * 0.09 + y * 0.11, commands);
        }
    }
    // West wall.
    if !matches!(door_side, WallSide::W) {
        let x = center.x - half.x + wall_inset + win_w * 0.5;
        for i in 0..rows {
            let t = (i as f32 + 0.5) / rows as f32;
            let y = center.y - half.y + t * (half.y * 2.0);
            place(Vec2::new(x, y), x * 0.05 + y * 0.17, commands);
        }
    }
    // East wall.
    if !matches!(door_side, WallSide::E) {
        let x = center.x + half.x - wall_inset - win_w * 0.5;
        for i in 0..rows {
            let t = (i as f32 + 0.5) / rows as f32;
            let y = center.y - half.y + t * (half.y * 2.0);
            place(Vec2::new(x, y), x * 0.11 + y * 0.06, commands);
        }
    }
}

/// Streetlight flicker — combines a slow sine pulse with an occasional
/// "glitch" dim to sell the busted-power-grid feel.  Each lamp gets a
/// unique phase so the world isn't blinking in unison.
fn animate_lamp_flicker(
    time: Res<Time>,
    mut q: Query<(&LampFlicker, &mut Sprite)>,
) {
    let t = time.elapsed_seconds();
    for (flicker, mut sprite) in &mut q {
        let slow = (t * 1.6 + flicker.phase).sin() * 0.5 + 0.5; // 0..1
        // "Glitch" — a higher-frequency component sometimes drops harder.
        let glitch = ((t * 11.0 + flicker.phase * 2.3).sin() * 0.5 + 0.5).powi(6);
        let mix = slow * 0.85 + glitch * 0.15;
        // Scale color brightness by mix (0.55..1.0); keeps lamp colour but
        // dims it during dips.
        let brightness = 0.55 + mix * 0.45;
        sprite.color = Color::srgba(brightness, brightness * 0.98, brightness * 0.78, 1.0);
    }
}

/// Soft pulse on the gate sprites — alternates between cool grey and a warm
/// highlight so the player notices each unlockable gate even from far away.
fn pulse_gate_barriers(
    time: Res<Time>,
    mut q: Query<(&GateBarrier, &mut Sprite)>,
) {
    let t = time.elapsed_seconds();
    for (gate, mut sprite) in &mut q {
        let phase = gate.from_seg as f32 * 0.9;
        // Sine in [0, 1] for the warm highlight ramp.
        let pulse = (t * 1.4 + phase).sin() * 0.5 + 0.5;
        // Lerp between cool grey-ish base and warm gold when pulsing.
        let r = 0.92 + 0.08 * pulse;
        let g = 0.84 + 0.10 * pulse;
        let b = 0.62 + 0.05 * pulse;
        sprite.color = Color::srgba(r, g, b, 1.0);
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Scene assembly
// ════════════════════════════════════════════════════════════════════════

/// Z-layer plan:
///   -20 ground (grass)
///   -19 road asphalt
///   -18 sidewalk
///   -13 blood / oil / debris flat decals
///   -5  building wall
///   -4  building roof (sits over wall, no interior reveal)
///   -3  small props in front of buildings (planters, mailboxes)
///   -1  vehicles, dumpsters, large props
///   +5  zombies (unchanged)
///   +6  building rooftop highlights / lamps / flags
///   +8.5 segment fog overlay
///   +9  gate barrier sprite
///   +10 player
fn spawn_map(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut obstacles: ResMut<MapObstacles>,
    mut wall_indices: ResMut<BuildingWallIndices>,
    mut floor_obstacles: ResMut<FloorObstacleIndices>,
    gfx: Res<GraphicsSettings>,
) {
    let _ = gfx;

    // ── Ground per segment (themed grass) ───────────────────────────────
    for seg in SEGMENTS.iter() {
        let tex = images.add(build_grass_image(seg.theme));
        let origin_x = -MAP_WIDTH * 0.5 + segment_origin_x(seg.id) as f32 * TILE_SIZE
            + SEG_WIDTH * 0.5;
        commands.spawn(SpriteBundle {
            texture: tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(SEG_WIDTH, MAP_HEIGHT)),
                ..default()
            },
            transform: Transform::from_xyz(origin_x, 0.0, -20.0),
            ..default()
        });
    }

    // ── Roads (per segment: horizontal band + vertical band + sidewalks) ──
    let road_tex = images.add(build_road_image());
    let sidewalk_tex = images.add(build_sidewalk_image());
    for seg in SEGMENTS.iter() {
        let seg_x_origin = segment_origin_x(seg.id);
        let road_h_min = -MAP_HEIGHT * 0.5 + ROAD_H_TOP as f32 * TILE_SIZE;
        let road_h_max = -MAP_HEIGHT * 0.5 + (ROAD_H_BOT + 1) as f32 * TILE_SIZE;
        let road_h_cy = (road_h_min + road_h_max) * 0.5;
        let road_h_h = road_h_max - road_h_min;
        let road_v_min = -MAP_WIDTH * 0.5 + (seg_x_origin + ROAD_V_LEFT) as f32 * TILE_SIZE;
        let road_v_max =
            -MAP_WIDTH * 0.5 + (seg_x_origin + ROAD_V_RIGHT + 1) as f32 * TILE_SIZE;
        let road_v_cx = (road_v_min + road_v_max) * 0.5;
        let road_v_w = road_v_max - road_v_min;

        let seg_x_min = -MAP_WIDTH * 0.5 + seg_x_origin as f32 * TILE_SIZE;
        let seg_cx = seg_x_min + SEG_WIDTH * 0.5;

        // Horizontal road band — full segment width.
        commands.spawn(SpriteBundle {
            texture: road_tex.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(SEG_WIDTH, road_h_h)),
                ..default()
            },
            transform: Transform::from_xyz(seg_cx, road_h_cy, -19.0),
            ..default()
        });
        // Vertical road band — full map height.
        commands.spawn(SpriteBundle {
            texture: road_tex.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(road_v_w, MAP_HEIGHT)),
                ..default()
            },
            transform: Transform::from_xyz(road_v_cx, 0.0, -19.0),
            ..default()
        });
        // Sidewalk rim around roads (4 strips along H, 4 along V).
        // Top + bottom strips of horizontal road.
        for &row in &[ROAD_H_TOP - 1, ROAD_H_BOT + 1] {
            let cy = -MAP_HEIGHT * 0.5 + (row as f32 + 0.5) * TILE_SIZE;
            commands.spawn(SpriteBundle {
                texture: sidewalk_tex.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::new(SEG_WIDTH, TILE_SIZE)),
                    ..default()
                },
                transform: Transform::from_xyz(seg_cx, cy, -18.0),
                ..default()
            });
        }
        // Left + right strips of vertical road.
        for &col in &[ROAD_V_LEFT - 1, ROAD_V_RIGHT + 1] {
            let cx = -MAP_WIDTH * 0.5 + (seg_x_origin + col) as f32 * TILE_SIZE
                + TILE_SIZE * 0.5;
            commands.spawn(SpriteBundle {
                texture: sidewalk_tex.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::new(TILE_SIZE, MAP_HEIGHT)),
                    ..default()
                },
                transform: Transform::from_xyz(cx, 0.0, -18.0),
                ..default()
            });
        }
    }

    // ── Perimeter walls (with gaps from spawn points) ─────────────────────
    let perimeter_wall_tex = images.add(build_perimeter_wall_image());
    let (north_gaps, south_gaps, east_gaps, west_gaps) = collect_perimeter_gaps();
    let half_wt = WALL_THICK * 0.5;
    let perim_n_y = MAP_HEIGHT * 0.5 + half_wt;
    let perim_s_y = -MAP_HEIGHT * 0.5 - half_wt;
    let perim_e_x = MAP_WIDTH * 0.5 + half_wt;
    let perim_w_x = -MAP_WIDTH * 0.5 - half_wt;

    let mut perimeter_rects: Vec<(Vec2, Vec2)> = Vec::new();
    push_horizontal_wall(&mut perimeter_rects, perim_n_y, MAP_WIDTH * 0.5, half_wt, &north_gaps);
    push_horizontal_wall(&mut perimeter_rects, perim_s_y, MAP_WIDTH * 0.5, half_wt, &south_gaps);
    push_vertical_wall(&mut perimeter_rects, perim_e_x, MAP_HEIGHT * 0.5, half_wt, &east_gaps);
    push_vertical_wall(&mut perimeter_rects, perim_w_x, MAP_HEIGHT * 0.5, half_wt, &west_gaps);

    for &(pos, half) in &perimeter_rects {
        commands.spawn(SpriteBundle {
            texture: perimeter_wall_tex.clone(),
            sprite: Sprite {
                custom_size: Some(half * 2.0),
                ..default()
            },
            transform: Transform::from_xyz(pos.x, pos.y, -3.0),
            ..default()
        });
        obstacles.list.push(Obstacle {
            pos,
            shape: ObstacleShape::Rect(half),
        });
    }

    // ── Buildings: floor, walls (with door gap), roof, door, welcome mat ──
    // Wall / floor / door / furniture textures are pure functions of small
    // enums, so build each distinct one ONCE and share the `Handle` across
    // every building/floor/item that needs it instead of allocating hundreds
    // of byte-identical `Image` assets at startup.  The welcome mat and door
    // frame are fully constant, so they're hoisted out of the loop entirely.
    let welcome_tex = images.add(build_welcome_mat_image());
    let frame_tex = images.add(build_door_frame_image());
    let mut wall_tex_cache: std::collections::HashMap<BuildingType, Handle<Image>> =
        std::collections::HashMap::new();
    let mut floor_tex_cache: std::collections::HashMap<BuildingType, Handle<Image>> =
        std::collections::HashMap::new();
    let mut door_tex_cache: std::collections::HashMap<(BuildingType, WallSide), Handle<Image>> =
        std::collections::HashMap::new();
    let mut furn_tex_cache: std::collections::HashMap<FurnKind, Handle<Image>> =
        std::collections::HashMap::new();
    for (idx, b) in BUILDINGS.iter().enumerate() {
        let (center, half) = building_world_rect(b);
        let wall_tex = wall_tex_cache
            .entry(b.kind)
            .or_insert_with(|| images.add(build_building_wall_image(b.kind)))
            .clone();

        // Interior floors at z=-7 (above ground, below props/walls).  One
        // sprite per playable floor, all stacked at the same world coords
        // but tagged so visibility flips on staircase use.  Top floor of
        // multi-story buildings is rendered separately as the rooftop.
        let floor_half = Vec2::new(
            (half.x - BUILDING_WALL_THICK * 0.5).max(2.0),
            (half.y - BUILDING_WALL_THICK * 0.5).max(2.0),
        );
        let interior_count = building_floor_count(b.kind).saturating_sub(
            if has_roof_access(b.kind) { 1 } else { 0 },
        );
        let interior_count = interior_count.max(1);
        for floor in 0..interior_count {
            let floor_tex = floor_tex_cache
                .entry(b.kind)
                .or_insert_with(|| images.add(build_interior_floor_image(b.kind)))
                .clone();
            commands.spawn((
                SpriteBundle {
                    texture: floor_tex,
                    sprite: Sprite {
                        custom_size: Some(floor_half * 2.0),
                        ..default()
                    },
                    transform: Transform::from_xyz(center.x, center.y, -7.0),
                    ..default()
                },
                FloorEntity { building: idx, floor },
            ));
        }

        // Wall segments (4 sides, with gap on door side).  Each segment
        // becomes its own sprite + obstacle.  For multi-floor buildings
        // we stash the obstacle indices so they can be toggled passable
        // while the local player is on the roof.
        let mut walls: Vec<(Vec2, Vec2)> = Vec::new();
        push_building_walls(&mut walls, b);
        let mut wall_obstacle_ids: Vec<(usize, ObstacleShape)> = Vec::new();
        for (pos, half_seg) in walls {
            commands.spawn(SpriteBundle {
                texture: wall_tex.clone(),
                sprite: Sprite {
                    custom_size: Some(half_seg * 2.0),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, -5.0),
                ..default()
            });
            let shape = ObstacleShape::Rect(half_seg);
            let obs_idx = obstacles.list.len();
            obstacles.list.push(Obstacle { pos, shape });
            wall_obstacle_ids.push((obs_idx, shape));
        }
        if has_roof_access(b.kind) {
            wall_indices.walls.insert(idx, wall_obstacle_ids);
        }

        // Roof at z=6 — above zombies (z=5) but below the player (z=10).
        // The visibility-toggle system hides this when the local player
        // is inside the building so the interior shows through.
        let roof_tex = images.add(build_roof_image(b.kind, b.roof, b.w, b.h));
        commands.spawn((
            SpriteBundle {
                texture: roof_tex,
                sprite: Sprite {
                    custom_size: Some(half * 2.0),
                    ..default()
                },
                transform: Transform::from_xyz(center.x, center.y, 6.0),
                ..default()
            },
            BuildingRoof { idx },
        ));

        // ── Window glow decals on apartment + tower facades ────────────
        // Tagged with `BuildingRoof` so they vanish when the player walks
        // inside (they belong to the exterior shell).  Each window has its
        // own phase so the building reads as several independent rooms.
        if matches!(b.kind, BuildingType::Apartment | BuildingType::Tower) {
            spawn_building_windows(&mut commands, &mut images, idx, b);
        }

        // Door panel — fits the wall gap, lives BELOW the roof (z=-4.5)
        // so it doesn't float above the building from outside.  Visible
        // only when the player is inside (roof hidden).  External entry
        // signal comes from the door-frame markers below.
        let door_world = building_door_world(b);
        let side = building_door_side(b);
        let door_tex = door_tex_cache
            .entry((b.kind, side))
            .or_insert_with(|| images.add(build_door_image(b.kind, side)))
            .clone();
        let door_size = match side {
            WallSide::N | WallSide::S => Vec2::new(TILE_SIZE, BUILDING_WALL_THICK + 4.0),
            WallSide::E | WallSide::W => Vec2::new(BUILDING_WALL_THICK + 4.0, TILE_SIZE),
        };
        commands.spawn(SpriteBundle {
            texture: door_tex,
            sprite: Sprite {
                custom_size: Some(door_size),
                ..default()
            },
            transform: Transform::from_xyz(door_world.x, door_world.y, -4.5),
            ..default()
        });

        // External entry frame — two short side jambs flanking the door
        // tile, drawn ON the ground just outside the wall.  Subtle but
        // breaks up the wall silhouette so the player spots the entrance
        // without a giant floating panel.  (Texture hoisted above the loop.)
        let frame_offset = TILE_SIZE * 0.5 + 3.0;
        let (frame_pos, frame_size) = match side {
            WallSide::N => (
                Vec2::new(door_world.x, door_world.y + frame_offset),
                Vec2::new(TILE_SIZE * 1.2, 8.0),
            ),
            WallSide::S => (
                Vec2::new(door_world.x, door_world.y - frame_offset),
                Vec2::new(TILE_SIZE * 1.2, 8.0),
            ),
            WallSide::E => (
                Vec2::new(door_world.x + frame_offset, door_world.y),
                Vec2::new(8.0, TILE_SIZE * 1.2),
            ),
            WallSide::W => (
                Vec2::new(door_world.x - frame_offset, door_world.y),
                Vec2::new(8.0, TILE_SIZE * 1.2),
            ),
        };
        commands.spawn(SpriteBundle {
            texture: frame_tex.clone(),
            sprite: Sprite {
                custom_size: Some(frame_size),
                ..default()
            },
            transform: Transform::from_xyz(frame_pos.x, frame_pos.y, -2.4),
            ..default()
        });
        // Subtle welcome mat just past the frame — much smaller and
        // dimmer than before so it reads as a faint marker, not a flag.
        let mat_offset = TILE_SIZE * 0.95;
        let (mat_pos, mat_size) = match side {
            WallSide::N => (Vec2::new(door_world.x, door_world.y + mat_offset), Vec2::new(TILE_SIZE * 0.65, 8.0)),
            WallSide::S => (Vec2::new(door_world.x, door_world.y - mat_offset), Vec2::new(TILE_SIZE * 0.65, 8.0)),
            WallSide::E => (Vec2::new(door_world.x + mat_offset, door_world.y), Vec2::new(8.0, TILE_SIZE * 0.65)),
            WallSide::W => (Vec2::new(door_world.x - mat_offset, door_world.y), Vec2::new(8.0, TILE_SIZE * 0.65)),
        };
        commands.spawn(SpriteBundle {
            texture: welcome_tex.clone(),
            sprite: Sprite {
                custom_size: Some(mat_size),
                color: Color::srgba(0.85, 0.85, 0.85, 0.85),
                ..default()
            },
            transform: Transform::from_xyz(mat_pos.x, mat_pos.y, -2.3),
            ..default()
        });

        // Gas-station forecourt (canopy + 2 pumps) south of the store.
        if matches!(b.kind, BuildingType::Gas) {
            spawn_gas_forecourt(&mut commands, &mut images, b);
        }

        // Interior furniture per floor.  For multi-story buildings each
        // playable floor has its own sprite set + obstacle indices, so
        // collisions match what's currently rendered.
        let inner_half = Vec2::new(
            (half.x - BUILDING_WALL_THICK - 6.0).max(0.0),
            (half.y - BUILDING_WALL_THICK - 6.0).max(0.0),
        );
        let total_floors = building_floor_count(b.kind);
        let multi = has_roof_access(b.kind);
        for floor in 0..total_floors {
            for &(fk, dx, dy) in furniture_for_floor(b.kind, floor) {
                let furn_half = furniture_half(fk);
                if dx.abs() + furn_half.x > inner_half.x
                    || dy.abs() + furn_half.y > inner_half.y
                {
                    continue;
                }
                let pos = Vec2::new(center.x + dx, center.y + dy);
                let img = furn_tex_cache
                    .entry(fk)
                    .or_insert_with(|| images.add(build_furniture_image(fk)))
                    .clone();
                // Decorative items (rugs, paintings, plants, lamps) render
                // beneath solid furniture so a chair on a rug reads correctly.
                let z = if furniture_collides(fk) { -6.0 } else { -6.7 };
                commands.spawn((
                    SpriteBundle {
                        texture: img,
                        sprite: Sprite {
                            custom_size: Some(furn_half * 2.0),
                            ..default()
                        },
                        transform: Transform::from_xyz(pos.x, pos.y, z),
                        ..default()
                    },
                    FloorEntity { building: idx, floor },
                ));
                if !furniture_collides(fk) {
                    continue;
                }
                let shape = ObstacleShape::Rect(furn_half * 0.85);
                let obs_idx = obstacles.list.len();
                obstacles.list.push(Obstacle { pos, shape });
                if multi {
                    floor_obstacles
                        .by
                        .entry((idx, floor))
                        .or_default()
                        .push((obs_idx, shape));
                }
            }
        }

        // Multi-floor extras (Apartment, Hospital): staircase + rooftop
        // content spawned in the same loop so we have access to `idx`.
        if has_roof_access(b.kind) {
            spawn_multi_floor_extras(
                &mut commands,
                &mut images,
                &mut obstacles,
                &mut floor_obstacles,
                idx,
                b,
            );
        }
    }

    // ── Props per segment ─────────────────────────────────────────────────
    // ~700 props collapse to ~30 distinct kinds; cache the texture per kind so
    // we don't `images.add` hundreds of identical tree/bush/etc. images.
    let mut prop_tex_cache: std::collections::HashMap<PropKind, Handle<Image>> =
        std::collections::HashMap::new();
    for p in PROPS {
        let center = prop_world_center(p);
        let img = prop_tex_cache
            .entry(p.kind)
            .or_insert_with(|| images.add(build_prop_image(p.kind)))
            .clone();
        let size = prop_size_px(p);
        let z = prop_z(p.kind);
        let prop_entity = commands
            .spawn((
                SpriteBundle {
                    texture: img,
                    sprite: Sprite {
                        custom_size: Some(size),
                        ..default()
                    },
                    transform: Transform::from_xyz(center.x, center.y, z),
                    ..default()
                },
                MapDecor,
            ))
            .id();
        let mut obs_idx: Option<usize> = None;
        if let Some(shape) = prop_collision(p) {
            obs_idx = Some(obstacles.list.len());
            obstacles.list.push(Obstacle { pos: center, shape });
        }
        // Vehicle props become destructible explodables — bus pile, mil
        // trucks, jeeps and ambulances all wreck when shot, chain-igniting
        // crowds of zombies for cinematic kills.
        if let (Some(expl), Some(idx)) = (vehicle_explodable_for(p.kind), obs_idx) {
            commands
                .entity(prop_entity)
                .insert((expl, ExplodableObstacleIdx(idx)));
        }
        // Streetlights flicker procedurally — adds atmosphere to the
        // post-apo blackout vibe.  Phase derived from world position so
        // adjacent lamps don't blink in sync.
        if matches!(p.kind, PropKind::Lamp) {
            let phase = (center.x * 0.013 + center.y * 0.029) % std::f32::consts::TAU;
            commands.entity(prop_entity).insert(LampFlicker { phase });
        }
    }

    // ── Explodables (destructible car wrecks + fuel barrels) ─────────────
    // These have HP and can be destroyed by gunfire, chaining explosions
    // through grouped enemies.  Their obstacle entry is tracked so we can
    // remove it from the resolver once they detonate (otherwise the world
    // would still block movement at the wreck's old position).
    let mut car_wreck_tex: Option<Handle<Image>> = None;
    let mut fuel_barrel_tex: Option<Handle<Image>> = None;
    for spec in EXPLODABLES {
        let center = explodable_world_center(spec);
        let img = match spec.kind {
            ExplodableVisualKind::CarWreck => car_wreck_tex
                .get_or_insert_with(|| images.add(build_explodable_car_wreck_image()))
                .clone(),
            ExplodableVisualKind::FuelBarrel => fuel_barrel_tex
                .get_or_insert_with(|| images.add(build_explodable_fuel_barrel_image()))
                .clone(),
        };
        let half = spec.kind.collision_half();
        let obs_idx = obstacles.list.len();
        obstacles.list.push(Obstacle {
            pos: center,
            shape: ObstacleShape::Rect(half),
        });
        commands.spawn((
            SpriteBundle {
                texture: img,
                sprite: Sprite {
                    custom_size: Some(spec.kind.sprite_size()),
                    ..default()
                },
                transform: Transform::from_xyz(center.x, center.y, -1.0),
                ..default()
            },
            spec.kind.default_spec(),
            ExplodableObstacleIdx(obs_idx),
        ));
    }

    // ── Spawn-point markers (visual only) ─────────────────────────────────
    let board_tex = images.add(build_board_image());
    for sp in SPAWN_POINTS {
        let pos = tile_center(sp.tile.0, sp.tile.1);
        let (size, z) = match sp.side {
            WallSide::N | WallSide::S => (Vec2::new(sp.span_tiles as f32 * TILE_SIZE, 12.0), -2.5),
            WallSide::E | WallSide::W => (Vec2::new(12.0, sp.span_tiles as f32 * TILE_SIZE), -2.5),
        };
        let offset = TILE_SIZE * 0.5;
        let shifted = match sp.side {
            WallSide::N => pos + Vec2::new(0.0, offset),
            WallSide::S => pos - Vec2::new(0.0, offset),
            WallSide::E => pos + Vec2::new(offset, 0.0),
            WallSide::W => pos - Vec2::new(offset, 0.0),
        };
        commands.spawn((
            SpriteBundle {
                texture: board_tex.clone(),
                sprite: Sprite {
                    custom_size: Some(size),
                    ..default()
                },
                transform: Transform::from_xyz(shifted.x, shifted.y, z),
                ..default()
            },
            MapDecor,
        ));
    }

    // ── Gates + per-segment fog overlays ──────────────────────────────────
    let fog_tex = images.add(build_segment_fog_image());
    spawn_segment_fog_and_gates(&mut commands, &mut images, &mut obstacles, fog_tex);

    // ── Overcast atmosphere overlay (whole map, heavier) ──────────────────
    // Two layers stacked: a darker base tint (whole world dimmed by a deep
    // navy at ~38% strength) and a lighter mottled "stormcloud" mask on
    // top with subtle vignetting at the edges.  Together they read as a
    // proper post-apocalyptic stormy afternoon.
    // On mobile, collapse the two stacked full-screen overlays into ONE
    // pre-blended quad — two full-viewport alpha blends per frame are a top
    // fill-rate cost on tiled, bandwidth-bound mobile GPUs.  Desktop keeps the
    // exact two-layer look (overdraw is free on a dGPU).
    if cfg!(any(target_os = "android", target_os = "ios")) {
        let atmo_tex = images.add(build_overcast_composite_image());
        commands.spawn(SpriteBundle {
            texture: atmo_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(MAP_WIDTH, MAP_HEIGHT)),
                ..default()
            },
            transform: Transform::from_xyz(0.0, 0.0, 20.0),
            ..default()
        });
    } else {
        let overcast_tex = images.add(build_overcast_image());
        commands.spawn(SpriteBundle {
            texture: overcast_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(MAP_WIDTH, MAP_HEIGHT)),
                color: Color::srgba(0.10, 0.12, 0.18, 0.38),
                ..default()
            },
            transform: Transform::from_xyz(0.0, 0.0, 20.0),
            ..default()
        });
        let storm_tex = images.add(build_stormcloud_image());
        commands.spawn(SpriteBundle {
            texture: storm_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(MAP_WIDTH, MAP_HEIGHT)),
                color: Color::srgba(0.18, 0.20, 0.26, 0.32),
                ..default()
            },
            transform: Transform::from_xyz(0.0, 0.0, 20.5),
            ..default()
        });
    }

    // Build the spatial-grid index now that every obstacle is in place.
    // Subsequent shape→Circle(0.0) transitions (toggle floor/roof, destroyed
    // explodables) don't need a rebuild — the grid stores indices, not shapes.
    obstacles.rebuild_grid();
}

fn build_stormcloud_image() -> Image {
    // Larger-scale grey blotches simulating drifting cloud cover.  Tints
    // are biased dark so the multiplied result reads "stormy".
    let mut c = Canvas::new(128, 128);
    for y in 0..128 {
        for x in 0..128 {
            let n = (x * 5 + y * 7) % 13 + (x * 11 + y * 3) % 17;
            let v = (140 + n * 4 - 30).clamp(80, 220) as u8;
            c.put(x, y, [v, v, v.saturating_add(12), 255]);
        }
    }
    // Heavier patches sprinkled across.
    for (cx, cy, r) in [
        (24i32, 30i32, 18i32),
        (80, 22, 24),
        (40, 100, 22),
        (104, 96, 20),
        (62, 64, 28),
    ] {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    let x = cx + dx;
                    let y = cy + dy;
                    if (0..128).contains(&x) && (0..128).contains(&y) {
                        c.put(x, y, [40, 44, 56, 255]);
                    }
                }
            }
        }
    }
    c.into_image()
}

fn build_overcast_image() -> Image {
    // Gentle grey-blue noise — small variation per tile so the overlay
    // doesn't read as a flat film.  Alpha is set on the sprite at spawn.
    let mut c = Canvas::new(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            let n = (x * 7 + y * 11) % 9;
            let v = (180 + n * 4) as u8;
            c.put(x, y, [v, v, v.saturating_add(8), 255]);
        }
    }
    c.into_image()
}

/// Mobile-only: pre-blend the overcast tint + stormcloud mask into ONE texture
/// so the atmosphere is a single full-map quad instead of two stacked
/// alpha-blended ones.  Collapses the two "over" blends — overcast (a1=0.38)
/// then stormcloud (a2=0.32) — into a single source: constant alpha `ac` and
/// the colour back-premultiplied by `ac`, which reproduces the two-layer blend.
fn build_overcast_composite_image() -> Image {
    const A1: f32 = 0.38;
    const A2: f32 = 0.32;
    let ac = 1.0 - (1.0 - A1) * (1.0 - A2); // 0.5784
    let tint1 = [0.10f32, 0.12, 0.18];
    let tint2 = [0.18f32, 0.20, 0.26];
    let mut c = Canvas::new(128, 128);
    for y in 0..128i32 {
        for x in 0..128i32 {
            // Overcast source is 64x64; both layers stretch across the whole map,
            // so sample it at (x/2, y/2) to keep the UVs aligned.
            let (ox, oy) = (x / 2, y / 2);
            let n1 = (ox * 7 + oy * 11) % 9;
            let v1 = (180 + n1 * 4) as f32;
            let layer1 = [v1, v1, (v1 + 8.0).min(255.0)];
            // Stormcloud source (128x128): base noise + heavy circular patches.
            let n = (x * 5 + y * 7) % 13 + (x * 11 + y * 3) % 17;
            let base = (140 + n * 4 - 30).clamp(80, 220) as f32;
            let mut layer2 = [base, base, (base + 12.0).min(255.0)];
            for (cx, cy, r) in [(24i32, 30i32, 18i32), (80, 22, 24), (40, 100, 22), (104, 96, 20), (62, 64, 28)] {
                let (dx, dy) = (x - cx, y - cy);
                if dx * dx + dy * dy <= r * r {
                    layer2 = [40.0, 44.0, 56.0];
                }
            }
            let mut px = [0u8; 4];
            for ch in 0..3 {
                let c1 = layer1[ch] / 255.0 * tint1[ch];
                let c2 = layer2[ch] / 255.0 * tint2[ch];
                let acc = c1 * A1 * (1.0 - A2) + c2 * A2;
                px[ch] = (acc / ac * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            px[3] = (ac * 255.0).round() as u8;
            c.put(x, y, px);
        }
    }
    c.into_image()
}

fn spawn_segment_fog_and_gates(
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    obstacles: &mut ResMut<MapObstacles>,
    fog_tex: Handle<Image>,
) {
    // Fog covering each locked segment.  Segment 1 is always unlocked, so
    // skip it.
    for seg in SEGMENTS.iter() {
        if seg.id == 1 {
            continue;
        }
        let origin_x = -MAP_WIDTH * 0.5 + segment_origin_x(seg.id) as f32 * TILE_SIZE
            + SEG_WIDTH * 0.5;
        // A locked-segment fog quad can cover the whole viewport at a frontier —
        // a third full-screen blend layer.  On mobile render it opaque (it
        // already reads as a near-solid grey wall at 0.78) so a tiled GPU skips
        // that blend; desktop keeps the translucent look.
        let fog_alpha = if cfg!(any(target_os = "android", target_os = "ios")) {
            1.0
        } else {
            0.78
        };
        commands.spawn((
            SpriteBundle {
                texture: fog_tex.clone(),
                sprite: Sprite {
                    custom_size: Some(Vec2::new(SEG_WIDTH, MAP_HEIGHT)),
                    color: Color::srgba(0.18, 0.19, 0.20, fog_alpha),
                    ..default()
                },
                transform: Transform::from_xyz(origin_x, 0.0, SEGMENT_FOG_Z),
                ..default()
            },
            SegmentFog { idx: seg.id },
        ));
    }

    for gate in GATES {
        let pos = gate_world_pos(gate.from_seg);
        let half = Vec2::new(8.0, MAP_HEIGHT * 0.5);
        let obstacle_idx = obstacles.list.len();
        obstacles.list.push(Obstacle {
            pos,
            shape: ObstacleShape::Rect(half),
        });
        let visual_tex = images.add(build_gate_image(gate.kind));
        commands.spawn((
            SpriteBundle {
                texture: visual_tex,
                sprite: Sprite {
                    custom_size: Some(Vec2::new(48.0, MAP_HEIGHT)),
                    ..default()
                },
                transform: Transform::from_xyz(pos.x, pos.y, GATE_BARRIER_Z),
                ..default()
            },
            GateBarrier {
                from_seg: gate.from_seg,
                to_seg: gate.to_seg,
                obstacle_idx,
            },
        ));
    }
}

fn spawn_gas_forecourt(
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    b: &Building,
) {
    let (center, _) = building_world_rect(b);
    // Canopy 1 tile south of building bottom edge.
    let canopy_w = b.w as f32 * TILE_SIZE;
    let canopy_h = TILE_SIZE * 3.0;
    let canopy_y = center.y - b.h as f32 * TILE_SIZE * 0.5 - canopy_h * 0.5 - 8.0;
    let canopy_tex = images.add(build_gas_canopy_image());
    commands.spawn(SpriteBundle {
        texture: canopy_tex,
        sprite: Sprite {
            custom_size: Some(Vec2::new(canopy_w, canopy_h)),
            ..default()
        },
        transform: Transform::from_xyz(center.x, canopy_y, -1.5),
        ..default()
    });
    // Two pumps under the canopy.
    let pump_tex = images.add(build_gas_pump_image());
    for &dx in &[-canopy_w * 0.25, canopy_w * 0.25] {
        commands.spawn(SpriteBundle {
            texture: pump_tex.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(20.0, 28.0)),
                ..default()
            },
            transform: Transform::from_xyz(center.x + dx, canopy_y, -1.0),
            ..default()
        });
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Segment unlock systems (port of previous gate-buy mechanic)
// ════════════════════════════════════════════════════════════════════════

fn reset_segment_state(
    mut state: ResMut<MapSegmentUnlockState>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut obstacles: ResMut<MapObstacles>,
    fog: Query<Entity, With<SegmentFog>>,
    barriers: Query<Entity, With<GateBarrier>>,
) {
    *state = MapSegmentUnlockState::default();

    for entity in fog.iter() {
        commands.entity(entity).despawn();
    }
    for entity in barriers.iter() {
        commands.entity(entity).despawn();
    }
    for gate in GATES {
        obstacles.remove_at(gate_world_pos(gate.from_seg));
    }

    let fog_tex = images.add(build_segment_fog_image());
    spawn_segment_fog_and_gates(&mut commands, &mut images, &mut obstacles, fog_tex);
    obstacles.rebuild_grid();
}

fn update_segment_fog_visibility(
    mut commands: Commands,
    state: Res<MapSegmentUnlockState>,
    mut obstacles: ResMut<MapObstacles>,
    fog: Query<(Entity, &SegmentFog)>,
    barriers: Query<(Entity, &GateBarrier)>,
) {
    for (entity, f) in fog.iter() {
        if state.is_unlocked(f.idx) {
            commands.entity(entity).despawn();
        }
    }
    for (entity, b) in barriers.iter() {
        if state.is_unlocked(b.to_seg) {
            commands.entity(entity).despawn();
            if let Some(obs) = obstacles.list.get_mut(b.obstacle_idx) {
                obs.shape = ObstacleShape::Circle(0.0);
            }
        }
    }
}

fn spawn_multi_floor_extras(
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    obstacles: &mut ResMut<MapObstacles>,
    floor_obstacles: &mut ResMut<FloorObstacleIndices>,
    idx: usize,
    b: &Building,
) {
    let (center, half) = building_world_rect(b);
    let inner_half = Vec2::new(
        (half.x - BUILDING_WALL_THICK).max(8.0),
        (half.y - BUILDING_WALL_THICK).max(8.0),
    );
    let roof_floor = top_floor(b.kind);

    // ── Interior partition walls for every residential floor ──
    // These split the inner volume into four mieszkania around a central
    // corridor on each upper floor.  Each floor gets its own copy of the
    // walls (with FloorEntity tag) so they're visible/collidable only
    // while the player is on that exact floor.  Floor 0 (lobby) and the
    // top floor (rooftop) stay open.
    if matches!(b.kind, BuildingType::Apartment | BuildingType::Hospital) {
        let interior_wall_tex = images.add(build_interior_partition_image(b.kind));
        let walls = residential_floor_walls(b);
        for floor in 1..roof_floor {
            for &(offset, wall_half) in &walls {
                let pos = Vec2::new(center.x + offset.x, center.y + offset.y);
                commands.spawn((
                    SpriteBundle {
                        texture: interior_wall_tex.clone(),
                        sprite: Sprite {
                            custom_size: Some(wall_half * 2.0),
                            ..default()
                        },
                        transform: Transform::from_xyz(pos.x, pos.y, -5.5),
                        visibility: Visibility::Hidden,
                        ..default()
                    },
                    FloorEntity {
                        building: idx,
                        floor,
                    },
                ));
                let shape = ObstacleShape::Rect(wall_half);
                let obs_idx = obstacles.list.len();
                obstacles.list.push(Obstacle { pos, shape });
                floor_obstacles
                    .by
                    .entry((idx, floor))
                    .or_default()
                    .push((obs_idx, shape));
            }
        }
    }

    // ── Staircase prop (visible on every floor — no FloorEntity tag) ──
    let stair_pos = staircase_world_pos(b);
    let stair_tex = images.add(build_staircase_image());
    commands.spawn((
        SpriteBundle {
            texture: stair_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(TILE_SIZE * 0.95, TILE_SIZE * 1.4)),
                ..default()
            },
            transform: Transform::from_xyz(stair_pos.x, stair_pos.y, -3.5),
            ..default()
        },
        Staircase { building: idx },
    ));

    // ── Rooftop floor (concrete) — visible only when player on roof ──
    let rooftop_floor_tex = images.add(build_rooftop_floor_image());
    commands.spawn((
        SpriteBundle {
            texture: rooftop_floor_tex,
            sprite: Sprite {
                custom_size: Some(inner_half * 2.0),
                ..default()
            },
            transform: Transform::from_xyz(center.x, center.y, -6.5),
            visibility: Visibility::Hidden,
            ..default()
        },
        FloorEntity { building: idx, floor: roof_floor },
    ));

    // ── Rooftop decor: HVAC + antenna + vent (only the HVAC blocks).  ──
    // HVAC obstacle is registered into floor_obstacles so it only collides
    // while the player is actually standing on the roof.
    let hvac_tex = images.add(build_hvac_image());
    let antenna_tex = images.add(build_antenna_image());
    let vent_tex = images.add(build_roof_vent_image());

    let hvac_pos = Vec2::new(center.x - inner_half.x * 0.4, center.y);
    commands.spawn((
        SpriteBundle {
            texture: hvac_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(48.0, 36.0)),
                ..default()
            },
            transform: Transform::from_xyz(hvac_pos.x, hvac_pos.y, -4.0),
            visibility: Visibility::Hidden,
            ..default()
        },
        FloorEntity { building: idx, floor: roof_floor },
    ));
    let hvac_shape = ObstacleShape::Rect(Vec2::new(20.0, 14.0));
    let hvac_obs_idx = obstacles.list.len();
    obstacles.list.push(Obstacle {
        pos: hvac_pos,
        shape: hvac_shape,
    });
    floor_obstacles
        .by
        .entry((idx, roof_floor))
        .or_default()
        .push((hvac_obs_idx, hvac_shape));

    let antenna_pos = Vec2::new(center.x + inner_half.x * 0.5, center.y - inner_half.y * 0.3);
    commands.spawn((
        SpriteBundle {
            texture: antenna_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(20.0, 48.0)),
                ..default()
            },
            transform: Transform::from_xyz(antenna_pos.x, antenna_pos.y, -3.5),
            visibility: Visibility::Hidden,
            ..default()
        },
        FloorEntity { building: idx, floor: roof_floor },
    ));

    let vent_pos = Vec2::new(center.x + inner_half.x * 0.2, center.y + inner_half.y * 0.4);
    commands.spawn((
        SpriteBundle {
            texture: vent_tex,
            sprite: Sprite {
                custom_size: Some(Vec2::new(28.0, 20.0)),
                ..default()
            },
            transform: Transform::from_xyz(vent_pos.x, vent_pos.y, -4.0),
            visibility: Visibility::Hidden,
            ..default()
        },
        FloorEntity { building: idx, floor: roof_floor },
    ));
}

fn reset_player_floor_state(
    mut state: ResMut<PlayerFloorState>,
    mut cooldown: ResMut<StairCooldown>,
) {
    *state = PlayerFloorState::default();
    cooldown.0 = 0.0;
}

fn track_player_building_floor(
    mut state: ResMut<PlayerFloorState>,
    players: Query<(&Transform, &Player)>,
    ctx: Res<NetContext>,
) {
    let local_pos = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .or_else(|| players.iter().next())
        .map(|(t, _)| t.translation.truncate());
    let Some(p) = local_pos else {
        return;
    };

    // Find which (multi-floor) building the local player is currently in.
    let mut new_inside: Option<usize> = None;
    for (idx, b) in BUILDINGS.iter().enumerate() {
        if !has_roof_access(b.kind) {
            continue;
        }
        let (center, half) = building_world_rect(b);
        let margin = BUILDING_WALL_THICK * 0.5;
        if (p.x - center.x).abs() < half.x - margin
            && (p.y - center.y).abs() < half.y - margin
        {
            new_inside = Some(idx);
            break;
        }
    }

    match (state.building, new_inside) {
        (Some(_), None) => {
            // Left a building → reset to default ground state.
            *state = PlayerFloorState::default();
        }
        (Some(old), Some(new)) if old != new => {
            // Stepped through one multi-floor building into another.
            state.building = Some(new);
            state.floor = 0;
        }
        (None, Some(new)) => {
            state.building = Some(new);
            state.floor = 0;
        }
        _ => {}
    }
}

fn update_floor_entity_visibility(
    state: Res<PlayerFloorState>,
    mut entities: Query<(&FloorEntity, &mut Visibility)>,
) {
    // Floor visibility only changes when the player changes floor/building.
    // `PlayerFloorState` is now mutated only on real transitions (the per-tick
    // stair cooldown lives in its own resource), so this gate skips the full
    // entity sweep on the vast majority of frames.
    if !state.is_changed() {
        return;
    }
    for (fe, mut vis) in entities.iter_mut() {
        let want_visible = match state.building {
            Some(b) if b == fe.building => state.floor == fe.floor,
            // Player is outside this building: only ground floor entities
            // are visible (rooftop content stays hidden).
            _ => fe.floor == 0,
        };
        *vis = if want_visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Distance at which an E-press triggers the staircase.  Generous so
/// players don't have to pixel-hunt for the exact tile.
const STAIR_INTERACT_RADIUS: f32 = 48.0;
/// Auto-trigger radius — narrower so the player can step *near* the stair
/// without warping floors.  Stepping ON it fires the auto-cycle.
const STAIR_AUTO_RADIUS: f32 = 22.0;
/// Seconds the cooldown holds after a floor change so the player can step
/// off the stair tile without immediately being warped back up.
const STAIR_COOLDOWN_AFTER_CHANGE: f32 = 1.2;

fn staircase_interact(
    time: Res<Time>,
    mut state: ResMut<PlayerFloorState>,
    mut cooldown: ResMut<StairCooldown>,
    mut local: ResMut<crate::net::LocalInput>,
    players: Query<(&Transform, &Player)>,
    stairs: Query<(&Staircase, &Transform)>,
    ctx: Res<NetContext>,
) {
    cooldown.0 = (cooldown.0 - time.delta_seconds()).max(0.0);

    let local_pos = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .or_else(|| players.iter().next())
        .map(|(t, _)| t.translation.truncate());
    let Some(p) = local_pos else {
        return;
    };

    let pressed = local.0.interact;

    for (s, t) in stairs.iter() {
        // Only the staircase belonging to the building the player is
        // currently inside should respond.
        if state.building != Some(s.building) {
            continue;
        }
        let stair_pos = t.translation.truncate();
        let dist_sq = p.distance_squared(stair_pos);

        // E-press: explicit floor advance from anywhere within the
        // generous interact radius.
        if pressed && dist_sq <= STAIR_INTERACT_RADIUS * STAIR_INTERACT_RADIUS {
            let kind = BUILDINGS[s.building].kind;
            let count = building_floor_count(kind);
            state.floor = (state.floor + 1) % count;
            cooldown.0 = STAIR_COOLDOWN_AFTER_CHANGE;
            local.0.interact = false;
            return;
        }

        // Auto-cycle: stepping directly onto the stair tile bumps you up
        // to the next floor automatically (with cooldown so you don't
        // teleport every tick while standing on it).
        if dist_sq <= STAIR_AUTO_RADIUS * STAIR_AUTO_RADIUS && cooldown.0 <= 0.0 {
            let kind = BUILDINGS[s.building].kind;
            let count = building_floor_count(kind);
            state.floor = (state.floor + 1) % count;
            cooldown.0 = STAIR_COOLDOWN_AFTER_CHANGE;
            return;
        }
    }
}

fn toggle_roof_walls(
    state: Res<PlayerFloorState>,
    indices: Res<BuildingWallIndices>,
    mut obstacles: ResMut<MapObstacles>,
) {
    // Wall passability only changes on a floor/building transition; skip the
    // ResMut<MapObstacles> acquisition + writes on unchanged frames.
    if !state.is_changed() {
        return;
    }
    // Each frame, walls of the building the local player is on roof of
    // become non-collision (so the player can walk off the edge).  Walls
    // of every other multi-floor building are restored to their original
    // shape.
    for (b_idx, walls) in indices.walls.iter() {
        let kind = BUILDINGS[*b_idx].kind;
        let active = state.building == Some(*b_idx) && state.floor == top_floor(kind);
        for (obs_idx, original) in walls {
            if let Some(o) = obstacles.list.get_mut(*obs_idx) {
                o.shape = if active {
                    ObstacleShape::Circle(0.0)
                } else {
                    *original
                };
            }
        }
    }
}

/// Per-floor furniture obstacle toggle.  Mirrors `update_floor_entity_visibility`
/// but on the collision side: an obstacle is active only when the local
/// player is on the matching floor (or, for floor 0, when they're outside
/// any of the multi-floor building it belongs to).
fn toggle_floor_obstacles(
    state: Res<PlayerFloorState>,
    indices: Res<FloorObstacleIndices>,
    mut obstacles: ResMut<MapObstacles>,
) {
    // Per-floor furniture solidity only changes on a floor/building transition.
    if !state.is_changed() {
        return;
    }
    for ((b_idx, floor), items) in indices.by.iter() {
        let active = if *floor == 0 {
            // Ground-floor furniture stays solid as long as the player
            // isn't *upstairs* in this same building.
            state.building != Some(*b_idx) || state.floor == 0
        } else {
            state.building == Some(*b_idx) && state.floor == *floor
        };
        for (obs_idx, original) in items {
            if let Some(o) = obstacles.list.get_mut(*obs_idx) {
                o.shape = if active {
                    *original
                } else {
                    ObstacleShape::Circle(0.0)
                };
            }
        }
    }
}

fn check_roof_fall(
    mut state: ResMut<PlayerFloorState>,
    mut players: Query<(&mut Transform, &Player)>,
    mut dmg: EventWriter<crate::player::PlayerDamagedEvent>,
    ctx: Res<NetContext>,
) {
    // Only the local player can fall — roof state is per-client.
    let Some(b_idx) = state.building else {
        return;
    };
    let Some(b) = BUILDINGS.get(b_idx) else {
        return;
    };
    if state.floor != top_floor(b.kind) {
        return;
    }
    let (center, half) = building_world_rect(b);

    // While on the roof, the building's walls are toggled passable, so
    // the player can walk straight off the edge.  Fire as soon as their
    // centre is one tile past the wall outer face — far enough that the
    // visual reads as "I just stepped off", not flickering at the doorway.
    let leave_margin = -TILE_SIZE * 0.6;
    for (mut t, p) in players.iter_mut() {
        if p.id != ctx.my_id {
            continue;
        }
        let pp = t.translation.truncate();
        let outside = (pp.x - center.x).abs() > half.x - leave_margin
            || (pp.y - center.y).abs() > half.y - leave_margin;
        if outside {
            // Splat — reset to ground at the staircase, take a chunk of HP.
            dmg.send(crate::player::PlayerDamagedEvent {
                target_id: p.id,
                amount: 60,
            });
            let stair = staircase_world_pos(b);
            t.translation.x = stair.x;
            t.translation.y = stair.y;
            state.floor = 0;
        }
        break;
    }
}

fn update_building_roof_visibility(
    mut roofs: Query<(&BuildingRoof, &mut Sprite, &mut Visibility)>,
    players: Query<(&Transform, &Player)>,
    ctx: Res<NetContext>,
    time: Res<Time>,
) {
    let local_pos = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .or_else(|| players.iter().next())
        .map(|(t, _)| t.translation.truncate());
    let dt = time.delta_seconds();
    for (roof, mut sprite, mut vis) in roofs.iter_mut() {
        let Some(b) = BUILDINGS.get(roof.idx) else {
            continue;
        };
        let (center, half) = building_world_rect(b);
        // Margin = wall thickness so the cutout only kicks in once the
        // player is fully past the wall midline (no flicker at the door).
        let margin = BUILDING_WALL_THICK;
        let inside = local_pos
            .map(|p| {
                (p.x - center.x).abs() < half.x - margin
                    && (p.y - center.y).abs() < half.y - margin
            })
            .unwrap_or(false);
        // Smooth alpha tween: roof never hard-disappears.  When the player
        // is inside, drop to ~0 alpha so the interior reads cleanly; when
        // outside, return to full opacity.  Decay rate is fast enough that
        // walking past the threshold feels responsive (~0.25 s).
        let target_alpha: f32 = if inside { 0.0 } else { 1.0 };
        let mut color = sprite.color.to_srgba();
        let cur = color.alpha;
        // Roof already settled at its target alpha — skip the sprite/visibility
        // writes so a resting roof (the common case: player nowhere near it)
        // doesn't dirty the Sprite and force a renderer re-extract every frame.
        // Identical visual result; the tween below is a no-op at the target.
        if (target_alpha - cur).abs() < 1e-3 {
            continue;
        }
        let step = (target_alpha - cur).clamp(-dt * 4.0, dt * 4.0);
        color.alpha = (cur + step).clamp(0.0, 1.0);
        sprite.color = color.into();
        // Bevy still needs Visibility::Inherited so the sprite renders at
        // all — we never set it to Hidden any more.
        *vis = Visibility::Inherited;
    }
}

fn refresh_segment_unlock_hint(
    state: Res<MapSegmentUnlockState>,
    score: Res<crate::Score>,
    mut hint: ResMut<SegmentUnlockHint>,
    players: Query<(&Transform, &Player)>,
    ctx: Res<NetContext>,
) {
    let local_pos = players
        .iter()
        .find(|(_, p)| p.id == ctx.my_id)
        .or_else(|| players.iter().next())
        .map(|(t, _)| t.translation.truncate());
    let Some(p) = local_pos else {
        *hint = SegmentUnlockHint::default();
        return;
    };
    let mut best: Option<(u8, u32, f32)> = None;
    let max_d2 = (SEGMENT_UNLOCK_RADIUS * 1.5).powi(2);
    for gate in GATES {
        if state.is_unlocked(gate.to_seg) {
            continue;
        }
        let bp = gate_world_pos(gate.from_seg);
        let d2 = p.distance_squared(bp);
        if d2 <= max_d2 && best.map(|(_, _, bd2)| d2 < bd2).unwrap_or(true) {
            best = Some((gate.to_seg, gate.cost, d2));
        }
    }
    match best {
        Some((idx, cost, _)) => {
            hint.segment_idx = Some(idx);
            hint.cost = cost;
            hint.affordable = score.0 >= cost;
        }
        None => *hint = SegmentUnlockHint::default(),
    }
}

#[allow(clippy::too_many_arguments)]
fn unlock_segments_by_input(
    mut state: ResMut<MapSegmentUnlockState>,
    mut score: ResMut<crate::Score>,
    players: Query<(&Transform, &Player)>,
    mut local: ResMut<crate::net::LocalInput>,
    mut remote: ResMut<crate::net::RemoteInputs>,
    ctx: Res<NetContext>,
    net: Res<crate::net::NetMode>,
) {
    if matches!(*net, crate::net::NetMode::Client) {
        return;
    }
    for gate in GATES {
        if state.is_unlocked(gate.to_seg) {
            continue;
        }
        if score.0 < gate.cost {
            continue;
        }
        let pos = gate_world_pos(gate.from_seg);
        for (t, p) in players.iter() {
            let pp = t.translation.truncate();
            if pp.distance_squared(pos) > SEGMENT_UNLOCK_RADIUS * SEGMENT_UNLOCK_RADIUS {
                continue;
            }
            let pressed = if p.id == ctx.my_id {
                local.0.interact
            } else {
                remote.0.get(&p.id).map(|i| i.interact).unwrap_or(false)
            };
            if !pressed {
                continue;
            }
            score.0 = score.0.saturating_sub(gate.cost);
            state.unlock(gate.to_seg);
            if p.id == ctx.my_id {
                local.0.interact = false;
            } else if let Some(input) = remote.0.get_mut(&p.id) {
                input.interact = false;
            }
            break;
        }
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Pixel-art builders
// ════════════════════════════════════════════════════════════════════════

fn theme_grass_palette(theme: Theme) -> (Rgba, Rgba, Rgba) {
    // Heavier overcast / post-apocalyptic palette: more desaturation, hint
    // of cool blue-grey to read as "stormy afternoon" rather than summer
    // afternoon.  Each value reduced ~25-30% from the prototype.
    match theme {
        // base, dark tuft, light tuft
        Theme::Suburb => ([36, 60, 30, 255], [22, 42, 20, 255], [66, 92, 46, 255]),
        Theme::Downtown => ([46, 50, 44, 255], [30, 34, 28, 255], [70, 74, 60, 255]),
        Theme::Industrial => ([56, 46, 30, 255], [34, 28, 18, 255], [86, 72, 48, 255]),
        Theme::Hospital => ([40, 64, 44, 255], [24, 46, 28, 255], [70, 100, 64, 255]),
        Theme::Military => ([48, 56, 32, 255], [30, 38, 18, 255], [76, 88, 50, 255]),
    }
}

fn build_grass_image(theme: Theme) -> Image {
    let (base, dark, light) = theme_grass_palette(theme);
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, base);
    for &(x, y) in &[
        (2, 3), (5, 2), (8, 5), (11, 3), (14, 6), (17, 4), (20, 2),
        (23, 5), (26, 3), (29, 6), (3, 10), (7, 13), (10, 11),
        (13, 14), (16, 12), (19, 10), (22, 13), (25, 11), (28, 14),
        (1, 18), (4, 20), (8, 19), (12, 22), (15, 20), (18, 19),
        (21, 22), (24, 20), (27, 19), (30, 22), (2, 26), (6, 28),
        (9, 27), (13, 29), (16, 27), (20, 28), (24, 26), (28, 29),
    ] {
        c.put(x, y, dark);
    }
    for &(x, y) in &[
        (3, 2), (12, 4), (22, 11), (7, 21), (25, 27), (17, 30),
        (4, 14), (15, 7), (28, 20),
    ] {
        c.put(x, y, light);
    }
    c.into_image()
}

fn build_road_image() -> Image {
    let base: Rgba = [52, 52, 56, 255];
    let dark: Rgba = [40, 40, 44, 255];
    let light: Rgba = [66, 66, 70, 255];
    let stain: Rgba = [22, 20, 20, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, base);
    for &(x, y) in &[
        (3, 2), (9, 5), (16, 3), (22, 6), (28, 4), (4, 11),
        (13, 9), (20, 13), (27, 10), (6, 17), (12, 19), (18, 16),
        (24, 20), (30, 18), (2, 24), (9, 27), (15, 25), (21, 28),
        (27, 26), (5, 30), (14, 31),
    ] {
        c.put(x, y, dark);
    }
    for &(x, y) in &[
        (6, 7), (14, 14), (24, 6), (8, 22), (22, 24), (3, 15),
        (29, 13),
    ] {
        c.put(x, y, light);
    }
    for &(x, y) in &[(10, 2), (20, 10), (4, 20), (26, 28)] {
        c.put(x, y, stain);
    }
    c.into_image()
}

fn build_sidewalk_image() -> Image {
    let base: Rgba = [150, 148, 142, 255];
    let dark: Rgba = [118, 116, 110, 255];
    let light: Rgba = [176, 172, 164, 255];
    let crack: Rgba = [84, 82, 76, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, base);
    for px in 0..4 {
        for py in 0..2 {
            let ox = px * 8;
            let oy = py * 16;
            c.fill_rect(ox, oy, 8, 1, dark);
            c.fill_rect(ox, oy, 1, 16, dark);
            c.fill_rect(ox + 1, oy + 1, 6, 1, light);
        }
    }
    for &(x, y) in &[(4, 8), (5, 8), (6, 9), (18, 4), (19, 4), (20, 5), (25, 22), (26, 22)] {
        c.put(x, y, crack);
    }
    c.into_image()
}

fn build_perimeter_wall_image() -> Image {
    let base: Rgba = [98, 90, 70, 255];
    let dark: Rgba = [58, 52, 40, 255];
    let light: Rgba = [140, 128, 98, 255];
    let rust: Rgba = [122, 64, 32, 255];
    let mut c = Canvas::new(16, 16);
    c.fill_rect(0, 0, 16, 16, base);
    for i in 0..8 {
        let x = i * 2;
        c.fill_rect(x, 0, 1, 16, dark);
        c.fill_rect(x + 1, 0, 1, 16, light);
    }
    for &(x, y) in &[(3, 4), (10, 11), (6, 13)] {
        c.put(x, y, rust);
    }
    c.into_image()
}

fn build_board_image() -> Image {
    let dark: Rgba = [38, 24, 14, 255];
    let mid: Rgba = [78, 50, 30, 255];
    let light: Rgba = [120, 78, 46, 255];
    let nail: Rgba = [180, 178, 170, 255];
    let mut c = Canvas::new(24, 12);
    for plank in 0i32..3 {
        let oy = plank * 4;
        c.fill_rect(0, oy, 24, 1, dark);
        c.fill_rect(0, oy + 1, 24, 1, light);
        c.fill_rect(0, oy + 2, 24, 1, mid);
        c.fill_rect(0, oy + 3, 24, 1, dark);
    }
    for &(x, y) in &[(2, 2), (21, 2), (2, 6), (21, 6), (2, 10), (21, 10)] {
        c.put(x, y, nail);
    }
    c.into_image()
}

// ──── Building wall + roof palette ─────────────────────────────────────

fn building_palette(kind: BuildingType) -> (Rgba, Rgba, Rgba, Rgba) {
    // (wall, roof, roof_dark, trim)
    match kind {
        BuildingType::House => ([168, 123, 74, 255], [139, 58, 42, 255], [90, 31, 21, 255], [90, 53, 32, 255]),
        BuildingType::Shed => ([122, 90, 48, 255], [90, 64, 48, 255], [47, 32, 23, 255], [58, 40, 24, 255]),
        BuildingType::Garage => ([136, 136, 136, 255], [90, 90, 90, 255], [42, 42, 42, 255], [58, 58, 58, 255]),
        BuildingType::Shop => ([168, 152, 120, 255], [90, 90, 90, 255], [42, 42, 42, 255], [58, 58, 58, 255]),
        // Apartment: gray concrete-panel "blok" palette with cool blue tint.
        BuildingType::Apartment => ([148, 152, 158, 255], [98, 100, 108, 255], [54, 56, 62, 255], [180, 184, 192, 255]),
        BuildingType::Civic => ([189, 189, 189, 255], [122, 122, 122, 255], [58, 58, 58, 255], [58, 58, 58, 255]),
        BuildingType::Church => ([154, 136, 112, 255], [58, 58, 74, 255], [26, 26, 37, 255], [90, 74, 58, 255]),
        BuildingType::Market => ([168, 152, 120, 255], [139, 106, 58, 255], [74, 56, 24, 255], [58, 58, 58, 255]),
        BuildingType::Bank => ([189, 189, 189, 255], [90, 90, 90, 255], [42, 42, 42, 255], [58, 58, 58, 255]),
        BuildingType::Tower => ([122, 106, 80, 255], [58, 64, 34, 255], [31, 34, 16, 255], [42, 46, 24, 255]),
        BuildingType::Factory => ([122, 90, 64, 255], [107, 58, 26, 255], [58, 31, 14, 255], [42, 32, 24, 255]),
        BuildingType::Warehouse => ([122, 106, 80, 255], [90, 58, 42, 255], [42, 26, 16, 255], [42, 32, 24, 255]),
        BuildingType::Depot => ([136, 136, 128, 255], [58, 58, 58, 255], [26, 26, 26, 255], [42, 32, 24, 255]),
        BuildingType::Tank => ([136, 136, 136, 255], [102, 102, 102, 255], [58, 58, 58, 255], [26, 26, 26, 255]),
        BuildingType::Hospital => ([232, 238, 244, 255], [168, 184, 200, 255], [90, 104, 120, 255], [30, 77, 107, 255]),
        BuildingType::Morgue => ([189, 200, 212, 255], [122, 138, 154, 255], [58, 74, 90, 255], [30, 77, 107, 255]),
        BuildingType::Park => ([139, 111, 71, 255], [62, 112, 32, 255], [31, 56, 16, 255], [42, 24, 16, 255]),
        BuildingType::Bunker => ([90, 90, 74, 255], [58, 64, 34, 255], [31, 34, 16, 255], [42, 46, 24, 255]),
        BuildingType::Tent => ([58, 58, 42, 255], [90, 107, 42, 255], [47, 58, 20, 255], [26, 26, 16, 255]),
        BuildingType::Helipad => ([58, 58, 58, 255], [42, 42, 42, 255], [26, 26, 26, 255], [255, 217, 61, 255]),
        BuildingType::Gas => ([186, 186, 186, 255], [196, 74, 42, 255], [122, 42, 24, 255], [58, 58, 58, 255]),
    }
}

fn build_building_wall_image(kind: BuildingType) -> Image {
    let (wall, _, dark, trim) = building_palette(kind);
    let mut c = Canvas::new(16, 16);
    c.fill_rect(0, 0, 16, 16, wall);
    // Outline
    c.fill_rect(0, 0, 16, 1, dark);
    c.fill_rect(0, 15, 16, 1, dark);
    c.fill_rect(0, 0, 1, 16, dark);
    c.fill_rect(15, 0, 1, 16, dark);
    // Windows hint (small bright squares)
    for &(x, y) in &[(4, 4), (10, 4), (4, 10), (10, 10)] {
        c.fill_rect(x, y, 2, 2, trim);
    }
    c.into_image()
}

// ──── Interior furniture sprites ───────────────────────────────────────

fn furniture_half(kind: FurnKind) -> Vec2 {
    use FurnKind::*;
    match kind {
        Bed => Vec2::new(20.0, 28.0),
        Couch => Vec2::new(36.0, 14.0),
        Tv => Vec2::new(20.0, 8.0),
        Counter => Vec2::new(40.0, 10.0),
        Desk => Vec2::new(24.0, 14.0),
        Cot => Vec2::new(14.0, 22.0),
        Shelf => Vec2::new(16.0, 8.0),
        Altar => Vec2::new(22.0, 14.0),
        Crate => Vec2::new(14.0, 14.0),
        Barrels => Vec2::new(13.0, 13.0),
        Gurney => Vec2::new(13.0, 24.0),
        Bench => Vec2::new(28.0, 8.0),
        Fridge => Vec2::new(13.0, 16.0),
        Stove => Vec2::new(14.0, 12.0),
        KitchenSink => Vec2::new(16.0, 9.0),
        Toilet => Vec2::new(8.0, 11.0),
        Bathtub => Vec2::new(22.0, 11.0),
        BathSink => Vec2::new(13.0, 9.0),
        Dresser => Vec2::new(20.0, 9.0),
        Wardrobe => Vec2::new(15.0, 11.0),
        Nightstand => Vec2::new(8.0, 8.0),
        CoffeeTable => Vec2::new(16.0, 9.0),
        Bookshelf => Vec2::new(18.0, 9.0),
        DiningTable => Vec2::new(20.0, 14.0),
        DiningChair => Vec2::new(7.0, 7.0),
        ArmChair => Vec2::new(13.0, 12.0),
        Fireplace => Vec2::new(20.0, 10.0),
        // Decorative items still have a half-extent for sprite sizing,
        // but `furniture_collides` returns false so they don't block
        // movement.  Rugs/paintings should never push the player around.
        FloorLamp => Vec2::new(6.0, 18.0),
        Rug => Vec2::new(28.0, 20.0),
        Plant => Vec2::new(9.0, 14.0),
        Painting => Vec2::new(14.0, 4.0),
        Trashcan => Vec2::new(6.0, 9.0),
        FilingCabinet => Vec2::new(11.0, 9.0),
        OfficeChair => Vec2::new(8.0, 9.0),
    }
}

/// Whether a furniture piece blocks movement.  Decorative items (rugs,
/// floor lamps, plants, paintings) only spawn the sprite — `spawn_map`
/// skips obstacle creation when this returns false.
fn furniture_collides(kind: FurnKind) -> bool {
    !matches!(
        kind,
        FurnKind::Rug | FurnKind::Painting | FurnKind::FloorLamp | FurnKind::Plant
    )
}

fn build_furniture_image(kind: FurnKind) -> Image {
    match kind {
        FurnKind::Bed => build_bed(),
        FurnKind::Couch => build_couch(),
        FurnKind::Tv => build_tv(),
        FurnKind::Counter => build_counter(),
        FurnKind::Desk => build_desk(),
        FurnKind::Cot => build_cot(),
        FurnKind::Shelf => build_shelf(),
        FurnKind::Altar => build_altar(),
        FurnKind::Crate => build_crate(),
        FurnKind::Barrels => build_barrels(),
        FurnKind::Gurney => build_gurney(),
        FurnKind::Bench => build_bench(),
        FurnKind::Fridge => build_fridge(),
        FurnKind::Stove => build_stove(),
        FurnKind::KitchenSink => build_kitchen_sink(),
        FurnKind::Toilet => build_toilet(),
        FurnKind::Bathtub => build_bathtub(),
        FurnKind::BathSink => build_bath_sink(),
        FurnKind::Dresser => build_dresser(),
        FurnKind::Wardrobe => build_wardrobe(),
        FurnKind::Nightstand => build_nightstand(),
        FurnKind::CoffeeTable => build_coffee_table(),
        FurnKind::Bookshelf => build_bookshelf(),
        FurnKind::DiningTable => build_dining_table(),
        FurnKind::DiningChair => build_dining_chair(),
        FurnKind::ArmChair => build_armchair(),
        FurnKind::Fireplace => build_fireplace(),
        FurnKind::FloorLamp => build_floor_lamp(),
        FurnKind::Rug => build_rug(),
        FurnKind::Plant => build_plant(),
        FurnKind::Painting => build_painting(),
        FurnKind::Trashcan => build_trashcan(),
        FurnKind::FilingCabinet => build_filing_cabinet(),
        FurnKind::OfficeChair => build_office_chair(),
    }
}

fn build_bed() -> Image {
    let frame: Rgba = [62, 40, 22, 255];
    let frame_d: Rgba = [38, 22, 12, 255];
    let sheet: Rgba = [180, 188, 220, 255];
    let blanket: Rgba = [148, 70, 60, 255];
    let pillow: Rgba = [232, 232, 232, 255];
    let mut c = Canvas::new(40, 56);
    c.fill_rect(0, 0, 40, 56, [0, 0, 0, 0]);
    c.fill_rect(2, 2, 36, 52, frame);
    c.fill_rect(2, 2, 36, 1, frame_d);
    c.fill_rect(2, 53, 36, 1, frame_d);
    c.fill_rect(2, 2, 1, 52, frame_d);
    c.fill_rect(37, 2, 1, 52, frame_d);
    c.fill_rect(4, 4, 32, 48, sheet);
    c.fill_rect(4, 28, 32, 24, blanket);
    c.fill_rect(4, 28, 32, 1, frame_d);
    c.fill_rect(8, 6, 24, 14, pillow);
    c.fill_rect(8, 6, 24, 1, frame_d);
    c.into_image()
}

fn build_couch() -> Image {
    let body: Rgba = [86, 60, 40, 255];
    let body_d: Rgba = [46, 32, 20, 255];
    let body_h: Rgba = [134, 92, 60, 255];
    let cushion: Rgba = [156, 110, 78, 255];
    let mut c = Canvas::new(72, 28);
    c.fill_rect(0, 0, 72, 28, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 72, 28, body);
    c.fill_rect(0, 0, 72, 1, body_d);
    c.fill_rect(0, 27, 72, 1, body_d);
    c.fill_rect(0, 0, 1, 28, body_d);
    c.fill_rect(71, 0, 1, 28, body_d);
    // Backrest band
    c.fill_rect(2, 1, 68, 6, body_h);
    c.fill_rect(2, 6, 68, 1, body_d);
    // Three cushions
    for i in 0i32..3 {
        let x = 4 + i * 22;
        c.fill_rect(x, 9, 20, 16, cushion);
        c.fill_rect(x, 9, 20, 1, body_d);
        c.fill_rect(x, 24, 20, 1, body_d);
    }
    c.into_image()
}

fn build_tv() -> Image {
    let frame: Rgba = [22, 22, 28, 255];
    let screen: Rgba = [40, 50, 64, 255];
    let stand: Rgba = [60, 62, 70, 255];
    let mut c = Canvas::new(40, 16);
    c.fill_rect(0, 0, 40, 16, [0, 0, 0, 0]);
    c.fill_rect(2, 0, 36, 12, frame);
    c.fill_rect(4, 2, 32, 8, screen);
    c.fill_rect(4, 2, 32, 1, [120, 140, 160, 255]);
    c.fill_rect(16, 12, 8, 4, stand);
    c.fill_rect(12, 14, 16, 2, stand);
    c.into_image()
}

fn build_counter() -> Image {
    let body: Rgba = [104, 78, 50, 255];
    let body_d: Rgba = [58, 40, 22, 255];
    let top: Rgba = [178, 158, 130, 255];
    let mut c = Canvas::new(80, 20);
    c.fill_rect(0, 0, 80, 20, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 80, 20, body);
    c.fill_rect(0, 0, 80, 4, top);
    c.fill_rect(0, 4, 80, 1, body_d);
    c.fill_rect(0, 19, 80, 1, body_d);
    // Drawer dividers
    for i in 0..6 {
        c.fill_rect(i * 14 + 4, 8, 2, 8, body_d);
    }
    c.into_image()
}

fn build_desk() -> Image {
    let body: Rgba = [106, 72, 42, 255];
    let body_d: Rgba = [62, 40, 22, 255];
    let top: Rgba = [160, 116, 70, 255];
    let paper: Rgba = [230, 226, 214, 255];
    let mut c = Canvas::new(48, 28);
    c.fill_rect(0, 0, 48, 28, body);
    c.fill_rect(0, 0, 48, 1, body_d);
    c.fill_rect(0, 27, 48, 1, body_d);
    c.fill_rect(0, 0, 1, 28, body_d);
    c.fill_rect(47, 0, 1, 28, body_d);
    c.fill_rect(2, 2, 44, 2, top);
    // Papers + monitor block
    c.fill_rect(8, 8, 14, 8, paper);
    c.put(10, 10, [40, 40, 50, 255]);
    c.put(11, 10, [40, 40, 50, 255]);
    c.fill_rect(28, 6, 14, 12, [22, 22, 28, 255]);
    c.fill_rect(30, 8, 10, 7, [60, 90, 130, 255]);
    c.into_image()
}

fn build_cot() -> Image {
    let frame: Rgba = [80, 82, 92, 255];
    let frame_d: Rgba = [40, 42, 50, 255];
    let sheet: Rgba = [172, 154, 124, 255];
    let pillow: Rgba = [214, 204, 184, 255];
    let mut c = Canvas::new(28, 44);
    c.fill_rect(0, 0, 28, 44, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 28, 44, frame);
    c.fill_rect(2, 2, 24, 40, sheet);
    c.fill_rect(2, 2, 24, 1, frame_d);
    c.fill_rect(2, 41, 24, 1, frame_d);
    c.fill_rect(6, 4, 16, 10, pillow);
    c.put(8, 6, [124, 110, 84, 255]);
    c.into_image()
}

fn build_shelf() -> Image {
    let frame: Rgba = [98, 66, 38, 255];
    let dark: Rgba = [52, 34, 18, 255];
    let wood: Rgba = [140, 96, 56, 255];
    let box_a: Rgba = [192, 80, 54, 255];
    let box_b: Rgba = [86, 132, 180, 255];
    let box_c: Rgba = [200, 176, 98, 255];
    let mut c = Canvas::new(32, 16);
    c.fill_rect(0, 0, 32, 16, frame);
    c.fill_rect(0, 0, 32, 1, dark);
    c.fill_rect(0, 15, 32, 1, dark);
    c.fill_rect(0, 0, 1, 16, dark);
    c.fill_rect(31, 0, 1, 16, dark);
    c.fill_rect(1, 7, 30, 1, wood);
    c.fill_rect(2, 1, 8, 5, box_a);
    c.fill_rect(11, 1, 8, 5, box_b);
    c.fill_rect(20, 1, 10, 5, box_c);
    c.fill_rect(2, 9, 10, 5, box_b);
    c.fill_rect(13, 9, 8, 5, box_c);
    c.fill_rect(22, 9, 8, 5, box_a);
    c.into_image()
}

fn build_altar() -> Image {
    let stone: Rgba = [156, 152, 144, 255];
    let stone_d: Rgba = [88, 84, 78, 255];
    let cloth: Rgba = [148, 50, 38, 255];
    let cloth_d: Rgba = [88, 24, 18, 255];
    let cross: Rgba = [212, 188, 96, 255];
    let candle: Rgba = [228, 224, 200, 255];
    let flame: Rgba = [232, 168, 60, 255];
    let mut c = Canvas::new(44, 28);
    c.fill_rect(0, 0, 44, 28, [0, 0, 0, 0]);
    // Stone block
    c.fill_rect(0, 8, 44, 20, stone);
    c.fill_rect(0, 8, 44, 1, stone_d);
    c.fill_rect(0, 27, 44, 1, stone_d);
    // Cloth on top
    c.fill_rect(2, 6, 40, 5, cloth);
    c.fill_rect(2, 10, 40, 1, cloth_d);
    // Cross
    c.fill_rect(20, 0, 4, 10, cross);
    c.fill_rect(15, 2, 14, 3, cross);
    // Candles
    for &x in &[5i32, 35] {
        c.fill_rect(x, 4, 2, 4, candle);
        c.put(x, 3, flame);
        c.put(x + 1, 3, flame);
    }
    c.into_image()
}

// ──── Kitchen ─────────────────────────────────────────────────────────────

fn build_fridge() -> Image {
    let body: Rgba = [220, 220, 224, 255];
    let body_d: Rgba = [150, 150, 156, 255];
    let body_h: Rgba = [248, 248, 250, 255];
    let handle: Rgba = [60, 62, 70, 255];
    let mut c = Canvas::new(26, 32);
    c.fill_rect(0, 0, 26, 32, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 26, 32, body);
    c.fill_rect(0, 0, 26, 1, body_d);
    c.fill_rect(0, 31, 26, 1, body_d);
    c.fill_rect(0, 0, 1, 32, body_d);
    c.fill_rect(25, 0, 1, 32, body_d);
    // Highlight column
    c.fill_rect(2, 1, 1, 30, body_h);
    // Door split
    c.fill_rect(0, 12, 26, 1, body_d);
    // Handles
    c.fill_rect(20, 4, 2, 6, handle);
    c.fill_rect(20, 16, 2, 12, handle);
    c.into_image()
}

fn build_stove() -> Image {
    let body: Rgba = [80, 82, 90, 255];
    let body_d: Rgba = [40, 42, 50, 255];
    let panel: Rgba = [160, 160, 170, 255];
    let burner: Rgba = [30, 32, 36, 255];
    let burner_glow: Rgba = [220, 90, 40, 255];
    let mut c = Canvas::new(28, 24);
    c.fill_rect(0, 0, 28, 24, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 28, 24, body);
    c.fill_rect(0, 0, 28, 1, body_d);
    c.fill_rect(0, 23, 28, 1, body_d);
    // Top panel
    c.fill_rect(2, 2, 24, 12, panel);
    // 4 burners
    for &(x, y, glow) in &[(4, 4, false), (16, 4, true), (4, 8, false), (16, 8, false)] {
        c.fill_rect(x, y, 5, 4, burner);
        if glow {
            c.fill_rect(x + 1, y + 1, 3, 2, burner_glow);
        }
    }
    // Oven door
    c.fill_rect(2, 16, 24, 6, [60, 60, 68, 255]);
    c.fill_rect(4, 18, 20, 2, [180, 180, 200, 255]);
    c.into_image()
}

fn build_kitchen_sink() -> Image {
    let counter: Rgba = [196, 188, 168, 255];
    let counter_d: Rgba = [120, 112, 92, 255];
    let basin: Rgba = [180, 184, 196, 255];
    let basin_d: Rgba = [104, 110, 124, 255];
    let tap: Rgba = [200, 200, 210, 255];
    let mut c = Canvas::new(32, 18);
    c.fill_rect(0, 0, 32, 18, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 32, 18, counter);
    c.fill_rect(0, 0, 32, 1, counter_d);
    c.fill_rect(0, 17, 32, 1, counter_d);
    // Basin
    c.fill_rect(6, 4, 20, 10, basin);
    c.fill_rect(6, 4, 20, 1, basin_d);
    c.fill_rect(6, 13, 20, 1, basin_d);
    c.fill_rect(6, 4, 1, 10, basin_d);
    c.fill_rect(25, 4, 1, 10, basin_d);
    // Tap
    c.fill_rect(15, 0, 2, 4, tap);
    c.fill_rect(13, 4, 6, 1, tap);
    c.into_image()
}

// ──── Bathroom ────────────────────────────────────────────────────────────

fn build_toilet() -> Image {
    let porcelain: Rgba = [240, 240, 244, 255];
    let porcelain_d: Rgba = [180, 180, 188, 255];
    let seat: Rgba = [255, 255, 255, 255];
    let mut c = Canvas::new(16, 22);
    c.fill_rect(0, 0, 16, 22, [0, 0, 0, 0]);
    // Tank (back)
    c.fill_rect(2, 0, 12, 8, porcelain);
    c.fill_rect(2, 0, 12, 1, porcelain_d);
    c.fill_rect(2, 7, 12, 1, porcelain_d);
    // Bowl (oval-ish)
    c.fill_rect(3, 8, 10, 12, porcelain);
    c.fill_rect(2, 10, 12, 8, porcelain);
    c.fill_rect(2, 10, 12, 1, porcelain_d);
    c.fill_rect(2, 17, 12, 1, porcelain_d);
    // Seat highlight
    c.fill_rect(4, 11, 8, 5, seat);
    c.into_image()
}

fn build_bathtub() -> Image {
    let porcelain: Rgba = [238, 238, 242, 255];
    let porcelain_d: Rgba = [170, 170, 178, 255];
    let water: Rgba = [120, 170, 210, 255];
    let water_h: Rgba = [180, 220, 240, 255];
    let mut c = Canvas::new(44, 22);
    c.fill_rect(0, 0, 44, 22, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 44, 22, porcelain);
    c.fill_rect(0, 0, 44, 1, porcelain_d);
    c.fill_rect(0, 21, 44, 1, porcelain_d);
    c.fill_rect(0, 0, 1, 22, porcelain_d);
    c.fill_rect(43, 0, 1, 22, porcelain_d);
    // Inner basin filled with water
    c.fill_rect(3, 3, 38, 16, water);
    c.fill_rect(3, 3, 38, 1, water_h);
    c.fill_rect(3, 18, 38, 1, porcelain_d);
    // Tap on the right side
    c.fill_rect(38, 8, 4, 2, [200, 200, 210, 255]);
    c.fill_rect(40, 6, 2, 5, [200, 200, 210, 255]);
    c.into_image()
}

fn build_bath_sink() -> Image {
    let cabinet: Rgba = [110, 78, 50, 255];
    let cabinet_d: Rgba = [62, 42, 22, 255];
    let basin: Rgba = [240, 240, 244, 255];
    let basin_d: Rgba = [170, 170, 180, 255];
    let mirror: Rgba = [180, 200, 220, 255];
    let mut c = Canvas::new(26, 18);
    c.fill_rect(0, 0, 26, 18, [0, 0, 0, 0]);
    // Mirror above
    c.fill_rect(4, 0, 18, 4, mirror);
    c.fill_rect(4, 0, 18, 1, [120, 130, 150, 255]);
    // Basin
    c.fill_rect(2, 4, 22, 6, basin);
    c.fill_rect(2, 4, 22, 1, basin_d);
    c.fill_rect(2, 9, 22, 1, basin_d);
    // Cabinet
    c.fill_rect(2, 10, 22, 8, cabinet);
    c.fill_rect(2, 10, 22, 1, cabinet_d);
    c.fill_rect(2, 17, 22, 1, cabinet_d);
    c.fill_rect(12, 12, 2, 4, cabinet_d);
    c.into_image()
}

// ──── Bedroom ─────────────────────────────────────────────────────────────

fn build_dresser() -> Image {
    let body: Rgba = [110, 76, 44, 255];
    let body_d: Rgba = [62, 40, 22, 255];
    let body_h: Rgba = [156, 110, 66, 255];
    let knob: Rgba = [212, 188, 96, 255];
    let mut c = Canvas::new(40, 18);
    c.fill_rect(0, 0, 40, 18, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 40, 18, body);
    c.fill_rect(0, 0, 40, 2, body_h);
    c.fill_rect(0, 0, 40, 1, body_d);
    c.fill_rect(0, 17, 40, 1, body_d);
    c.fill_rect(0, 0, 1, 18, body_d);
    c.fill_rect(39, 0, 1, 18, body_d);
    // 3 drawers
    for &y in &[3i32, 9] {
        for col in 0..3 {
            let x = 3 + col * 12;
            c.fill_rect(x, y, 10, 4, body_h);
            c.fill_rect(x, y, 10, 1, body_d);
            c.fill_rect(x, y + 3, 10, 1, body_d);
            c.put(x + 4, y + 1, knob);
            c.put(x + 5, y + 1, knob);
        }
    }
    c.into_image()
}

fn build_wardrobe() -> Image {
    let body: Rgba = [88, 60, 36, 255];
    let body_d: Rgba = [48, 30, 16, 255];
    let body_h: Rgba = [136, 92, 56, 255];
    let knob: Rgba = [212, 188, 96, 255];
    let mut c = Canvas::new(30, 22);
    c.fill_rect(0, 0, 30, 22, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 30, 22, body);
    c.fill_rect(0, 0, 30, 1, body_d);
    c.fill_rect(0, 21, 30, 1, body_d);
    c.fill_rect(0, 0, 1, 22, body_d);
    c.fill_rect(29, 0, 1, 22, body_d);
    // Two doors
    c.fill_rect(2, 2, 12, 18, body_h);
    c.fill_rect(16, 2, 12, 18, body_h);
    c.fill_rect(14, 0, 2, 22, body_d);
    c.put(11, 11, knob);
    c.put(18, 11, knob);
    c.into_image()
}

fn build_nightstand() -> Image {
    let body: Rgba = [110, 76, 44, 255];
    let body_d: Rgba = [62, 40, 22, 255];
    let top: Rgba = [156, 110, 66, 255];
    let knob: Rgba = [212, 188, 96, 255];
    let mut c = Canvas::new(16, 16);
    c.fill_rect(0, 0, 16, 16, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 16, 16, body);
    c.fill_rect(0, 0, 16, 2, top);
    c.fill_rect(0, 0, 16, 1, body_d);
    c.fill_rect(0, 15, 16, 1, body_d);
    c.fill_rect(0, 0, 1, 16, body_d);
    c.fill_rect(15, 0, 1, 16, body_d);
    // Drawer
    c.fill_rect(3, 5, 10, 6, top);
    c.fill_rect(3, 5, 10, 1, body_d);
    c.put(7, 8, knob);
    c.put(8, 8, knob);
    c.into_image()
}

// ──── Living / dining ─────────────────────────────────────────────────────

fn build_coffee_table() -> Image {
    let top: Rgba = [156, 110, 66, 255];
    let top_d: Rgba = [88, 56, 30, 255];
    let book_a: Rgba = [200, 80, 60, 255];
    let book_b: Rgba = [80, 130, 200, 255];
    let mut c = Canvas::new(32, 18);
    c.fill_rect(0, 0, 32, 18, [0, 0, 0, 0]);
    c.fill_rect(2, 2, 28, 14, top);
    c.fill_rect(2, 2, 28, 1, top_d);
    c.fill_rect(2, 15, 28, 1, top_d);
    c.fill_rect(2, 2, 1, 14, top_d);
    c.fill_rect(29, 2, 1, 14, top_d);
    // Magazines on table
    c.fill_rect(6, 5, 8, 4, book_a);
    c.fill_rect(16, 7, 10, 5, book_b);
    c.into_image()
}

fn build_bookshelf() -> Image {
    let frame: Rgba = [88, 56, 30, 255];
    let frame_d: Rgba = [50, 30, 14, 255];
    let shelf: Rgba = [120, 80, 44, 255];
    let book_a: Rgba = [180, 60, 50, 255];
    let book_b: Rgba = [60, 120, 180, 255];
    let book_c: Rgba = [200, 170, 80, 255];
    let book_d: Rgba = [80, 150, 90, 255];
    let mut c = Canvas::new(36, 18);
    c.fill_rect(0, 0, 36, 18, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 36, 18, frame);
    c.fill_rect(0, 0, 36, 1, frame_d);
    c.fill_rect(0, 17, 36, 1, frame_d);
    c.fill_rect(0, 0, 1, 18, frame_d);
    c.fill_rect(35, 0, 1, 18, frame_d);
    // Shelf divider
    c.fill_rect(1, 8, 34, 1, shelf);
    // Top row of books
    let cols_top = [(2, book_a, 3), (5, book_b, 2), (7, book_c, 4), (11, book_d, 3),
                    (14, book_a, 2), (16, book_b, 3), (19, book_c, 4), (23, book_d, 2),
                    (25, book_a, 3), (28, book_b, 4), (32, book_c, 2)];
    for &(x, col, w) in &cols_top {
        c.fill_rect(x, 1, w, 6, col);
    }
    // Bottom row — alternate colors
    let cols_bot = [(2, book_b, 3), (5, book_c, 2), (7, book_d, 4), (11, book_a, 3),
                    (14, book_b, 2), (16, book_c, 3), (19, book_d, 4), (23, book_a, 2),
                    (25, book_b, 3), (28, book_c, 4), (32, book_d, 2)];
    for &(x, col, w) in &cols_bot {
        c.fill_rect(x, 9, w, 6, col);
    }
    c.into_image()
}

fn build_dining_table() -> Image {
    let top: Rgba = [148, 100, 58, 255];
    let top_d: Rgba = [82, 52, 28, 255];
    let cloth: Rgba = [220, 218, 208, 255];
    let plate: Rgba = [240, 240, 240, 255];
    let mut c = Canvas::new(40, 28);
    c.fill_rect(0, 0, 40, 28, [0, 0, 0, 0]);
    c.fill_rect(2, 2, 36, 24, top);
    c.fill_rect(2, 2, 36, 1, top_d);
    c.fill_rect(2, 25, 36, 1, top_d);
    c.fill_rect(2, 2, 1, 24, top_d);
    c.fill_rect(37, 2, 1, 24, top_d);
    // Tablecloth runner
    c.fill_rect(8, 4, 24, 20, cloth);
    // Plates
    c.fill_rect(11, 8, 5, 4, plate);
    c.fill_rect(24, 8, 5, 4, plate);
    c.fill_rect(11, 16, 5, 4, plate);
    c.fill_rect(24, 16, 5, 4, plate);
    c.into_image()
}

fn build_dining_chair() -> Image {
    let body: Rgba = [108, 72, 42, 255];
    let body_d: Rgba = [64, 40, 22, 255];
    let cushion: Rgba = [180, 70, 60, 255];
    let mut c = Canvas::new(14, 14);
    c.fill_rect(0, 0, 14, 14, [0, 0, 0, 0]);
    // Backrest
    c.fill_rect(1, 0, 12, 4, body);
    c.fill_rect(1, 0, 12, 1, body_d);
    // Seat
    c.fill_rect(0, 4, 14, 8, body);
    c.fill_rect(0, 4, 14, 1, body_d);
    c.fill_rect(2, 6, 10, 4, cushion);
    // Legs
    c.fill_rect(0, 12, 2, 2, body_d);
    c.fill_rect(12, 12, 2, 2, body_d);
    c.into_image()
}

fn build_armchair() -> Image {
    let body: Rgba = [70, 90, 130, 255];
    let body_d: Rgba = [38, 50, 78, 255];
    let body_h: Rgba = [110, 140, 180, 255];
    let cushion: Rgba = [140, 170, 200, 255];
    let mut c = Canvas::new(26, 24);
    c.fill_rect(0, 0, 26, 24, [0, 0, 0, 0]);
    // Back
    c.fill_rect(2, 0, 22, 8, body);
    c.fill_rect(2, 0, 22, 2, body_h);
    // Arms
    c.fill_rect(0, 6, 5, 16, body);
    c.fill_rect(21, 6, 5, 16, body);
    // Seat
    c.fill_rect(5, 8, 16, 14, body_d);
    c.fill_rect(6, 10, 14, 10, cushion);
    c.fill_rect(6, 10, 14, 1, body_h);
    c.fill_rect(0, 22, 26, 2, body_d);
    c.into_image()
}

fn build_fireplace() -> Image {
    let stone: Rgba = [128, 124, 116, 255];
    let stone_d: Rgba = [78, 74, 68, 255];
    let stone_h: Rgba = [168, 162, 152, 255];
    let inside: Rgba = [30, 22, 18, 255];
    let log: Rgba = [88, 56, 30, 255];
    let flame: Rgba = [232, 168, 60, 255];
    let flame_h: Rgba = [255, 230, 130, 255];
    let mut c = Canvas::new(40, 22);
    c.fill_rect(0, 0, 40, 22, [0, 0, 0, 0]);
    // Stone surround
    c.fill_rect(0, 0, 40, 22, stone);
    c.fill_rect(0, 0, 40, 2, stone_h);
    c.fill_rect(0, 20, 40, 2, stone_d);
    // Stone block pattern
    for col in 0..5 {
        c.fill_rect(col * 8, 2, 1, 18, stone_d);
    }
    c.fill_rect(0, 11, 40, 1, stone_d);
    // Hearth opening
    c.fill_rect(8, 5, 24, 14, inside);
    c.fill_rect(8, 5, 24, 1, stone_d);
    // Logs
    c.fill_rect(11, 14, 18, 3, log);
    c.fill_rect(11, 14, 18, 1, [60, 36, 16, 255]);
    // Flames
    c.fill_rect(13, 9, 4, 6, flame);
    c.fill_rect(20, 8, 4, 7, flame);
    c.fill_rect(25, 10, 3, 5, flame);
    c.fill_rect(15, 11, 2, 3, flame_h);
    c.fill_rect(21, 10, 2, 4, flame_h);
    c.into_image()
}

// ──── Decorative (no collision) ───────────────────────────────────────────

fn build_floor_lamp() -> Image {
    let stand: Rgba = [60, 60, 68, 255];
    let shade: Rgba = [220, 200, 130, 255];
    let shade_h: Rgba = [255, 240, 180, 255];
    let glow: Rgba = [255, 230, 150, 200];
    let mut c = Canvas::new(12, 36);
    c.fill_rect(0, 0, 12, 36, [0, 0, 0, 0]);
    // Base
    c.fill_rect(3, 33, 6, 3, stand);
    c.fill_rect(2, 35, 8, 1, stand);
    // Pole
    c.fill_rect(5, 8, 2, 26, stand);
    // Shade (cone-like)
    c.fill_rect(2, 0, 8, 8, shade);
    c.fill_rect(2, 0, 8, 1, shade_h);
    c.fill_rect(1, 1, 1, 6, shade);
    c.fill_rect(10, 1, 1, 6, shade);
    // Glow halo
    c.fill_rect(3, 8, 6, 2, glow);
    c.into_image()
}

fn build_rug() -> Image {
    let outer: Rgba = [148, 60, 50, 255];
    let inner: Rgba = [200, 90, 70, 255];
    let core: Rgba = [232, 200, 130, 255];
    let dark: Rgba = [88, 30, 24, 255];
    let mut c = Canvas::new(56, 40);
    c.fill_rect(0, 0, 56, 40, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 56, 40, outer);
    c.fill_rect(2, 2, 52, 36, inner);
    c.fill_rect(6, 6, 44, 28, core);
    // Border lines
    c.fill_rect(0, 0, 56, 1, dark);
    c.fill_rect(0, 39, 56, 1, dark);
    c.fill_rect(0, 0, 1, 40, dark);
    c.fill_rect(55, 0, 1, 40, dark);
    // Pattern dots
    for x in [12, 22, 32, 42] {
        c.fill_rect(x, 14, 2, 2, outer);
        c.fill_rect(x, 24, 2, 2, outer);
    }
    // Tassels (top + bottom)
    for x in (1..56).step_by(3) {
        c.put(x, 0, dark);
        c.put(x, 39, dark);
    }
    c.into_image()
}

fn build_plant() -> Image {
    let pot: Rgba = [140, 80, 50, 255];
    let pot_d: Rgba = [82, 46, 26, 255];
    let leaf: Rgba = [54, 130, 60, 255];
    let leaf_h: Rgba = [110, 180, 80, 255];
    let leaf_d: Rgba = [30, 80, 38, 255];
    let mut c = Canvas::new(18, 28);
    c.fill_rect(0, 0, 18, 28, [0, 0, 0, 0]);
    // Pot
    c.fill_rect(3, 18, 12, 10, pot);
    c.fill_rect(3, 18, 12, 1, [180, 110, 70, 255]);
    c.fill_rect(3, 27, 12, 1, pot_d);
    c.fill_rect(3, 18, 1, 10, pot_d);
    c.fill_rect(14, 18, 1, 10, pot_d);
    // Leaves — a few overlapping ovals
    c.fill_rect(7, 0, 4, 12, leaf);
    c.fill_rect(2, 4, 5, 10, leaf);
    c.fill_rect(11, 4, 5, 10, leaf);
    c.fill_rect(4, 8, 3, 8, leaf_h);
    c.fill_rect(11, 8, 3, 8, leaf_h);
    c.fill_rect(8, 1, 2, 10, leaf_h);
    c.fill_rect(0, 9, 2, 6, leaf_d);
    c.fill_rect(16, 9, 2, 6, leaf_d);
    c.into_image()
}

fn build_painting() -> Image {
    let frame: Rgba = [212, 188, 96, 255];
    let frame_d: Rgba = [148, 124, 50, 255];
    let sky: Rgba = [110, 160, 200, 255];
    let mountain: Rgba = [120, 100, 80, 255];
    let snow: Rgba = [240, 240, 245, 255];
    let mut c = Canvas::new(28, 8);
    c.fill_rect(0, 0, 28, 8, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 28, 8, frame);
    c.fill_rect(0, 0, 28, 1, frame_d);
    c.fill_rect(0, 7, 28, 1, frame_d);
    // Inner painting
    c.fill_rect(2, 2, 24, 4, sky);
    // Mountains
    c.fill_rect(4, 3, 6, 3, mountain);
    c.fill_rect(10, 4, 8, 2, mountain);
    c.fill_rect(18, 3, 6, 3, mountain);
    // Snow caps
    c.put(6, 3, snow);
    c.put(7, 3, snow);
    c.put(20, 3, snow);
    c.put(21, 3, snow);
    c.into_image()
}

// ──── Misc ────────────────────────────────────────────────────────────────

fn build_trashcan() -> Image {
    let body: Rgba = [80, 84, 88, 255];
    let body_d: Rgba = [40, 44, 48, 255];
    let lid: Rgba = [120, 124, 130, 255];
    let mut c = Canvas::new(12, 18);
    c.fill_rect(0, 0, 12, 18, [0, 0, 0, 0]);
    c.fill_rect(1, 2, 10, 14, body);
    c.fill_rect(1, 2, 10, 1, body_d);
    c.fill_rect(1, 15, 10, 1, body_d);
    c.fill_rect(1, 2, 1, 14, body_d);
    c.fill_rect(10, 2, 1, 14, body_d);
    // Vertical ribs
    c.fill_rect(4, 4, 1, 10, body_d);
    c.fill_rect(7, 4, 1, 10, body_d);
    // Lid
    c.fill_rect(0, 0, 12, 2, lid);
    c.fill_rect(0, 1, 12, 1, body_d);
    c.into_image()
}

fn build_filing_cabinet() -> Image {
    let body: Rgba = [90, 92, 100, 255];
    let body_d: Rgba = [54, 56, 64, 255];
    let body_h: Rgba = [128, 130, 138, 255];
    let knob: Rgba = [212, 188, 96, 255];
    let label: Rgba = [220, 220, 220, 255];
    let mut c = Canvas::new(22, 18);
    c.fill_rect(0, 0, 22, 18, [0, 0, 0, 0]);
    c.fill_rect(0, 0, 22, 18, body);
    c.fill_rect(0, 0, 22, 1, body_h);
    c.fill_rect(0, 17, 22, 1, body_d);
    c.fill_rect(0, 0, 1, 18, body_d);
    c.fill_rect(21, 0, 1, 18, body_d);
    // Drawer dividers
    c.fill_rect(0, 6, 22, 1, body_d);
    c.fill_rect(0, 12, 22, 1, body_d);
    // Labels and knobs
    for &y in &[2i32, 8, 14] {
        c.fill_rect(7, y, 8, 2, label);
        c.put(18, y + 1, knob);
        c.put(19, y + 1, knob);
    }
    c.into_image()
}

fn build_office_chair() -> Image {
    let body: Rgba = [40, 42, 48, 255];
    let body_d: Rgba = [22, 22, 28, 255];
    let cushion: Rgba = [70, 90, 130, 255];
    let cushion_h: Rgba = [110, 140, 180, 255];
    let mut c = Canvas::new(16, 18);
    c.fill_rect(0, 0, 16, 18, [0, 0, 0, 0]);
    // Backrest
    c.fill_rect(3, 0, 10, 6, cushion);
    c.fill_rect(3, 0, 10, 1, cushion_h);
    c.fill_rect(3, 0, 1, 6, body_d);
    c.fill_rect(12, 0, 1, 6, body_d);
    // Seat
    c.fill_rect(2, 6, 12, 6, cushion);
    c.fill_rect(2, 6, 12, 1, cushion_h);
    c.fill_rect(2, 11, 12, 1, body_d);
    // Star base
    c.fill_rect(7, 12, 2, 4, body);
    c.fill_rect(0, 16, 16, 2, body);
    c.fill_rect(0, 17, 16, 1, body_d);
    c.into_image()
}

fn build_interior_floor_image(kind: BuildingType) -> Image {
    // Per-archetype floor texture: wood for residential, tile for commercial,
    // concrete for industrial/military, stone for civic/church.
    let (a, b, c) = match kind {
        BuildingType::House | BuildingType::Apartment | BuildingType::Park => (
            [132, 90, 54, 255] as Rgba,
            [94, 62, 36, 255] as Rgba,
            [162, 114, 68, 255] as Rgba,
        ),
        BuildingType::Shop | BuildingType::Market | BuildingType::Bank
        | BuildingType::Hospital | BuildingType::Morgue | BuildingType::Gas => (
            [216, 214, 208, 255],
            [150, 146, 138, 255],
            [238, 234, 228, 255],
        ),
        BuildingType::Civic | BuildingType::Church => (
            [108, 106, 102, 255],
            [80, 78, 74, 255],
            [148, 144, 136, 255],
        ),
        BuildingType::Factory | BuildingType::Warehouse | BuildingType::Depot
        | BuildingType::Garage | BuildingType::Bunker | BuildingType::Tower
        | BuildingType::Helipad | BuildingType::Tank => (
            [124, 124, 128, 255],
            [88, 88, 92, 255],
            [158, 158, 160, 255],
        ),
        BuildingType::Tent | BuildingType::Shed => (
            [108, 88, 60, 255],
            [72, 56, 36, 255],
            [148, 122, 84, 255],
        ),
    };
    let mut canvas = Canvas::new(32, 32);
    canvas.fill_rect(0, 0, 32, 32, a);
    // Plank/tile edges so the floor reads as oriented surface.
    if matches!(
        kind,
        BuildingType::House | BuildingType::Apartment | BuildingType::Park
            | BuildingType::Tent | BuildingType::Shed
    ) {
        for plank in 0i32..4 {
            let oy = plank * 8;
            canvas.fill_rect(0, oy, 32, 1, b);
            canvas.fill_rect(0, oy + 7, 32, 1, b);
            canvas.fill_rect(1, oy + 1, 30, 1, c);
        }
    } else {
        // Tile grid
        for gx in 0..2 {
            for gy in 0..2 {
                let ox = gx * 16;
                let oy = gy * 16;
                canvas.fill_rect(ox, oy, 16, 1, b);
                canvas.fill_rect(ox, oy, 1, 16, b);
                canvas.fill_rect(ox + 1, oy + 1, 14, 1, c);
            }
        }
    }
    canvas.into_image()
}

/// Sprite for an apartment window — warm yellow glow with crossbars.
/// Sized at ~55%×42% of a tile so multiple fit on each wall side.
fn build_window_image() -> Image {
    let frame: Rgba = [40, 32, 22, 255];
    let glass: Rgba = [255, 220, 130, 255];
    let glass_hi: Rgba = [255, 245, 200, 255];
    let glass_dim: Rgba = [200, 150, 70, 255];
    let mut c = Canvas::new(14, 11);
    // Frame
    c.fill_rect(0, 0, 14, 11, frame);
    // Pane
    c.fill_rect(1, 1, 12, 9, glass);
    c.fill_rect(1, 1, 12, 2, glass_hi);
    c.fill_rect(1, 8, 12, 1, glass_dim);
    // Cross-bars
    c.fill_rect(7, 1, 1, 9, frame);
    c.fill_rect(1, 5, 12, 1, frame);
    c.into_image()
}

/// Sprite for the interior partition walls inside a multi-story building's
/// residential floor — a simpler, lighter-shaded plaster-on-concrete look
/// distinct from the heavy outer brick walls.
fn build_interior_partition_image(kind: BuildingType) -> Image {
    let (base, light, dark, accent) = match kind {
        BuildingType::Hospital => (
            [228, 230, 232, 255] as Rgba,
            [248, 250, 252, 255] as Rgba,
            [170, 174, 180, 255] as Rgba,
            [120, 170, 220, 255] as Rgba,
        ),
        // Apartment + default — typical "wielka płyta" plaster: dirty cream
        // over concrete with a beige skirting board.
        _ => (
            [206, 198, 178, 255] as Rgba,
            [232, 224, 202, 255] as Rgba,
            [148, 138, 116, 255] as Rgba,
            [104, 78, 48, 255] as Rgba,
        ),
    };
    let mut c = Canvas::new(32, 8);
    c.fill_rect(0, 0, 32, 8, base);
    c.fill_rect(0, 0, 32, 1, light);
    c.fill_rect(0, 7, 32, 1, dark);
    // Skirting / chair-rail stripe so the panel reads as architectural.
    c.fill_rect(0, 5, 32, 1, accent);
    // Panel seam every 8 px to suggest pre-cast slabs.
    for x in (8i32..32).step_by(8) {
        c.fill_rect(x, 1, 1, 6, dark);
    }
    c.into_image()
}

fn build_staircase_image() -> Image {
    // Top-down view of a flight of stairs going up.  Wide steps with
    // shadows + handrail stripes on each side.
    let step: Rgba = [128, 124, 116, 255];
    let step_d: Rgba = [60, 56, 50, 255];
    let step_h: Rgba = [188, 184, 174, 255];
    let rail: Rgba = [40, 40, 46, 255];
    let arrow: Rgba = [232, 220, 120, 255];
    let mut c = Canvas::new(24, 36);
    c.fill_rect(0, 0, 24, 36, [0, 0, 0, 0]);
    // Frame with handrails
    c.fill_rect(0, 0, 24, 36, step);
    c.fill_rect(0, 0, 24, 1, step_d);
    c.fill_rect(0, 35, 24, 1, step_d);
    c.fill_rect(0, 0, 2, 36, rail);
    c.fill_rect(22, 0, 2, 36, rail);
    // Steps — bands of light/dark to show climbing direction
    for i in 0i32..7 {
        let y = 2 + i * 5;
        c.fill_rect(2, y, 20, 4, step);
        c.fill_rect(2, y, 20, 1, step_h);
        c.fill_rect(2, y + 3, 20, 1, step_d);
    }
    // Up arrow at the top of the stair (toward player goes up)
    c.fill_rect(11, 2, 2, 6, arrow);
    c.put(10, 3, arrow);
    c.put(13, 3, arrow);
    c.put(9, 4, arrow);
    c.put(14, 4, arrow);
    c.into_image()
}

fn build_rooftop_floor_image() -> Image {
    // Concrete deck with crack lines and a slight bird-poo pattern.
    let base: Rgba = [124, 126, 132, 255];
    let dark: Rgba = [82, 84, 92, 255];
    let light: Rgba = [162, 164, 170, 255];
    let stain: Rgba = [196, 196, 188, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, base);
    // Joint lines (every 8 px)
    for n in 0i32..4 {
        c.fill_rect(0, n * 8, 32, 1, dark);
        c.fill_rect(n * 8, 0, 1, 32, dark);
    }
    // Highlights
    for &(x, y) in &[(2, 2), (10, 10), (18, 18), (26, 6), (4, 26)] {
        c.put(x, y, light);
    }
    // Stain spots
    for &(x, y) in &[(14, 6), (22, 22), (6, 18)] {
        c.put(x, y, stain);
    }
    c.into_image()
}

fn build_hvac_image() -> Image {
    let metal: Rgba = [148, 152, 158, 255];
    let metal_d: Rgba = [82, 84, 92, 255];
    let metal_h: Rgba = [196, 200, 206, 255];
    let vent: Rgba = [54, 56, 62, 255];
    let warn: Rgba = [220, 170, 30, 255];
    let mut c = Canvas::new(48, 36);
    c.fill_rect(0, 0, 48, 36, [0, 0, 0, 0]);
    c.fill_rect(2, 2, 44, 32, metal);
    c.fill_rect(2, 2, 44, 1, metal_h);
    c.fill_rect(2, 33, 44, 1, metal_d);
    c.fill_rect(2, 2, 1, 32, metal_d);
    c.fill_rect(45, 2, 1, 32, metal_d);
    // Vent grille on top
    for i in 0i32..6 {
        let x = 6 + i * 6;
        c.fill_rect(x, 6, 4, 8, vent);
        c.fill_rect(x, 6, 4, 1, [16, 18, 22, 255]);
    }
    // Warning sticker
    c.fill_rect(10, 22, 8, 6, warn);
    c.fill_rect(10, 22, 8, 1, metal_d);
    // Pipes coming out the side
    c.fill_rect(46, 14, 2, 4, metal_d);
    c.into_image()
}

fn build_antenna_image() -> Image {
    let mast: Rgba = [40, 42, 48, 255];
    let base: Rgba = [108, 110, 116, 255];
    let warn: Rgba = [220, 60, 50, 255];
    let mut c = Canvas::new(20, 48);
    c.fill_rect(0, 0, 20, 48, [0, 0, 0, 0]);
    // Square base
    c.fill_rect(4, 38, 12, 8, base);
    c.fill_rect(4, 38, 12, 1, [40, 42, 48, 255]);
    c.fill_rect(4, 45, 12, 1, [40, 42, 48, 255]);
    // Mast
    c.fill_rect(9, 4, 2, 36, mast);
    // Cross arms
    c.fill_rect(3, 12, 14, 1, mast);
    c.fill_rect(5, 18, 10, 1, mast);
    c.fill_rect(2, 24, 16, 1, mast);
    c.fill_rect(6, 30, 8, 1, mast);
    // Red warning light at the tip
    c.put(9, 2, warn);
    c.put(10, 2, warn);
    c.put(9, 3, warn);
    c.put(10, 3, warn);
    c.into_image()
}

fn build_roof_vent_image() -> Image {
    let metal: Rgba = [108, 110, 118, 255];
    let metal_d: Rgba = [54, 56, 62, 255];
    let metal_h: Rgba = [148, 152, 160, 255];
    let opening: Rgba = [22, 24, 28, 255];
    let mut c = Canvas::new(28, 20);
    c.fill_rect(0, 0, 28, 20, [0, 0, 0, 0]);
    // Mushroom-cap vent
    c.fill_circle(14, 9, 9, metal_d);
    c.fill_circle(14, 9, 8, metal);
    c.fill_circle(14, 9, 5, opening);
    c.fill_rect(11, 12, 6, 7, metal);
    c.fill_rect(11, 12, 6, 1, metal_d);
    c.put(12, 7, metal_h);
    c.into_image()
}

fn build_door_frame_image() -> Image {
    // Stone threshold with two clear jambs flanking the doorway opening.
    // Drawn just outside the wall so the entrance is unambiguous.
    let stone: Rgba = [156, 152, 144, 255];
    let stone_d: Rgba = [88, 84, 78, 255];
    let stone_h: Rgba = [200, 196, 188, 255];
    let mut c = Canvas::new(40, 10);
    c.fill_rect(0, 0, 40, 10, [0, 0, 0, 0]);
    // Left jamb (chunky stone block)
    c.fill_rect(0, 0, 7, 10, stone);
    c.fill_rect(0, 0, 7, 1, stone_h);
    c.fill_rect(0, 9, 7, 1, stone_d);
    c.fill_rect(0, 0, 1, 10, stone_d);
    // Inner block detail
    c.fill_rect(2, 4, 3, 1, stone_d);
    // Right jamb mirror
    c.fill_rect(33, 0, 7, 10, stone);
    c.fill_rect(33, 0, 7, 1, stone_h);
    c.fill_rect(33, 9, 7, 1, stone_d);
    c.fill_rect(39, 0, 1, 10, stone_d);
    c.fill_rect(35, 4, 3, 1, stone_d);
    // Threshold step (low stone band linking the jambs)
    c.fill_rect(7, 3, 26, 4, stone_d);
    c.fill_rect(7, 3, 26, 1, [62, 58, 54, 255]);
    c.fill_rect(7, 6, 26, 1, [42, 40, 38, 255]);
    c.into_image()
}

fn build_welcome_mat_image() -> Image {
    let mat: Rgba = [148, 50, 38, 255];
    let mat_d: Rgba = [88, 24, 18, 255];
    let mat_hi: Rgba = [196, 88, 70, 255];
    let stripe: Rgba = [228, 200, 92, 255];
    let mut c = Canvas::new(20, 10);
    c.fill_rect(0, 0, 20, 10, [0, 0, 0, 0]);
    c.fill_rect(1, 1, 18, 8, mat);
    c.fill_rect(1, 1, 18, 1, mat_d);
    c.fill_rect(1, 8, 18, 1, mat_d);
    c.fill_rect(1, 1, 1, 8, mat_d);
    c.fill_rect(18, 1, 1, 8, mat_d);
    c.fill_rect(2, 2, 16, 1, mat_hi);
    c.fill_rect(2, 4, 16, 2, stripe);
    c.fill_rect(2, 5, 16, 1, mat_d);
    c.into_image()
}

fn build_door_image(kind: BuildingType, side: WallSide) -> Image {
    // Doors are rendered with custom_size matching their wall slot, so we
    // build the canvas at the same aspect (32×16 for N/S walls, 16×32 for
    // E/W) — that way the planks and knob don't stretch into ovals.
    let (_, panel, dark, trim) = building_palette(kind);
    let plank_hi: Rgba = [
        ((panel[0] as i32 + 30).min(255)) as u8,
        ((panel[1] as i32 + 22).min(255)) as u8,
        ((panel[2] as i32 + 16).min(255)) as u8,
        255,
    ];
    let knob = [220, 188, 70, 255];
    let knob_d = [120, 96, 30, 255];
    let hinge = [50, 50, 56, 255];

    match side {
        WallSide::N | WallSide::S => {
            // Horizontal door: 32 wide × 16 tall.  Planks run vertically
            // (top→bottom of canvas) so the door reads as a wide threshold.
            let mut c = Canvas::new(32, 16);
            c.fill_rect(0, 0, 32, 16, [0, 0, 0, 0]);
            // Outer frame
            c.fill_rect(1, 1, 30, 14, dark);
            c.fill_rect(2, 2, 28, 12, panel);
            // Plank seams (3 planks)
            for px in [10, 20] {
                c.fill_rect(px, 2, 1, 12, dark);
                c.fill_rect(px - 1, 2, 1, 12, plank_hi);
            }
            // Highlight along the top edge of each plank
            c.fill_rect(2, 2, 28, 1, plank_hi);
            // Hinges on the left side
            c.fill_rect(2, 4, 3, 2, hinge);
            c.fill_rect(2, 10, 3, 2, hinge);
            // Knob on the right (slightly inset)
            c.fill_rect(26, 7, 2, 2, knob);
            c.put(28, 8, knob_d);
            // Decorative trim band (matches roof palette so doors feel
            // tied to the rest of the building).
            c.fill_rect(2, 8, 28, 1, trim);
            c.into_image()
        }
        WallSide::E | WallSide::W => {
            // Vertical door: 16 wide × 32 tall.  Planks run horizontally
            // (left→right of canvas) which matches a side-hung door panel.
            let mut c = Canvas::new(16, 32);
            c.fill_rect(0, 0, 16, 32, [0, 0, 0, 0]);
            c.fill_rect(1, 1, 14, 30, dark);
            c.fill_rect(2, 2, 12, 28, panel);
            for py in [10, 20] {
                c.fill_rect(2, py, 12, 1, dark);
                c.fill_rect(2, py - 1, 12, 1, plank_hi);
            }
            c.fill_rect(2, 2, 1, 28, plank_hi);
            // Hinges on top side
            c.fill_rect(4, 2, 2, 3, hinge);
            c.fill_rect(10, 2, 2, 3, hinge);
            // Knob near bottom
            c.fill_rect(7, 26, 2, 2, knob);
            c.put(8, 28, knob_d);
            // Trim band
            c.fill_rect(2, 16, 12, 1, trim);
            c.into_image()
        }
    }
}

fn build_roof_image(kind: BuildingType, style: RoofStyle, w_tiles: i32, h_tiles: i32) -> Image {
    let (_, roof, dark, trim) = building_palette(kind);
    let tile_px = 12;
    let w = (w_tiles * tile_px).max(16);
    let h = (h_tiles * tile_px).max(16);
    let mut c = Canvas::new(w, h);
    c.fill_rect(0, 0, w, h, [0, 0, 0, 0]);

    match style {
        RoofStyle::Gable => {
            // Real terracotta-tile roof: two slopes split by a ridge cap,
            // each slope filled with staggered scalloped dachówka rows.
            // Side facing the camera is brighter; far side darker for depth.
            let horiz = w >= h;
            let near = roof;
            let far: Rgba = [
                ((roof[0] as f32 * 0.65) as u8),
                ((roof[1] as f32 * 0.65) as u8),
                ((roof[2] as f32 * 0.65) as u8),
                255,
            ];
            let near_d: Rgba = [
                ((roof[0] as f32 * 0.78) as u8),
                ((roof[1] as f32 * 0.78) as u8),
                ((roof[2] as f32 * 0.78) as u8),
                255,
            ];
            let far_d: Rgba = [
                ((roof[0] as f32 * 0.45) as u8),
                ((roof[1] as f32 * 0.45) as u8),
                ((roof[2] as f32 * 0.45) as u8),
                255,
            ];
            let ridge = dark;
            let ridge_h: Rgba = [
                ((dark[0] as i32 + 40).min(255)) as u8,
                ((dark[1] as i32 + 40).min(255)) as u8,
                ((dark[2] as i32 + 40).min(255)) as u8,
                255,
            ];
            let smoke = [80, 80, 84, 200];

            let draw_dachówka = |c: &mut Canvas, x0: i32, y0: i32, w: i32, h: i32,
                                 base: Rgba, base_d: Rgba| {
                c.fill_rect(x0, y0, w, h, base);
                // Tile rows ~3 px tall, staggered every other row by 2 px.
                let row_h = 3;
                let tile_w = 4;
                let mut y = y0;
                let mut row_idx = 0i32;
                while y < y0 + h {
                    let rh = row_h.min(y0 + h - y);
                    let offset = if row_idx % 2 == 0 { 0 } else { 2 };
                    let mut x = x0 - offset;
                    while x < x0 + w {
                        // Curved scallop bottom of each tile drawn as a
                        // shadow line with a small dot of highlight.
                        let tx = x.max(x0);
                        let tw = (x + tile_w).min(x0 + w) - tx;
                        if tw > 0 {
                            // Bottom shadow line
                            c.fill_rect(tx, y + rh - 1, tw, 1, base_d);
                            // Subtle vertical seam between tiles
                            if tx > x0 {
                                c.fill_rect(tx, y, 1, rh, base_d);
                            }
                        }
                        x += tile_w;
                    }
                    y += row_h;
                    row_idx += 1;
                }
            };

            if horiz {
                let mid = h / 2;
                draw_dachówka(&mut c, 0, 0, w, mid, near, near_d);
                draw_dachówka(&mut c, 0, mid, w, h - mid, far, far_d);
                // Ridge cap (3 px tall) along the apex
                c.fill_rect(0, mid - 1, w, 3, ridge);
                c.fill_rect(0, mid - 1, w, 1, ridge_h);
                // Eaves shadow at top + bottom
                c.fill_rect(0, 0, w, 1, far_d);
                c.fill_rect(0, h - 1, w, 1, far_d);
                // Chimney + smoke wisp (top-right, on near slope)
                let cx = w - 9;
                let cy = 2;
                c.fill_rect(cx, cy, 5, 6, trim);
                c.fill_rect(cx, cy, 5, 1, ridge);
                c.fill_rect(cx, cy + 5, 5, 1, ridge);
                c.fill_rect(cx + 1, cy - 2, 3, 2, smoke);
                c.fill_rect(cx + 2, cy - 4, 2, 2, smoke);
            } else {
                let mid = w / 2;
                draw_dachówka(&mut c, 0, 0, mid, h, near, near_d);
                draw_dachówka(&mut c, mid, 0, w - mid, h, far, far_d);
                c.fill_rect(mid - 1, 0, 3, h, ridge);
                c.fill_rect(mid - 1, 0, 1, h, ridge_h);
                c.fill_rect(0, 0, 1, h, far_d);
                c.fill_rect(w - 1, 0, 1, h, far_d);
                let cx = 2;
                let cy = h - 9;
                c.fill_rect(cx, cy, 5, 6, trim);
                c.fill_rect(cx, cy, 5, 1, ridge);
                c.fill_rect(cx, cy + 5, 5, 1, ridge);
                c.fill_rect(cx + 1, cy - 2, 3, 2, smoke);
                c.fill_rect(cx + 2, cy - 4, 2, 2, smoke);
            }
        }
        RoofStyle::Flat => {
            // Solid gravel-covered roof with raised parapet around the edge.
            // The gravel is a stipple of three brightnesses; parapet steps
            // down with a highlight on the inner face for depth.
            let gravel_a = roof;
            let gravel_b: Rgba = [
                ((roof[0] as i32 - 14).max(0)) as u8,
                ((roof[1] as i32 - 14).max(0)) as u8,
                ((roof[2] as i32 - 14).max(0)) as u8,
                255,
            ];
            let gravel_c: Rgba = [
                ((roof[0] as i32 + 18).min(255)) as u8,
                ((roof[1] as i32 + 18).min(255)) as u8,
                ((roof[2] as i32 + 18).min(255)) as u8,
                255,
            ];
            c.fill_rect(0, 0, w, h, gravel_a);
            // Parapet (3 px wide stone wall around perimeter) + inner shadow
            c.fill_rect(0, 0, w, 3, dark);
            c.fill_rect(0, h - 3, w, 3, dark);
            c.fill_rect(0, 0, 3, h, dark);
            c.fill_rect(w - 3, 0, 3, h, dark);
            c.fill_rect(3, 3, w - 6, 1, [0, 0, 0, 110]);
            c.fill_rect(3, h - 4, w - 6, 1, [0, 0, 0, 110]);
            c.fill_rect(3, 3, 1, h - 6, [0, 0, 0, 110]);
            c.fill_rect(w - 4, 3, 1, h - 6, [0, 0, 0, 110]);
            // Gravel stipple — deterministic checker patches so it isn't a
            // flat slab but reads as aggregate.
            for y in 4..h - 4 {
                for x in 4..w - 4 {
                    let n = (x * 73 + y * 131 + x * y) & 0xFF;
                    if n < 30 {
                        c.put(x, y, gravel_b);
                    } else if n < 60 {
                        c.put(x, y, gravel_c);
                    }
                }
            }
            // No grid lines — the gravel stipple already gives texture.
            // AC unit (if room)
            if w >= 24 && h >= 18 {
                c.fill_rect(w / 2 - 3, h / 2 - 3, 6, 6, trim);
                c.fill_rect(w / 2 - 3, h / 2 - 3, 6, 1, [0, 0, 0, 255]);
            }
            // Type-specific roof extras
            match kind {
                BuildingType::Hospital | BuildingType::Morgue => {
                    // Red cross in centre
                    let cx = w / 2;
                    let cy = h / 2;
                    c.fill_rect(cx - 1, cy - 5, 2, 10, [196, 30, 30, 255]);
                    c.fill_rect(cx - 5, cy - 1, 10, 2, [196, 30, 30, 255]);
                }
                BuildingType::Civic => {
                    // Five columns at front
                    for i in 0..5 {
                        let x = 4 + i * (w - 8) / 5;
                        c.fill_rect(x, h - 6, 2, 4, [0, 0, 0, 200]);
                    }
                }
                BuildingType::Bank => {
                    let cx = w / 2;
                    let cy = h / 2;
                    // $ symbol — vertical bar + curls
                    c.fill_rect(cx, cy - 4, 1, 9, trim);
                    c.fill_rect(cx - 2, cy - 3, 4, 1, trim);
                    c.fill_rect(cx + 1, cy, 2, 1, trim);
                    c.fill_rect(cx - 2, cy + 3, 4, 1, trim);
                }
                BuildingType::Gas => {
                    // Red roof stripe + small SHOP marker
                    c.fill_rect(0, 2, w, 3, [196, 74, 42, 255]);
                    c.fill_rect(w / 2 - 6, h / 2 - 1, 12, 3, [255, 217, 61, 255]);
                }
                BuildingType::Tower => {
                    let cx = w / 2;
                    let cy = h / 2;
                    c.fill_rect(cx - 3, cy - 3, 6, 6, dark);
                }
                BuildingType::Tank => {
                    let cx = w / 2;
                    let cy = h / 2;
                    c.fill_rect(cx - 1, cy - 4, 2, 9, trim);
                    c.fill_rect(cx - 4, cy - 1, 9, 2, trim);
                }
                _ => {}
            }
        }
        RoofStyle::Apt => {
            // Polish "blok" rooftop seen straight down — solid bitumen
            // surface, raised parapet, tar seams between roof segments,
            // central stairwell shed, antenna mast, satellite dish, HVAC
            // box, vent pipes, drainage gutter.  Deliberately NO windows
            // here — windows belong on the FACADE which is rendered
            // separately by `spawn_building_windows`, not on the roof.
            let bitumen = roof;
            let seam = dark;
            let bitumen_d: Rgba = [
                ((roof[0] as i32 - 18).max(0)) as u8,
                ((roof[1] as i32 - 18).max(0)) as u8,
                ((roof[2] as i32 - 18).max(0)) as u8,
                255,
            ];
            let bitumen_h: Rgba = [
                ((roof[0] as i32 + 18).min(255)) as u8,
                ((roof[1] as i32 + 18).min(255)) as u8,
                ((roof[2] as i32 + 18).min(255)) as u8,
                255,
            ];
            let metal: Rgba = [110, 114, 122, 255];
            let metal_d: Rgba = [62, 64, 70, 255];
            let metal_h: Rgba = [180, 184, 192, 255];
            let red_lamp: Rgba = [220, 60, 50, 255];
            c.fill_rect(0, 0, w, h, bitumen);
            // Stipple texture so the surface doesn't look like flat paint.
            for y in 4..h - 4 {
                for x in 4..w - 4 {
                    let n = (x * 53 + y * 97 + x * y) & 0xFF;
                    if n < 32 {
                        c.put(x, y, bitumen_d);
                    } else if n < 56 {
                        c.put(x, y, bitumen_h);
                    }
                }
            }
            // Parapet wall — 3 px tall, thicker at the top edges so it
            // reads as a wall rather than a stripe.
            c.fill_rect(0, 0, w, 3, dark);
            c.fill_rect(0, h - 3, w, 3, dark);
            c.fill_rect(0, 0, 3, h, dark);
            c.fill_rect(w - 3, 0, 3, h, dark);
            c.fill_rect(3, 3, w - 6, 1, [0, 0, 0, 130]);
            c.fill_rect(3, h - 4, w - 6, 1, [0, 0, 0, 130]);
            c.fill_rect(3, 3, 1, h - 6, [0, 0, 0, 130]);
            c.fill_rect(w - 4, 3, 1, h - 6, [0, 0, 0, 130]);
            // Tar seams — three horizontal stripes splitting the surface
            // into roofing-felt strips.  Single colour, no panel highlight.
            let seam_step = ((h - 6) / 4).max(8);
            let mut sy = 4 + seam_step;
            while sy < h - 4 {
                c.fill_rect(4, sy, w - 8, 1, seam);
                sy += seam_step;
            }
            // Central stairwell shed (raised box with a pitched cover).
            if w >= 18 && h >= 14 {
                let cx = w / 2;
                let cy = h / 2;
                c.fill_rect(cx - 5, cy - 4, 10, 9, metal);
                c.fill_rect(cx - 5, cy - 4, 10, 1, metal_h);
                c.fill_rect(cx - 5, cy + 4, 10, 1, metal_d);
                c.fill_rect(cx - 5, cy - 4, 1, 9, metal_d);
                c.fill_rect(cx + 4, cy - 4, 1, 9, metal_d);
                // Door on the south face
                c.fill_rect(cx - 1, cy + 1, 3, 4, [40, 42, 50, 255]);
                c.put(cx + 1, cy + 3, [220, 188, 70, 255]);
            }
            // HVAC unit (top-left quadrant)
            let hx = 6;
            let hy = 6;
            if w > hx + 10 && h > hy + 8 {
                c.fill_rect(hx, hy, 9, 6, metal);
                c.fill_rect(hx, hy, 9, 1, metal_h);
                c.fill_rect(hx, hy + 5, 9, 1, metal_d);
                // Fan grille
                for fx in 0..3 {
                    c.fill_rect(hx + 1 + fx * 3, hy + 2, 1, 3, metal_d);
                }
            }
            // Satellite dish (bottom-right)
            let dx = w - 11;
            let dy = h - 11;
            if dx > 4 && dy > 4 {
                for ay in 0..7 {
                    for ax in 0..7 {
                        let ddx = ax - 3;
                        let ddy = ay - 3;
                        if ddx * ddx + ddy * ddy <= 9 {
                            c.put(dx + ax, dy + ay, metal);
                        }
                    }
                }
                c.put(dx + 3, dy + 3, metal_h);
                c.fill_rect(dx + 4, dy + 4, 1, 4, metal_d);
            }
            // Antenna mast with red warning light at the top.
            let ax = w * 3 / 4;
            let ay = 5;
            if ax > 4 && ax < w - 4 {
                c.fill_rect(ax, ay, 1, 8, metal_d);
                // Brace
                c.put(ax - 1, ay + 4, metal_d);
                c.put(ax + 1, ay + 4, metal_d);
                // Red warning lamp
                c.fill_rect(ax - 1, ay - 2, 3, 2, red_lamp);
                c.put(ax, ay - 3, red_lamp);
            }
            // Vent stacks (two short pipes lower-left)
            for vi in 0..2 {
                let vx = 5 + vi * 4;
                let vy = h - 9;
                if vy > 4 {
                    c.fill_rect(vx, vy, 3, 4, metal);
                    c.fill_rect(vx, vy, 3, 1, metal_h);
                    c.fill_rect(vx + 1, vy - 1, 1, 1, metal_d);
                }
            }
            // Drainage scupper near the bottom-right parapet
            if w >= 24 {
                c.fill_rect(w - 9, h - 4, 5, 1, [0, 0, 0, 180]);
            }
        }
        RoofStyle::Saw => {
            // Sawtooth factory roof: one slanted face (light) plus a near-
            // vertical glazed face per tooth, repeated across the long
            // dimension.  Each glazed face is a recessed strip of skylights
            // — clearly part of the roof surface, not "looking through" it.
            let face = roof;
            let face_d: Rgba = [
                ((roof[0] as i32 - 22).max(0)) as u8,
                ((roof[1] as i32 - 22).max(0)) as u8,
                ((roof[2] as i32 - 22).max(0)) as u8,
                255,
            ];
            let glass: Rgba = [120, 140, 160, 255];
            let glass_d: Rgba = [60, 70, 84, 255];
            let frame: Rgba = [54, 56, 62, 255];
            let tooth = 6;
            let mut y = 0;
            let mut alt = false;
            while y < h {
                let band_h = tooth.min(h - y);
                if alt {
                    // Glazed (recessed) face
                    c.fill_rect(0, y, w, band_h, frame);
                    c.fill_rect(0, y + 1, w, band_h.saturating_sub(2), glass_d);
                    // Mullions every 6 px
                    let mut x = 0;
                    while x < w {
                        c.fill_rect(x, y, 1, band_h, frame);
                        if x + 1 < w {
                            c.fill_rect(x + 1, y + 1, 1, band_h.saturating_sub(2), glass);
                        }
                        x += 6;
                    }
                } else {
                    c.fill_rect(0, y, w, band_h, face);
                    c.fill_rect(0, y, w, 1, face_d);
                    c.fill_rect(0, y + band_h - 1, w, 1, face_d);
                }
                y += tooth;
                alt = !alt;
            }
            // Outer parapet
            c.fill_rect(0, 0, w, 1, dark);
            c.fill_rect(0, h - 1, w, 1, dark);
            c.fill_rect(0, 0, 1, h, dark);
            c.fill_rect(w - 1, 0, 1, h, dark);
            // Two industrial chimney stacks (top-right area)
            c.fill_rect(w - 9, 2, 3, 7, trim);
            c.fill_rect(w - 9, 2, 3, 1, dark);
            c.fill_rect(w - 14, 2, 3, 7, trim);
            c.fill_rect(w - 14, 2, 3, 1, dark);
        }
        RoofStyle::Round => {
            // Cylindrical tank — solid dome with rivet ring, stenciled
            // hatch in the middle, ladder rungs on one side.
            let cx = w / 2;
            let cy = h / 2;
            let rx = w / 2 - 1;
            let ry = h / 2 - 1;
            let body_d: Rgba = [
                ((roof[0] as f32 * 0.6) as u8),
                ((roof[1] as f32 * 0.6) as u8),
                ((roof[2] as f32 * 0.6) as u8),
                255,
            ];
            let body_h: Rgba = [
                ((roof[0] as i32 + 30).min(255)) as u8,
                ((roof[1] as i32 + 30).min(255)) as u8,
                ((roof[2] as i32 + 30).min(255)) as u8,
                255,
            ];
            let rivet = [60, 60, 64, 255];
            for y in 0..h {
                for x in 0..w {
                    let ddx = (x - cx) as f32 / rx.max(1) as f32;
                    let ddy = (y - cy) as f32 / ry.max(1) as f32;
                    let r2 = ddx * ddx + ddy * ddy;
                    if r2 <= 1.0 {
                        // Shading: top-left highlight, bottom-right shadow.
                        let shade = ddx + ddy;
                        let col = if shade < -0.7 {
                            body_h
                        } else if shade > 0.7 {
                            body_d
                        } else {
                            roof
                        };
                        c.put(x, y, col);
                    }
                }
            }
            // Rivet ring just inside the rim
            for theta in (0..360).step_by(15) {
                let a = (theta as f32).to_radians();
                let px = cx as f32 + a.cos() * (rx as f32 - 2.0);
                let py = cy as f32 + a.sin() * (ry as f32 - 2.0);
                c.put(px as i32, py as i32, rivet);
            }
            // Hatch in the middle
            c.fill_rect(cx - 3, cy - 2, 6, 4, body_d);
            c.fill_rect(cx - 3, cy - 2, 6, 1, dark);
            c.fill_rect(cx - 3, cy + 1, 6, 1, dark);
            c.put(cx + 1, cy, rivet);
            // Ladder rungs along the right
            for r in 0..4 {
                c.fill_rect(cx + rx - 5, cy - 4 + r * 3, 4, 1, dark);
            }
        }
        RoofStyle::Tent => {
            // Pyramidal canvas: two triangles dark/light split by a ridge.
            let cx = w / 2;
            let cy = h / 2;
            for y in 0..h {
                let frac = (y - cy).abs() as f32 / (h / 2).max(1) as f32;
                let half_w = ((1.0 - frac) * (w / 2) as f32) as i32;
                if half_w < 1 {
                    continue;
                }
                let col = if y < cy { roof } else { dark };
                for x in (cx - half_w).max(0)..(cx + half_w).min(w) {
                    c.put(x, y, col);
                }
            }
            // Centre ridge
            c.fill_rect(cx - 1, 0, 2, h, [0, 0, 0, 200]);
        }
        RoofStyle::Pad => {
            c.fill_rect(0, 0, w, h, roof);
            c.fill_rect(0, 0, w, 2, dark);
            c.fill_rect(0, h - 2, w, 2, dark);
            // Yellow circle
            let cx = w / 2;
            let cy = h / 2;
            let r = ((w.min(h) - 6) / 2).max(4);
            for y in 0..h {
                for x in 0..w {
                    let dx = x - cx;
                    let dy = y - cy;
                    let d2 = dx * dx + dy * dy;
                    if d2 < r * r && d2 >= (r - 2) * (r - 2) {
                        c.put(x, y, trim);
                    }
                }
            }
            // "H" inside circle
            c.fill_rect(cx - 4, cy - 4, 2, 9, trim);
            c.fill_rect(cx + 2, cy - 4, 2, 9, trim);
            c.fill_rect(cx - 3, cy - 1, 6, 2, trim);
        }
    }
    c.into_image()
}

// ──── Gas station forecourt (canopy + pumps) ───────────────────────────

fn build_gas_canopy_image() -> Image {
    let mut c = Canvas::new(64, 24);
    c.fill_rect(0, 0, 64, 24, [0, 0, 0, 0]);
    c.fill_rect(0, 4, 64, 14, [196, 74, 42, 255]);
    c.fill_rect(0, 4, 64, 1, [122, 42, 24, 255]);
    c.fill_rect(0, 17, 64, 1, [122, 42, 24, 255]);
    // Support columns
    c.fill_rect(2, 0, 3, 24, [136, 136, 136, 255]);
    c.fill_rect(59, 0, 3, 24, [136, 136, 136, 255]);
    // Sign band
    c.fill_rect(20, 9, 24, 4, [255, 217, 61, 255]);
    c.into_image()
}

fn build_gas_pump_image() -> Image {
    let mut c = Canvas::new(20, 28);
    c.fill_rect(0, 0, 20, 28, [0, 0, 0, 0]);
    c.fill_rect(4, 4, 12, 20, [196, 74, 42, 255]);
    c.fill_rect(4, 4, 12, 1, [122, 42, 24, 255]);
    c.fill_rect(4, 23, 12, 1, [122, 42, 24, 255]);
    c.fill_rect(4, 4, 1, 20, [122, 42, 24, 255]);
    c.fill_rect(15, 4, 1, 20, [122, 42, 24, 255]);
    // Display
    c.fill_rect(6, 8, 8, 4, [40, 60, 90, 255]);
    // Nozzle
    c.fill_rect(12, 17, 6, 1, [40, 40, 48, 255]);
    c.fill_rect(17, 13, 1, 5, [40, 40, 48, 255]);
    c.into_image()
}

// ──── Segment fog + gate visuals ───────────────────────────────────────

fn build_segment_fog_image() -> Image {
    let mut c = Canvas::new(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            let n1 = (x * 13 + y * 17) % 11;
            let n2 = (x * 5 + y * 31) % 7;
            let v = (180 + (n1 - 5) * 3 + (n2 - 3) * 5).clamp(120, 230) as u8;
            c.put(x, y, [v, v, v.saturating_add(12), 255]);
        }
    }
    c.into_image()
}

fn build_gate_image(kind: GateKind) -> Image {
    let mut c = Canvas::new(32, 64);
    c.fill_rect(0, 0, 32, 64, [0, 0, 0, 0]);
    match kind {
        GateKind::Bridge => {
            // Two stone pillars + arch
            c.fill_rect(2, 8, 6, 48, [136, 132, 124, 255]);
            c.fill_rect(24, 8, 6, 48, [136, 132, 124, 255]);
            c.fill_rect(2, 8, 28, 4, [70, 66, 60, 255]);
            // Arch suggestion
            c.fill_rect(8, 12, 16, 6, [70, 66, 60, 255]);
        }
        GateKind::Breach => {
            // Pile of rubble with gap
            c.fill_rect(2, 0, 28, 28, [78, 70, 56, 255]);
            c.fill_rect(2, 36, 28, 28, [78, 70, 56, 255]);
            for &(x, y) in &[(6, 4), (12, 8), (18, 6), (24, 10), (8, 18), (14, 14), (22, 20),
                             (4, 40), (10, 44), (16, 48), (22, 52), (8, 58), (18, 60)] {
                c.put(x, y, [40, 36, 28, 255]);
            }
            // Hazard tape across gap
            for i in 0..16 {
                let col = if i % 2 == 0 { [220, 170, 40, 255] } else { [40, 32, 18, 255] };
                c.fill_rect(2 + i * 2, 30, 2, 4, col);
            }
        }
        GateKind::Tunnel => {
            // Concrete arch
            c.fill_rect(0, 0, 32, 8, [110, 108, 100, 255]);
            c.fill_rect(0, 56, 32, 8, [110, 108, 100, 255]);
            c.fill_rect(0, 0, 6, 64, [110, 108, 100, 255]);
            c.fill_rect(26, 0, 6, 64, [110, 108, 100, 255]);
            c.fill_rect(6, 28, 20, 8, [22, 24, 28, 255]);
        }
        GateKind::Gate => {
            // Military-style gate with red and white bar
            c.fill_rect(2, 0, 5, 64, [60, 64, 50, 255]);
            c.fill_rect(25, 0, 5, 64, [60, 64, 50, 255]);
            for i in 0..16 {
                let col = if i % 2 == 0 { [196, 74, 42, 255] } else { [232, 232, 220, 255] };
                c.fill_rect(7, 28 + (i % 2) * 4, 18, 2, col);
            }
            // Watchtower top
            c.fill_rect(4, 0, 24, 4, [40, 44, 30, 255]);
        }
    }
    c.into_image()
}

// ──── Prop rendering ───────────────────────────────────────────────────

fn prop_size_px(p: &Prop) -> Vec2 {
    Vec2::new(p.w as f32 * TILE_SIZE, p.h as f32 * TILE_SIZE)
}

fn prop_z(kind: PropKind) -> f32 {
    use PropKind::*;
    match kind {
        Blood | Debris | Oil | Crater => -13.0,
        Bush | HedgeH | HedgeV | Trash | Mailbox | Sign | Planter | Pallet
        | Crate | Barrels | BodyBag | Gurney | SandbagH | SandbagV | RazorH
        | RazorV | Flag => -1.5,
        Bench | Dumpster => -1.2,
        Car | Wreck | Bus | Truck | Ambulance | MilTruck | Jeep | Container
        | Crane | Forklift | Playground => -1.0,
        Tree => -0.6,
        Lamp => -0.4,
    }
}

fn prop_collision(p: &Prop) -> Option<ObstacleShape> {
    use PropKind::*;
    let half = Vec2::new(p.w as f32 * TILE_SIZE * 0.5, p.h as f32 * TILE_SIZE * 0.5);
    match p.kind {
        Tree => Some(ObstacleShape::Circle(10.0)),
        Bush => Some(ObstacleShape::Circle(14.0)),
        HedgeH | HedgeV => Some(ObstacleShape::Rect(half * 0.7)),
        Lamp => Some(ObstacleShape::Circle(5.0)),
        Mailbox => Some(ObstacleShape::Circle(8.0)),
        Trash => Some(ObstacleShape::Circle(10.0)),
        Dumpster => Some(ObstacleShape::Rect(half * 0.85)),
        Bench => Some(ObstacleShape::Rect(Vec2::new(half.x * 0.85, 8.0))),
        Sign => Some(ObstacleShape::Circle(5.0)),
        Planter => Some(ObstacleShape::Rect(half * 0.7)),
        Car | Wreck => Some(ObstacleShape::Rect(half * 0.85)),
        Bus => Some(ObstacleShape::Rect(half * 0.9)),
        Truck | Ambulance | MilTruck => Some(ObstacleShape::Rect(half * 0.9)),
        Jeep => Some(ObstacleShape::Rect(half * 0.85)),
        Container => Some(ObstacleShape::Rect(half * 0.95)),
        Barrels => Some(ObstacleShape::Circle(11.0)),
        Pallet => Some(ObstacleShape::Rect(half * 0.7)),
        Crate => Some(ObstacleShape::Rect(half * 0.7)),
        Crane => Some(ObstacleShape::Rect(half * 0.9)),
        Forklift => Some(ObstacleShape::Rect(half * 0.85)),
        Gurney => Some(ObstacleShape::Rect(half * 0.7)),
        BodyBag => None,
        Playground => Some(ObstacleShape::Rect(half * 0.85)),
        SandbagH | SandbagV => Some(ObstacleShape::Rect(half * 0.85)),
        RazorH | RazorV => Some(ObstacleShape::Rect(Vec2::new(half.x * 0.95, 4.0))),
        Flag => Some(ObstacleShape::Circle(4.0)),
        Crater => None,
        Blood | Debris | Oil => None,
    }
}

fn build_prop_image(kind: PropKind) -> Image {
    use PropKind::*;
    match kind {
        Tree => build_tree(),
        Bush => build_bush(),
        HedgeH => build_hedge(true),
        HedgeV => build_hedge(false),
        Planter => build_planter(),
        Car => build_car_civilian(),
        Wreck => build_car_wreck_red(),
        Bus => build_bus(),
        Truck => build_truck(),
        Ambulance => build_ambulance(),
        MilTruck => build_mil_truck(),
        Jeep => build_jeep(),
        Mailbox => build_mailbox(),
        Trash => build_trash_can(),
        Lamp => build_lamp(),
        Dumpster => build_dumpster(),
        Bench => build_bench(),
        Sign => build_sign(),
        Blood => build_blood(),
        Debris => build_debris(),
        Container => build_container(),
        Barrels => build_barrels(),
        Pallet => build_pallet(),
        Oil => build_oil_slick(),
        Crane => build_crane(),
        Forklift => build_forklift(),
        Crate => build_crate(),
        Gurney => build_gurney(),
        Playground => build_playground(),
        BodyBag => build_body_bag(),
        SandbagH => build_sandbag(true),
        SandbagV => build_sandbag(false),
        RazorH => build_razor(true),
        RazorV => build_razor(false),
        Crater => build_crater(),
        Flag => build_flag(),
    }
}

// ──── Individual prop sprites ──────────────────────────────────────────

fn build_tree() -> Image {
    let trunk: Rgba = [62, 40, 22, 255];
    let g1: Rgba = [14, 36, 20, 255];
    let g2: Rgba = [42, 82, 42, 255];
    let g3: Rgba = [88, 132, 68, 255];
    let shadow: Rgba = [0, 0, 0, 130];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_circle(16, 28, 8, shadow);
    c.fill_rect(14, 22, 4, 8, trunk);
    c.fill_circle(16, 14, 11, g1);
    c.fill_circle(16, 12, 9, g2);
    c.fill_circle(13, 9, 4, g3);
    c.fill_circle(20, 11, 3, g3);
    c.into_image()
}

fn build_bush() -> Image {
    let dark: Rgba = [38, 62, 30, 255];
    let base: Rgba = [64, 94, 52, 255];
    let light: Rgba = [108, 144, 70, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_circle(16, 17, 12, dark);
    c.fill_circle(16, 16, 11, base);
    c.fill_circle(11, 12, 4, light);
    c.fill_circle(20, 18, 4, light);
    c.into_image()
}

fn build_hedge(horizontal: bool) -> Image {
    let dark: Rgba = [38, 62, 30, 255];
    let base: Rgba = [70, 100, 50, 255];
    let light: Rgba = [108, 144, 70, 255];
    let (w, h) = if horizontal { (96, 32) } else { (32, 96) };
    let mut c = Canvas::new(w, h);
    c.fill_rect(0, 0, w, h, [0, 0, 0, 0]);
    let inset = 4;
    c.fill_rect(inset, inset, w - 2 * inset, h - 2 * inset, dark);
    c.fill_rect(inset + 1, inset + 1, w - 2 * inset - 2, h - 2 * inset - 2, base);
    if horizontal {
        for x in (inset + 4..w - inset - 4).step_by(8) {
            c.fill_rect(x, inset + 3, 4, h - 2 * inset - 6, light);
        }
    } else {
        for y in (inset + 4..h - inset - 4).step_by(8) {
            c.fill_rect(inset + 3, y, w - 2 * inset - 6, 4, light);
        }
    }
    c.into_image()
}

fn build_planter() -> Image {
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_rect(4, 12, 24, 16, [122, 78, 46, 255]);
    c.fill_rect(4, 12, 24, 1, [62, 38, 22, 255]);
    c.fill_rect(4, 27, 24, 1, [62, 38, 22, 255]);
    c.fill_rect(8, 6, 16, 8, [70, 100, 50, 255]);
    c.fill_circle(12, 8, 3, [220, 80, 80, 255]);
    c.fill_circle(20, 10, 3, [220, 220, 80, 255]);
    c.into_image()
}

fn build_car_civilian() -> Image {
    let body: Rgba = [62, 96, 144, 255];
    let dark: Rgba = [16, 22, 38, 255];
    let chrome: Rgba = [192, 196, 204, 255];
    let glass: Rgba = [82, 124, 160, 255];
    let tire: Rgba = [22, 22, 26, 255];
    let mut c = Canvas::new(64, 32);
    c.fill_rect(0, 0, 64, 32, [0, 0, 0, 0]);
    c.fill_rect(8, 4, 4, 5, tire);
    c.fill_rect(52, 4, 4, 5, tire);
    c.fill_rect(8, 23, 4, 5, tire);
    c.fill_rect(52, 23, 4, 5, tire);
    c.fill_rect(4, 6, 56, 20, body);
    c.fill_rect(20, 9, 24, 14, glass);
    c.fill_rect(4, 6, 56, 1, dark);
    c.fill_rect(4, 25, 56, 1, dark);
    c.fill_rect(4, 6, 1, 20, dark);
    c.fill_rect(59, 6, 1, 20, dark);
    c.fill_rect(60, 14, 2, 4, [248, 232, 152, 255]);
    c.fill_rect(2, 14, 2, 4, chrome);
    c.into_image()
}

fn build_car_wreck_red() -> Image {
    let body: Rgba = [148, 68, 46, 255];
    let dark: Rgba = [30, 18, 12, 255];
    let glass: Rgba = [62, 82, 102, 255];
    let rust: Rgba = [120, 60, 24, 255];
    let tire: Rgba = [22, 22, 26, 255];
    let mut c = Canvas::new(64, 32);
    c.fill_rect(0, 0, 64, 32, [0, 0, 0, 0]);
    for &x in &[8i32, 52] {
        c.fill_rect(x, 4, 4, 5, tire);
        c.fill_rect(x, 23, 4, 5, tire);
    }
    c.fill_rect(4, 6, 56, 20, body);
    c.fill_rect(20, 9, 24, 14, glass);
    for &(x, y, w, h) in &[(14, 7, 6, 2), (38, 23, 5, 2), (24, 11, 4, 1)] {
        c.fill_rect(x, y, w, h, rust);
    }
    c.fill_rect(4, 6, 56, 1, dark);
    c.fill_rect(4, 25, 56, 1, dark);
    c.fill_rect(56, 8, 4, 16, [22, 18, 12, 255]);
    c.into_image()
}

fn build_bus() -> Image {
    let yellow: Rgba = [214, 174, 52, 255];
    let dark: Rgba = [30, 22, 12, 255];
    let glass: Rgba = [62, 92, 110, 255];
    let tire: Rgba = [22, 22, 26, 255];
    let mut c = Canvas::new(96, 32);
    c.fill_rect(0, 0, 96, 32, [0, 0, 0, 0]);
    for &x in &[8i32, 28, 70, 86] {
        c.fill_rect(x, 4, 5, 5, tire);
        c.fill_rect(x, 23, 5, 5, tire);
    }
    c.fill_rect(4, 6, 88, 20, yellow);
    c.fill_rect(4, 6, 88, 1, dark);
    c.fill_rect(4, 25, 88, 1, dark);
    for x in (10..88).step_by(8) {
        c.fill_rect(x, 11, 6, 10, glass);
    }
    c.fill_rect(86, 12, 6, 8, glass);
    c.into_image()
}

fn build_truck() -> Image {
    let body: Rgba = [78, 78, 88, 255];
    let dark: Rgba = [30, 30, 36, 255];
    let cargo: Rgba = [122, 90, 60, 255];
    let glass: Rgba = [62, 82, 102, 255];
    let tire: Rgba = [22, 22, 26, 255];
    let mut c = Canvas::new(96, 64);
    c.fill_rect(0, 0, 96, 64, [0, 0, 0, 0]);
    c.fill_rect(8, 8, 60, 48, cargo);
    c.fill_rect(8, 8, 60, 1, dark);
    c.fill_rect(8, 55, 60, 1, dark);
    c.fill_rect(68, 16, 24, 32, body);
    c.fill_rect(72, 22, 16, 12, glass);
    for &x in &[16i32, 56, 80] {
        c.fill_rect(x, 0, 6, 8, tire);
        c.fill_rect(x, 56, 6, 8, tire);
    }
    c.into_image()
}

fn build_ambulance() -> Image {
    let body: Rgba = [232, 232, 232, 255];
    let dark: Rgba = [30, 30, 36, 255];
    let red: Rgba = [196, 50, 40, 255];
    let glass: Rgba = [62, 82, 102, 255];
    let tire: Rgba = [22, 22, 26, 255];
    let mut c = Canvas::new(64, 32);
    c.fill_rect(0, 0, 64, 32, [0, 0, 0, 0]);
    for &x in &[8i32, 52] {
        c.fill_rect(x, 4, 4, 5, tire);
        c.fill_rect(x, 23, 4, 5, tire);
    }
    c.fill_rect(4, 6, 56, 20, body);
    c.fill_rect(20, 9, 24, 14, glass);
    c.fill_rect(28, 13, 8, 2, red);
    c.fill_rect(31, 10, 2, 8, red);
    c.fill_rect(4, 6, 56, 1, dark);
    c.fill_rect(4, 25, 56, 1, dark);
    c.into_image()
}

fn build_mil_truck() -> Image {
    let body: Rgba = [60, 72, 42, 255];
    let dark: Rgba = [30, 36, 20, 255];
    let cargo: Rgba = [40, 48, 28, 255];
    let glass: Rgba = [40, 50, 38, 255];
    let tire: Rgba = [22, 22, 26, 255];
    let mut c = Canvas::new(96, 64);
    c.fill_rect(0, 0, 96, 64, [0, 0, 0, 0]);
    c.fill_rect(8, 8, 60, 48, cargo);
    c.fill_rect(8, 8, 60, 1, dark);
    c.fill_rect(8, 55, 60, 1, dark);
    // Cargo fabric ribs
    for y in (12..52).step_by(6) {
        c.fill_rect(10, y, 56, 1, dark);
    }
    c.fill_rect(68, 16, 24, 32, body);
    c.fill_rect(72, 22, 16, 12, glass);
    for &x in &[16i32, 56, 80] {
        c.fill_rect(x, 0, 6, 8, tire);
        c.fill_rect(x, 56, 6, 8, tire);
    }
    c.into_image()
}

fn build_jeep() -> Image {
    let body: Rgba = [70, 82, 50, 255];
    let dark: Rgba = [30, 36, 20, 255];
    let glass: Rgba = [40, 50, 38, 255];
    let tire: Rgba = [22, 22, 26, 255];
    let mut c = Canvas::new(64, 32);
    c.fill_rect(0, 0, 64, 32, [0, 0, 0, 0]);
    for &x in &[8i32, 52] {
        c.fill_rect(x, 4, 4, 5, tire);
        c.fill_rect(x, 23, 4, 5, tire);
    }
    c.fill_rect(4, 6, 56, 20, body);
    c.fill_rect(4, 6, 56, 1, dark);
    c.fill_rect(4, 25, 56, 1, dark);
    c.fill_rect(20, 9, 24, 14, glass);
    // Roll bar
    c.fill_rect(28, 4, 8, 2, dark);
    c.into_image()
}

fn build_mailbox() -> Image {
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_rect(13, 18, 6, 12, [78, 50, 30, 255]);
    c.fill_rect(8, 8, 16, 12, [60, 70, 108, 255]);
    c.fill_rect(8, 8, 16, 1, [110, 122, 170, 255]);
    c.fill_rect(8, 19, 16, 1, [28, 34, 60, 255]);
    c.fill_rect(11, 12, 10, 4, [28, 34, 60, 255]);
    c.fill_rect(24, 9, 4, 4, [220, 60, 50, 255]);
    c.into_image()
}

fn build_trash_can() -> Image {
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_rect(8, 10, 16, 18, [68, 72, 78, 255]);
    c.fill_rect(8, 10, 16, 1, [28, 30, 34, 255]);
    c.fill_rect(8, 27, 16, 1, [28, 30, 34, 255]);
    c.fill_rect(6, 8, 20, 4, [48, 52, 58, 255]);
    c.fill_rect(6, 8, 20, 1, [28, 30, 34, 255]);
    c.into_image()
}

fn build_lamp() -> Image {
    let mut c = Canvas::new(32, 64);
    c.fill_rect(0, 0, 32, 64, [0, 0, 0, 0]);
    c.fill_rect(13, 8, 6, 56, [80, 80, 88, 255]);
    c.fill_rect(13, 8, 1, 56, [30, 30, 34, 255]);
    c.fill_rect(8, 0, 16, 10, [40, 40, 44, 255]);
    c.fill_rect(10, 2, 12, 6, [228, 208, 128, 255]);
    c.fill_rect(13, 4, 6, 2, [252, 238, 180, 255]);
    c.into_image()
}

fn build_dumpster() -> Image {
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_rect(2, 10, 28, 20, [86, 112, 78, 255]);
    c.fill_rect(2, 10, 28, 1, [38, 54, 34, 255]);
    c.fill_rect(2, 29, 28, 1, [38, 54, 34, 255]);
    c.fill_rect(2, 6, 28, 4, [136, 160, 110, 255]);
    c.fill_rect(15, 6, 2, 4, [38, 54, 34, 255]);
    c.into_image()
}

fn build_bench() -> Image {
    let mut c = Canvas::new(64, 32);
    c.fill_rect(0, 0, 64, 32, [0, 0, 0, 0]);
    c.fill_rect(2, 14, 60, 4, [120, 82, 46, 255]);
    c.fill_rect(2, 18, 60, 4, [120, 82, 46, 255]);
    c.fill_rect(2, 14, 60, 1, [62, 40, 20, 255]);
    c.fill_rect(2, 21, 60, 1, [62, 40, 20, 255]);
    c.fill_rect(4, 22, 4, 8, [54, 52, 54, 255]);
    c.fill_rect(56, 22, 4, 8, [54, 52, 54, 255]);
    c.into_image()
}

fn build_sign() -> Image {
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_rect(15, 4, 2, 28, [110, 110, 118, 255]);
    c.fill_rect(8, 8, 16, 10, [220, 60, 50, 255]);
    c.fill_rect(8, 8, 16, 1, [110, 30, 24, 255]);
    c.fill_rect(8, 17, 16, 1, [110, 30, 24, 255]);
    c.fill_rect(11, 12, 10, 2, [240, 240, 240, 255]);
    c.into_image()
}

fn build_blood() -> Image {
    let dark: Rgba = [120, 18, 14, 255];
    let mid: Rgba = [160, 26, 20, 255];
    let light: Rgba = [200, 40, 28, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_circle(16, 16, 9, dark);
    c.fill_circle(16, 16, 7, mid);
    c.fill_circle(13, 13, 3, light);
    c.put(4, 22, dark);
    c.put(25, 8, mid);
    c.put(22, 26, dark);
    c.into_image()
}

fn build_debris() -> Image {
    let dark: Rgba = [68, 66, 62, 255];
    let mid: Rgba = [108, 104, 98, 255];
    let light: Rgba = [158, 154, 144, 255];
    let brick: Rgba = [148, 74, 48, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    for &(x, y, w, h, col) in &[
        (4, 6, 8, 6, mid), (14, 10, 10, 6, light), (22, 6, 7, 7, dark),
        (8, 18, 9, 6, brick), (18, 18, 7, 6, mid), (24, 22, 6, 6, light),
    ] {
        c.fill_rect(x, y, w, h, col);
    }
    c.into_image()
}

fn build_container() -> Image {
    let body: Rgba = [196, 74, 42, 255];
    let dark: Rgba = [80, 30, 18, 255];
    let bar: Rgba = [122, 42, 24, 255];
    let mut c = Canvas::new(96, 64);
    c.fill_rect(0, 0, 96, 64, [0, 0, 0, 0]);
    c.fill_rect(2, 4, 92, 56, body);
    c.fill_rect(2, 4, 92, 1, dark);
    c.fill_rect(2, 59, 92, 1, dark);
    c.fill_rect(2, 4, 1, 56, dark);
    c.fill_rect(93, 4, 1, 56, dark);
    for x in (8..88).step_by(8) {
        c.fill_rect(x, 6, 1, 52, bar);
    }
    c.fill_rect(40, 28, 16, 8, dark);
    c.into_image()
}

fn build_barrels() -> Image {
    let body: Rgba = [120, 66, 40, 255];
    let dark: Rgba = [68, 36, 20, 255];
    let ring: Rgba = [90, 90, 94, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_circle(16, 16, 13, dark);
    c.fill_circle(16, 16, 11, body);
    c.fill_rect(4, 11, 24, 1, ring);
    c.fill_rect(4, 21, 24, 1, ring);
    c.put(11, 9, [160, 94, 56, 255]);
    c.into_image()
}

/// Burnt-out wreck that takes damage and explodes — distinct silhouette
/// from the static `build_car_wreck_red` used for cosmetic decor (smashed
/// roof, smoke streaks, broken windows).
fn build_explodable_car_wreck_image() -> Image {
    let body: Rgba = [86, 36, 28, 255];
    let body_light: Rgba = [128, 60, 40, 255];
    let dark: Rgba = [22, 14, 10, 255];
    let glass: Rgba = [38, 50, 60, 255];
    let rust: Rgba = [108, 56, 22, 255];
    let tire: Rgba = [22, 22, 26, 255];
    let smoke: Rgba = [110, 110, 116, 200];
    let smoke_dark: Rgba = [60, 60, 66, 200];
    let warn: Rgba = [220, 80, 30, 255];

    let mut c = Canvas::new(64, 32);
    c.fill_rect(0, 0, 64, 32, [0, 0, 0, 0]);
    // Tires
    for &x in &[8i32, 52] {
        c.fill_rect(x, 4, 4, 5, tire);
        c.fill_rect(x, 23, 4, 5, tire);
    }
    // Body
    c.fill_rect(4, 6, 56, 20, body);
    c.fill_rect(4, 6, 56, 1, dark);
    c.fill_rect(4, 25, 56, 1, dark);
    c.fill_rect(4, 6, 1, 20, dark);
    c.fill_rect(59, 6, 1, 20, dark);
    // Crumpled hood (left side, smashed in)
    c.fill_rect(4, 12, 8, 8, dark);
    c.fill_rect(5, 13, 6, 6, body);
    c.fill_rect(5, 13, 6, 1, body_light);
    // Broken windshield
    c.fill_rect(20, 9, 24, 14, glass);
    c.fill_rect(24, 11, 4, 1, dark);
    c.fill_rect(32, 13, 6, 1, dark);
    c.fill_rect(28, 16, 8, 1, dark);
    c.fill_rect(38, 18, 4, 1, dark);
    // Rust patches
    for &(x, y, w, h) in &[(14, 7, 6, 2), (38, 23, 5, 2), (24, 11, 4, 1), (47, 8, 5, 2)] {
        c.fill_rect(x, y, w, h, rust);
    }
    // Smoke trails rising off the hood
    c.fill_circle(8, 4, 3, smoke_dark);
    c.fill_circle(12, 2, 2, smoke);
    c.fill_circle(15, 5, 2, smoke);
    // Hazard accents — easy spot-the-bomb cue for the player
    c.fill_rect(28, 6, 8, 1, warn);
    c.fill_rect(28, 25, 8, 1, warn);
    c.into_image()
}

/// Standalone fuel drum — single barrel painted bright red with a hazmat
/// stripe and warning triangle so the player can tell it apart from the
/// non-explodable industrial barrels in `build_barrels`.
fn build_explodable_fuel_barrel_image() -> Image {
    let body: Rgba = [196, 56, 36, 255];
    let body_light: Rgba = [232, 96, 60, 255];
    let body_dark: Rgba = [108, 28, 14, 255];
    let outline: Rgba = [12, 8, 6, 255];
    let ring: Rgba = [40, 40, 44, 255];
    let band: Rgba = [240, 220, 60, 255];
    let band_dark: Rgba = [40, 30, 8, 255];
    let cap: Rgba = [70, 70, 76, 255];

    let mut c = Canvas::new(28, 28);
    c.fill_rect(0, 0, 28, 28, [0, 0, 0, 0]);
    // Outline
    c.fill_rect(5, 3, 18, 23, outline);
    // Body fill
    c.fill_rect(6, 4, 16, 21, body);
    // Highlight (left rim)
    c.fill_rect(6, 4, 4, 20, body_light);
    c.fill_rect(7, 5, 1, 19, body_light);
    // Shadow (right rim)
    c.fill_rect(20, 4, 2, 21, body_dark);
    // Top rim
    c.fill_rect(6, 4, 16, 1, body_light);
    c.fill_rect(6, 24, 16, 1, body_dark);
    // Top cap
    c.fill_rect(11, 1, 6, 3, outline);
    c.fill_rect(12, 2, 4, 1, cap);
    // Steel bands top + bottom
    c.fill_rect(6, 8, 16, 1, ring);
    c.fill_rect(6, 19, 16, 1, ring);
    // Hazmat warning band
    c.fill_rect(6, 12, 16, 4, band);
    c.fill_rect(6, 12, 16, 1, band_dark);
    c.fill_rect(6, 15, 16, 1, band_dark);
    // Big "!" exclaim for warning (3 pixels stacked)
    c.put(13, 13, band_dark);
    c.put(13, 14, band_dark);
    c.into_image()
}

fn build_pallet() -> Image {
    let wood: Rgba = [138, 96, 54, 255];
    let dark: Rgba = [62, 40, 22, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_rect(2, 6, 28, 20, wood);
    for y in (8..24).step_by(4) {
        c.fill_rect(2, y, 28, 1, dark);
    }
    c.fill_rect(2, 6, 28, 1, dark);
    c.fill_rect(2, 25, 28, 1, dark);
    c.into_image()
}

fn build_oil_slick() -> Image {
    let dark: Rgba = [16, 14, 14, 255];
    let mid: Rgba = [30, 28, 28, 255];
    let sheen: Rgba = [80, 60, 100, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_circle(16, 16, 12, dark);
    c.fill_circle(16, 16, 10, mid);
    c.fill_circle(13, 13, 3, sheen);
    c.into_image()
}

fn build_crane() -> Image {
    let body: Rgba = [196, 162, 40, 255];
    let dark: Rgba = [88, 70, 16, 255];
    let metal: Rgba = [110, 110, 116, 255];
    let mut c = Canvas::new(64, 64);
    c.fill_rect(0, 0, 64, 64, [0, 0, 0, 0]);
    c.fill_rect(20, 40, 24, 24, body);
    c.fill_rect(20, 40, 24, 1, dark);
    c.fill_rect(28, 8, 8, 36, body);
    c.fill_rect(28, 8, 8, 1, dark);
    // Boom
    c.fill_rect(2, 12, 28, 4, metal);
    c.fill_rect(2, 16, 4, 12, metal);
    c.into_image()
}

fn build_forklift() -> Image {
    let body: Rgba = [232, 134, 40, 255];
    let dark: Rgba = [120, 60, 14, 255];
    let metal: Rgba = [110, 110, 116, 255];
    let tire: Rgba = [22, 22, 26, 255];
    let mut c = Canvas::new(64, 32);
    c.fill_rect(0, 0, 64, 32, [0, 0, 0, 0]);
    c.fill_rect(20, 6, 30, 22, body);
    c.fill_rect(20, 6, 30, 1, dark);
    c.fill_rect(20, 27, 30, 1, dark);
    c.fill_rect(50, 0, 6, 32, metal);
    c.fill_rect(54, 4, 8, 1, metal);
    c.fill_rect(54, 27, 8, 1, metal);
    for &x in &[24i32, 42] {
        c.fill_rect(x, 4, 5, 4, tire);
        c.fill_rect(x, 24, 5, 4, tire);
    }
    c.into_image()
}

fn build_crate() -> Image {
    let body: Rgba = [138, 94, 56, 255];
    let dark: Rgba = [82, 54, 30, 255];
    let light: Rgba = [176, 124, 78, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_rect(4, 4, 24, 24, body);
    c.fill_rect(4, 4, 24, 1, dark);
    c.fill_rect(4, 27, 24, 1, dark);
    c.fill_rect(4, 4, 1, 24, dark);
    c.fill_rect(27, 4, 1, 24, dark);
    c.fill_rect(4, 15, 24, 1, dark);
    c.fill_rect(15, 4, 1, 24, dark);
    c.fill_rect(6, 6, 6, 1, light);
    c.fill_rect(18, 18, 6, 1, light);
    c.into_image()
}

fn build_gurney() -> Image {
    let frame: Rgba = [110, 110, 116, 255];
    let sheet: Rgba = [232, 232, 232, 255];
    let red: Rgba = [196, 30, 30, 255];
    let mut c = Canvas::new(32, 64);
    c.fill_rect(0, 0, 32, 64, [0, 0, 0, 0]);
    c.fill_rect(4, 4, 24, 56, sheet);
    c.fill_rect(4, 4, 24, 1, frame);
    c.fill_rect(4, 59, 24, 1, frame);
    c.fill_rect(4, 4, 1, 56, frame);
    c.fill_rect(27, 4, 1, 56, frame);
    c.fill_rect(8, 24, 16, 4, red);
    // Wheels
    for &y in &[6i32, 56] {
        c.fill_rect(2, y, 4, 4, [22, 22, 26, 255]);
        c.fill_rect(26, y, 4, 4, [22, 22, 26, 255]);
    }
    c.into_image()
}

fn build_playground() -> Image {
    let frame: Rgba = [196, 74, 42, 255];
    let blue: Rgba = [60, 100, 196, 255];
    let yellow: Rgba = [255, 217, 61, 255];
    let mut c = Canvas::new(64, 64);
    c.fill_rect(0, 0, 64, 64, [0, 0, 0, 0]);
    c.fill_rect(8, 12, 48, 8, frame);
    c.fill_rect(28, 20, 8, 32, blue);
    c.fill_rect(8, 50, 16, 4, yellow);
    c.fill_rect(40, 50, 16, 4, yellow);
    c.fill_rect(28, 4, 4, 8, frame);
    c.fill_rect(34, 4, 4, 8, frame);
    c.into_image()
}

fn build_body_bag() -> Image {
    let mut c = Canvas::new(32, 64);
    c.fill_rect(0, 0, 32, 64, [0, 0, 0, 0]);
    c.fill_rect(6, 4, 20, 56, [38, 40, 42, 255]);
    c.fill_rect(6, 4, 20, 1, [10, 10, 12, 255]);
    c.fill_rect(6, 59, 20, 1, [10, 10, 12, 255]);
    c.fill_rect(6, 4, 1, 56, [10, 10, 12, 255]);
    c.fill_rect(25, 4, 1, 56, [10, 10, 12, 255]);
    // Zipper
    c.fill_rect(15, 6, 2, 52, [110, 110, 116, 255]);
    c.into_image()
}

fn build_sandbag(horizontal: bool) -> Image {
    let base: Rgba = [196, 172, 118, 255];
    let dark: Rgba = [128, 106, 68, 255];
    let light: Rgba = [236, 210, 152, 255];
    let (w, h) = if horizontal { (96, 32) } else { (32, 96) };
    let mut c = Canvas::new(w, h);
    c.fill_rect(0, 0, w, h, [0, 0, 0, 0]);
    if horizontal {
        // 3 bags side by side, two rows
        for row in 0..2i32 {
            let oy = 4 + row * 12;
            for i in 0..3 {
                let x = 4 + i * 30 + row * 6;
                c.fill_rect(x, oy, 28, 10, base);
                c.fill_rect(x, oy, 28, 1, dark);
                c.fill_rect(x, oy + 9, 28, 1, dark);
                c.fill_rect(x + 1, oy + 1, 26, 1, light);
            }
        }
    } else {
        for col in 0..2i32 {
            let ox = 4 + col * 12;
            for i in 0..3 {
                let y = 4 + i * 30 + col * 6;
                c.fill_rect(ox, y, 10, 28, base);
                c.fill_rect(ox, y, 1, 28, dark);
                c.fill_rect(ox + 9, y, 1, 28, dark);
                c.fill_rect(ox + 1, y + 1, 1, 26, light);
            }
        }
    }
    c.into_image()
}

fn build_razor(horizontal: bool) -> Image {
    let wire: Rgba = [170, 168, 170, 255];
    let dark: Rgba = [70, 68, 70, 255];
    let (w, h) = if horizontal { (128, 32) } else { (32, 128) };
    let mut c = Canvas::new(w, h);
    c.fill_rect(0, 0, w, h, [0, 0, 0, 0]);
    if horizontal {
        // Two parallel wires + barbs
        c.fill_rect(0, 12, w, 1, wire);
        c.fill_rect(0, 20, w, 1, wire);
        for x in (4..w - 2).step_by(8) {
            c.put(x, 8, dark);
            c.put(x, 24, dark);
            c.put(x - 1, 16, dark);
            c.put(x + 1, 16, dark);
        }
    } else {
        c.fill_rect(12, 0, 1, h, wire);
        c.fill_rect(20, 0, 1, h, wire);
        for y in (4..h - 2).step_by(8) {
            c.put(8, y, dark);
            c.put(24, y, dark);
            c.put(16, y - 1, dark);
            c.put(16, y + 1, dark);
        }
    }
    c.into_image()
}

fn build_crater() -> Image {
    let dark: Rgba = [42, 36, 28, 255];
    let mid: Rgba = [78, 66, 50, 255];
    let light: Rgba = [128, 112, 86, 255];
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_circle(16, 16, 13, light);
    c.fill_circle(16, 16, 10, mid);
    c.fill_circle(16, 16, 7, dark);
    c.into_image()
}

fn build_flag() -> Image {
    let mut c = Canvas::new(32, 32);
    c.fill_rect(0, 0, 32, 32, [0, 0, 0, 0]);
    c.fill_rect(15, 0, 2, 32, [80, 80, 88, 255]);
    c.fill_rect(17, 4, 12, 8, [196, 50, 40, 255]);
    c.fill_rect(17, 4, 12, 1, [88, 24, 18, 255]);
    c.fill_rect(17, 11, 12, 1, [88, 24, 18, 255]);
    c.into_image()
}

#[cfg(test)]
mod tests {
    use super::*;
    // Internal-only test helper — `bfs_distance_field_bounded` isn't part
    // of the public `crate::map` re-export surface, so we reach into the
    // sibling module directly.
    use crate::map_nav::bfs_distance_field_bounded;

    #[test]
    fn obstacle_grid_finds_circle_inside_one_cell() {
        let mut o = MapObstacles::default();
        o.list.push(Obstacle {
            pos: Vec2::new(100.0, 100.0),
            shape: ObstacleShape::Circle(20.0),
        });
        o.rebuild_grid();
        assert!(o.hits(Vec2::new(105.0, 100.0), 5.0));
        assert!(!o.hits(Vec2::new(500.0, 500.0), 5.0));
    }

    #[test]
    fn obstacle_grid_handles_circle_spanning_cells() {
        let mut o = MapObstacles::default();
        o.list.push(Obstacle {
            pos: Vec2::ZERO,
            shape: ObstacleShape::Circle(300.0),
        });
        o.rebuild_grid();
        assert!(o.hits(Vec2::new(250.0, 0.0), 10.0));
        assert!(!o.hits(Vec2::new(320.0, 0.0), 5.0));
    }

    #[test]
    fn obstacle_grid_resolve_pushes_circle_out() {
        let mut o = MapObstacles::default();
        o.list.push(Obstacle {
            pos: Vec2::ZERO,
            shape: ObstacleShape::Circle(20.0),
        });
        o.rebuild_grid();
        let mut p = Vec2::new(15.0, 0.0);
        o.resolve(&mut p, 5.0);
        assert!(p.length() >= 24.9, "resolved pos length {} < 25", p.length());
    }

    #[test]
    fn obstacle_grid_skips_zero_radius_circles() {
        let mut o = MapObstacles::default();
        o.list.push(Obstacle {
            pos: Vec2::ZERO,
            shape: ObstacleShape::Circle(0.0),
        });
        o.rebuild_grid();
        assert!(!o.hits(Vec2::ZERO, 5.0));
    }

    #[test]
    fn bfs_distance_field_zero_at_start() {
        let total = (MAP_COLS * MAP_ROWS) as usize;
        let walkable = vec![true; total];
        let dist = bfs_distance_field_bounded(&walkable, Vec2::ZERO, 5);
        let (sc, sr) = world_to_tile(Vec2::ZERO);
        assert_eq!(dist[nav_idx(sc, sr)], 0);
    }

    #[test]
    fn bfs_distance_field_respects_walls() {
        let total = (MAP_COLS * MAP_ROWS) as usize;
        let mut walkable = vec![true; total];
        // Wall everything in row 23 — BFS from below shouldn't reach row 24+.
        for c in 0..MAP_COLS {
            walkable[nav_idx(c, 23)] = false;
        }
        let start = tile_center(120, 0);
        let dist = bfs_distance_field_bounded(&walkable, start, 100);
        assert_eq!(dist[nav_idx(120, 0)], 0);
        assert_ne!(dist[nav_idx(120, 22)], u16::MAX);
        assert_eq!(dist[nav_idx(120, 24)], u16::MAX);
    }

    #[test]
    fn bfs_distance_field_bounded_caps_distance() {
        let total = (MAP_COLS * MAP_ROWS) as usize;
        let walkable = vec![true; total];
        let start = tile_center(120, 24);
        let dist = bfs_distance_field_bounded(&walkable, start, 5);
        let (c, r) = world_to_tile(start);
        // Tile 7 away should NOT be reached (max_dist=5).
        assert_eq!(dist[nav_idx(c + 7, r)], u16::MAX);
        // Tile within radius is reached.
        assert_ne!(dist[nav_idx(c + 3, r)], u16::MAX);
    }

    #[test]
    fn nav_idx_in_bounds_round_trip() {
        for col in [0, 1, MAP_COLS / 2, MAP_COLS - 1] {
            for row in [0, MAP_ROWS / 2, MAP_ROWS - 1] {
                assert!(in_bounds(col, row));
                let _ = nav_idx(col, row);
            }
        }
        assert!(!in_bounds(-1, 0));
        assert!(!in_bounds(0, MAP_ROWS));
    }
}

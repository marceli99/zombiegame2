//! Tile-grid navigation: walkability mask + bounded BFS distance fields
//! used by zombie pathfinding.
//!
//! Lives in its own module (vs. inline in `map.rs`) so the BFS and the
//! `NavGrid` resource are easy to navigate to and test in isolation.
//! Re-exported from `map.rs` so existing imports keep working.

use bevy::prelude::*;
use std::collections::{HashMap, VecDeque};

use crate::map::{
    in_bounds, is_walkable_tile, nav_idx, tile_center, world_to_tile, MapObstacles, MAP_COLS,
    MAP_ROWS,
};

#[derive(Resource)]
pub struct NavGrid {
    /// Static walkability from building/perimeter wall rects only — never
    /// changes after startup.
    pub walkable: Vec<bool>,
    /// `walkable` minus tiles blocked by `MapObstacles` entries (trees,
    /// props, wrecks, gates…), kept fresh by `refresh_nav_blocked`.  This is
    /// what BFS flow fields and spawn snapping consume, so zombie paths
    /// route *around* props instead of ramming them and relying on local
    /// steering to recover.
    pub effective: Vec<bool>,
    pub player_flow: HashMap<u8, Vec<u16>>,
    pub player_flow_tile: HashMap<u8, (i32, i32)>,
    /// Obstacles changed since the last `effective` bake.
    bake_pending: bool,
    /// Seconds until the next bake is allowed (debounce).  Lives here (not
    /// in a `Local`) so `expedite_nav_bake` can zero it on session entry —
    /// a leftover cooldown from the previous session must not delay baking
    /// the OnEnter obstacle resets (respawned wrecks, re-locked gates).
    bake_cooldown: f32,
}

impl Default for NavGrid {
    fn default() -> Self {
        let total = (MAP_COLS * MAP_ROWS) as usize;
        let mut walkable = vec![false; total];
        for row in 0..MAP_ROWS {
            for col in 0..MAP_COLS {
                walkable[(row * MAP_COLS + col) as usize] = is_walkable_tile(col, row);
            }
        }
        Self {
            effective: walkable.clone(),
            walkable,
            player_flow: HashMap::new(),
            player_flow_tile: HashMap::new(),
            bake_pending: false,
            bake_cooldown: 0.0,
        }
    }
}

/// OnEnter(Playing): drop any leftover debounce so the session's first
/// `refresh_nav_blocked` run bakes the freshly reset obstacles immediately.
pub fn expedite_nav_bake(mut nav: ResMut<NavGrid>) {
    nav.bake_cooldown = 0.0;
}

/// Clearance radius (world px) used when baking `MapObstacles` into the nav
/// grid: a tile is blocked when an obstacle overlaps a circle of this radius
/// at the tile centre.  Matches the Normal-zombie radius — bigger kinds
/// (Giant, r=20) may still get flow through gaps they can't fit and fall
/// back to local steering there, which is the pre-existing behavior.
pub const NAV_OBSTACLE_CLEARANCE: f32 = 10.0;

/// Minimum seconds between `effective`-mask recomputes.  Obstacle mutations
/// can cluster (e.g. every bullet chipping a wreck flags the resource), and
/// one full pass is ~12k grid queries — debouncing keeps that off the
/// per-frame budget while nav still reacts to a destroyed wreck or an
/// unlocked gate well within human reaction time.
const NAV_REFRESH_COOLDOWN: f32 = 0.35;

/// Re-bakes `NavGrid::effective` whenever `MapObstacles` changed, then
/// invalidates the cached per-player flow tiles so `update_nav_flow`
/// rebuilds its BFS fields against the new mask on its next run.
pub fn refresh_nav_blocked(
    time: Res<Time>,
    obstacles: Res<MapObstacles>,
    mut nav: ResMut<NavGrid>,
) {
    let nav = &mut *nav;
    if obstacles.is_changed() {
        nav.bake_pending = true;
    }
    nav.bake_cooldown = (nav.bake_cooldown - time.delta_seconds()).max(0.0);
    if !nav.bake_pending || nav.bake_cooldown > 0.0 {
        return;
    }
    nav.bake_pending = false;
    nav.bake_cooldown = NAV_REFRESH_COOLDOWN;

    for row in 0..MAP_ROWS {
        for col in 0..MAP_COLS {
            let i = nav_idx(col, row);
            nav.effective[i] = nav.walkable[i]
                && !obstacles.hits(tile_center(col, row), NAV_OBSTACLE_CLEARANCE);
        }
    }
    // Stale flow fields stay usable for the frames until their rebuild —
    // only the tile cache is cleared so every alive player's field is
    // recomputed on the next `update_nav_flow` pass.
    nav.player_flow_tile.clear();
}

/// Maximum BFS radius (in tiles) for the per-player flow field.  60 tiles
/// = ~1920 px ≈ 3 viewport widths.  Zombies further out fall through to the
/// straight-line fallback in `zombie_flow_direction`, which is fine because
/// they're well outside the player's awareness anyway and don't need clean
/// path planning.  Cap exists because BFS over the full 240×48 grid was
/// the dominant CPU cost of `update_nav_flow` — capping cuts visited tiles
/// from ~11 520 to a few thousand.
pub const NAV_FLOW_MAX_RADIUS_TILES: u16 = 60;

pub fn bfs_distance_field(walkable: &[bool], start: Vec2) -> Vec<u16> {
    bfs_distance_field_bounded(walkable, start, NAV_FLOW_MAX_RADIUS_TILES)
}

pub fn bfs_distance_field_bounded(
    walkable: &[bool],
    start: Vec2,
    max_dist: u16,
) -> Vec<u16> {
    let total = (MAP_COLS * MAP_ROWS) as usize;
    let mut dist = vec![u16::MAX; total];
    let (sc, sr) = world_to_tile(start);
    let (sc, sr) = snap_to_walkable(walkable, sc, sr);
    if !in_bounds(sc, sr) || !walkable[nav_idx(sc, sr)] {
        return dist;
    }
    dist[nav_idx(sc, sr)] = 0;
    // Capacity tuned to the bounded-BFS reach (≈π·r² tiles inside max_dist).
    // For the default radius of 60 that's ~11000, but the actual queue only
    // ever holds the wave-front so `with_capacity(512)` is a safe starting
    // point — VecDeque reallocates if needed.
    let mut queue: VecDeque<(i32, i32)> = VecDeque::with_capacity(512);
    queue.push_back((sc, sr));
    let dirs: [(i32, i32); 8] = [
        (-1, 0), (1, 0), (0, -1), (0, 1),
        (-1, -1), (-1, 1), (1, -1), (1, 1),
    ];
    while let Some((c, r)) = queue.pop_front() {
        let d = dist[nav_idx(c, r)];
        // Don't expand past the radius — neighbours stay at u16::MAX
        // and fall through to the straight-line steer fallback.
        if d >= max_dist {
            continue;
        }
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
    for ring in 1_i32..=8 {
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

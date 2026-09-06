//! Spatial-grid–accelerated obstacle collision used by everything in the
//! sim that needs to know "is this point inside a wall / prop?" — bullets,
//! zombies, throwables, the player.
//!
//! Lives in its own module (vs. inline in `map.rs`) because it's
//! standalone, hot-path code that's easier to reason about and test in
//! isolation than buried at line ~120 of a 4500-line file.  Re-exported
//! from `map.rs` so existing call sites (`use crate::map::MapObstacles`)
//! keep working unchanged.

use bevy::prelude::*;

use crate::map::{MAP_HEIGHT, MAP_WIDTH};

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

/// Side length of one spatial-grid cell, in world px.  Picked to comfortably
/// hold the largest "small" obstacle (props ≤32 px radius) inside one cell
/// while still covering big building rects in only ~12 cells.
pub const OBSTACLE_GRID_CELL: f32 = 128.0;

#[derive(Default)]
struct ObstacleGrid {
    cells: Vec<Vec<u32>>,
    cols: usize,
    rows: usize,
}

#[inline]
fn obstacle_aabb(o: &Obstacle) -> (Vec2, Vec2) {
    match o.shape {
        ObstacleShape::Circle(r) => (
            Vec2::new(o.pos.x - r, o.pos.y - r),
            Vec2::new(o.pos.x + r, o.pos.y + r),
        ),
        ObstacleShape::Rect(half) => (o.pos - half, o.pos + half),
    }
}

impl ObstacleGrid {
    #[inline]
    fn world_to_cell(p: Vec2) -> (i32, i32) {
        // Both axes are offset by half the map size so every in-map position
        // lands at a non-negative cell index.
        let cx = ((p.x + MAP_WIDTH * 0.5) / OBSTACLE_GRID_CELL).floor() as i32;
        let cy = ((p.y + MAP_HEIGHT * 0.5) / OBSTACLE_GRID_CELL).floor() as i32;
        (cx, cy)
    }

    fn rebuild(&mut self, list: &[Obstacle]) {
        self.cols = ((MAP_WIDTH / OBSTACLE_GRID_CELL).ceil() as usize) + 1;
        self.rows = ((MAP_HEIGHT / OBSTACLE_GRID_CELL).ceil() as usize) + 1;
        let total = self.cols * self.rows;
        self.cells.clear();
        self.cells.resize_with(total, Vec::new);
        for (i, o) in list.iter().enumerate() {
            // Skip zero-radius "removed" obstacles to keep cells lean.
            if matches!(o.shape, ObstacleShape::Circle(r) if r <= 0.0) {
                continue;
            }
            let (min, max) = obstacle_aabb(o);
            let (c0, r0) = Self::world_to_cell(min);
            let (c1, r1) = Self::world_to_cell(max);
            let cs = c0.max(0);
            let ce = c1.min(self.cols as i32 - 1);
            let rs = r0.max(0);
            let re = r1.min(self.rows as i32 - 1);
            for r in rs..=re {
                for c in cs..=ce {
                    self.cells[r as usize * self.cols + c as usize].push(i as u32);
                }
            }
        }
    }
}

#[derive(Resource, Default)]
pub struct MapObstacles {
    pub list: Vec<Obstacle>,
    grid: ObstacleGrid,
}

#[inline]
fn resolve_one(o: &Obstacle, pos: &mut Vec2, own_radius: f32) {
    match o.shape {
        ObstacleShape::Circle(r) => {
            if r <= 0.0 {
                return;
            }
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

#[inline]
fn hits_one(o: &Obstacle, pos: Vec2, radius: f32) -> bool {
    match o.shape {
        ObstacleShape::Circle(r) => {
            if r <= 0.0 {
                return false;
            }
            let min_d = r + radius;
            pos.distance_squared(o.pos) < min_d * min_d
        }
        ObstacleShape::Rect(half) => {
            let delta = pos - o.pos;
            let clamped = Vec2::new(
                delta.x.clamp(-half.x, half.x),
                delta.y.clamp(-half.y, half.y),
            );
            let closest = o.pos + clamped;
            pos.distance_squared(closest) < radius * radius
        }
    }
}

impl MapObstacles {
    /// Re-bin every obstacle into the spatial grid.  Cheap (~O(N × cells_per_obstacle)
    /// — typical run is well under a millisecond even for ~1000 entries).
    /// Call after any mutation that adds/removes entries, or whose shape
    /// AABB changes.  Pure shape→Circle(0) transitions don't need a rebuild
    /// (the grid query short-circuits via `hits_one`/`resolve_one` instead).
    pub fn rebuild_grid(&mut self) {
        self.grid.rebuild(&self.list);
    }

    pub fn resolve(&self, pos: &mut Vec2, own_radius: f32) {
        // Fallback: empty grid (during initial load before rebuild) — scan all.
        if self.grid.cells.is_empty() {
            for o in &self.list {
                resolve_one(o, pos, own_radius);
            }
            return;
        }
        // Resolve is idempotent: scanning the same obstacle twice is harmless
        // (the second pass sees the post-resolve position and is a no-op),
        // so we don't need to deduplicate across overlapping cells.
        let lo = Vec2::new(pos.x - own_radius, pos.y - own_radius);
        let hi = Vec2::new(pos.x + own_radius, pos.y + own_radius);
        let (c0, r0) = ObstacleGrid::world_to_cell(lo);
        let (c1, r1) = ObstacleGrid::world_to_cell(hi);
        let cs = c0.max(0) as usize;
        let ce = (c1.min(self.grid.cols as i32 - 1)).max(0) as usize;
        let rs = r0.max(0) as usize;
        let re = (r1.min(self.grid.rows as i32 - 1)).max(0) as usize;
        for r in rs..=re {
            let row_off = r * self.grid.cols;
            for c in cs..=ce {
                let cell = &self.grid.cells[row_off + c];
                for &idx in cell {
                    // SAFETY: indices are populated from list iteration so
                    // they're always in-bounds.  We only mutate `shape`
                    // post-build, never resize the list during lookup.
                    let o = unsafe { self.list.get_unchecked(idx as usize) };
                    resolve_one(o, pos, own_radius);
                }
            }
        }
    }

    pub fn remove_at(&mut self, pos: Vec2) {
        self.list.retain(|o| o.pos.distance_squared(pos) > 4.0);
        self.rebuild_grid();
    }

    /// Cheap intersection test: true if a circle of `radius` centred at `pos`
    /// overlaps any obstacle in the list.  Used by bullets, zombies and
    /// throwables.
    pub fn hits(&self, pos: Vec2, radius: f32) -> bool {
        if self.grid.cells.is_empty() {
            for o in &self.list {
                if hits_one(o, pos, radius) {
                    return true;
                }
            }
            return false;
        }
        let lo = Vec2::new(pos.x - radius, pos.y - radius);
        let hi = Vec2::new(pos.x + radius, pos.y + radius);
        let (c0, r0) = ObstacleGrid::world_to_cell(lo);
        let (c1, r1) = ObstacleGrid::world_to_cell(hi);
        let cs = c0.max(0) as usize;
        let ce = (c1.min(self.grid.cols as i32 - 1)).max(0) as usize;
        let rs = r0.max(0) as usize;
        let re = (r1.min(self.grid.rows as i32 - 1)).max(0) as usize;
        for r in rs..=re {
            let row_off = r * self.grid.cols;
            for c in cs..=ce {
                let cell = &self.grid.cells[row_off + c];
                for &idx in cell {
                    let o = unsafe { self.list.get_unchecked(idx as usize) };
                    if hits_one(o, pos, radius) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic LCG (Knuth MMIX constants) so the fuzz layout is
    /// identical on every run and platform — no `rand`, no time seeds.
    struct Lcg(u64);

    impl Lcg {
        fn next_f32(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            // Top 24 bits → uniform in [0, 1).
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * self.next_f32()
        }
    }

    /// ~50 mixed Circle+Rect obstacles spread over the whole map rect —
    /// the exact range `world_to_cell` offsets by.
    fn mixed_obstacle_set() -> Vec<Obstacle> {
        let mut rng = Lcg(0x5EED_2026);
        let mut list = Vec::new();
        for i in 0..50 {
            let pos = Vec2::new(
                rng.range(-MAP_WIDTH * 0.5 + 64.0, MAP_WIDTH * 0.5 - 64.0),
                rng.range(-MAP_HEIGHT * 0.5 + 64.0, MAP_HEIGHT * 0.5 - 64.0),
            );
            let shape = if i % 2 == 0 {
                ObstacleShape::Circle(rng.range(4.0, 40.0))
            } else {
                ObstacleShape::Rect(Vec2::new(rng.range(8.0, 80.0), rng.range(8.0, 80.0)))
            };
            list.push(Obstacle { pos, shape });
        }
        list
    }

    /// The grid-accelerated `hits` must agree with the brute-force fallback
    /// (empty grid) for every query — probing a lattice that straddles the
    /// 128-px cell edges (±1 px around each boundary plus cell centres)
    /// across the whole map.  A cell-binning off-by-one
    /// here means players/bullets clip through walls near cell borders, and
    /// a stale index makes the `get_unchecked` deref UB.
    #[test]
    fn grid_and_bruteforce_agree_on_hits() {
        let list = mixed_obstacle_set();
        let mut brute = MapObstacles::default();
        brute.list = list.clone(); // grid never built → fallback scan
        let mut grid = MapObstacles::default();
        grid.list = list;
        grid.rebuild_grid();

        let x0 = -MAP_WIDTH * 0.5;
        let y0 = -MAP_HEIGHT * 0.5;
        let cols = (MAP_WIDTH / OBSTACLE_GRID_CELL).ceil() as i32;
        let rows = (MAP_HEIGHT / OBSTACLE_GRID_CELL).ceil() as i32;
        let offsets = [-1.0, 0.0, 1.0, OBSTACLE_GRID_CELL * 0.5];
        let mut hit_count = 0u32;
        for cx in 0..=cols {
            for cy in 0..=rows {
                for &ox in &offsets {
                    for &oy in &offsets {
                        let p = Vec2::new(
                            x0 + cx as f32 * OBSTACLE_GRID_CELL + ox,
                            y0 + cy as f32 * OBSTACLE_GRID_CELL + oy,
                        );
                        for radius in [5.0, 18.0] {
                            let b = brute.hits(p, radius);
                            let g = grid.hits(p, radius);
                            assert_eq!(b, g, "hits mismatch at {p:?} r={radius}");
                            hit_count += g as u32;
                        }
                    }
                }
            }
        }
        // Sanity: the lattice actually intersected obstacles — an all-false
        // sweep would make the equivalence vacuous.
        assert!(hit_count > 0, "fuzz lattice never hit an obstacle");
    }

    /// `resolve` postcondition for a single obstacle: wherever the entity
    /// starts (including deep inside), it ends up non-colliding.  Allows
    /// 0.01 px of float slack against the strict `<` in `hits`.
    #[test]
    fn resolve_ends_outside_single_obstacle() {
        let shapes = [
            ObstacleShape::Circle(30.0),
            ObstacleShape::Rect(Vec2::new(50.0, 20.0)),
        ];
        for shape in shapes {
            let mut o = MapObstacles::default();
            o.list.push(Obstacle { pos: Vec2::new(200.0, -100.0), shape });
            o.rebuild_grid();
            let mut rng = Lcg(7);
            for _ in 0..200 {
                let mut p = Vec2::new(rng.range(120.0, 280.0), rng.range(-180.0, -20.0));
                o.resolve(&mut p, 10.0);
                assert!(!o.hits(p, 10.0 - 0.01), "resolve left {p:?} colliding");
            }
        }
    }

    /// Deep-penetration Rect branch (entity centre strictly inside the
    /// rect): pushed fully out along the axis with the smallest exit
    /// distance, on the nearer side.  All arithmetic here is exact in f32
    /// so the expected positions compare with `==`.
    #[test]
    fn rect_deep_penetration_pushes_out_along_nearest_axis() {
        let mut o = MapObstacles::default();
        let center = Vec2::new(10.0, -20.0);
        let half = Vec2::new(100.0, 40.0);
        o.list.push(Obstacle { pos: center, shape: ObstacleShape::Rect(half) });
        o.rebuild_grid();
        let own = 6.0;

        // Exact centre: Y is the shortest extent (40 < 100); the bottom/top
        // tie breaks to the top branch (`dy_bot < dy_top` is false).
        let mut p = center;
        o.resolve(&mut p, own);
        assert_eq!(p, Vec2::new(center.x, center.y + half.y + own));
        assert!(!o.hits(p, own - 0.01));

        // Near the left face: exit left (dx_left = 10 beats dy = 40).
        let mut p = center + Vec2::new(-90.0, 0.0);
        o.resolve(&mut p, own);
        assert_eq!(p, Vec2::new(center.x - half.x - own, center.y));
        assert!(!o.hits(p, own - 0.01));

        // Near the bottom face: exit down (dy_bot = 10 beats dx = 100).
        let mut p = center + Vec2::new(0.0, -30.0);
        o.resolve(&mut p, own);
        assert_eq!(p, Vec2::new(center.x, center.y - half.y - own));
        assert!(!o.hits(p, own - 0.01));
    }

    /// Shallow Rect overlap (centre outside the rect, closer than
    /// own_radius to its surface): pushed straight off the closest face.
    #[test]
    fn rect_shallow_overlap_pushes_off_nearest_face() {
        let mut o = MapObstacles::default();
        o.list.push(Obstacle {
            pos: Vec2::new(10.0, -20.0),
            shape: ObstacleShape::Rect(Vec2::new(100.0, 40.0)),
        });
        o.rebuild_grid();
        // 2 px above the top face, radius 6 → pushed to 6 px above it.
        let mut p = Vec2::new(10.0, 22.0);
        o.resolve(&mut p, 6.0);
        assert_eq!(p, Vec2::new(10.0, 26.0));
        assert!(!o.hits(p, 5.99));
    }
}

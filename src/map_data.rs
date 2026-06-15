// ════════════════════════════════════════════════════════════════════════
//  World blueprint — 5 themed segments in a horizontal strip:
//      [Suburbs] → [Downtown] → [Industrial] → [Hospital] → [Military]
//  Total: 240×48 tiles (7680×1536 px).  Each segment is 48×48 tiles with
//  its own road grid (rows 22-25 horizontal, cols 22-25 vertical).
//
//  Buildings + props are baked from the deterministic generator in the
//  prototype HTML (`assets/zombiegame2-map.html`) via:
//      node /tmp/.../gen.js | node /tmp/.../emit_rust.js > world_consts.rs
//  See `src/world_consts.rs` for the actual data tables.  Consumed by
//  `map.rs::spawn_map`.
// ════════════════════════════════════════════════════════════════════════

use bevy::math::IVec2;

// Re-export the baked tables so `map.rs` can use them directly.
// (`world_consts::ZOMBIE_SPAWNS` exists for a future "pre-placed ambient
// zombies" feature but isn't re-exported here yet.)
pub use crate::world_consts::{BUILDINGS, PROPS, SEGMENTS};

// ──── Segments ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Suburb,
    Downtown,
    Industrial,
    Hospital,
    Military,
}

#[derive(Clone, Copy, Debug)]
/// `name` and `difficulty` are baked into world_consts but unused at
/// runtime — kept to keep the world generator output stable.
#[allow(dead_code)]
pub struct Segment {
    pub id: u8,
    pub name: &'static str,
    pub pl_name: &'static str,
    pub difficulty: u8,
    pub theme: Theme,
}

// ──── Buildings ─────────────────────────────────────────────────────────

/// All building archetypes the world generator can emit.  The current
/// `world_consts.rs` doesn't instantiate `Civic` or `Market` but they're
/// kept so re-baking with a different seed/spec stays compatible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum BuildingType {
    House,
    Shed,
    Garage,
    Shop,
    Apartment,
    Civic,
    Church,
    Market,
    Bank,
    Tower,
    Factory,
    Warehouse,
    Depot,
    Tank,
    Hospital,
    Morgue,
    Park,
    Bunker,
    Tent,
    Helipad,
    Gas,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoofStyle {
    /// Pitched two-slope roof — houses, sheds, churches, market hall, park
    /// pavilion, gable tents.
    Gable,
    /// Flat parapet roof with AC unit highlight — most civic / commercial.
    Flat,
    /// Apartment-block grid of dark balcony rectangles + central elevator.
    Apt,
    /// Saw-tooth strips with skylights — factories.
    Saw,
    /// Cylindrical tank, ellipse fill.
    Round,
    /// Pyramidal canvas pitch — military tents.
    Tent,
    /// Helipad — flat concrete + circle + yellow "H".
    Pad,
}

#[derive(Clone, Copy, Debug)]
pub struct Building {
    pub seg_id: u8,
    pub kind: BuildingType,
    pub roof: RoofStyle,
    /// Local tile coords inside the segment (0..48).
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Door tile (local coords) — for visual marker only; buildings are
    /// solid blockers in this iteration (no enterable interiors yet).
    pub door: IVec2,
}

// ──── Props ─────────────────────────────────────────────────────────────

/// All prop archetypes the world generator can emit.  `Car` and `Wreck`
/// aren't used by the current bake but kept so a re-generation can use
/// them without protocol changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum PropKind {
    // Suburb / hospital flora
    Tree,
    Bush,
    HedgeH,
    HedgeV,
    Planter,
    // Vehicles
    Car,
    Wreck,
    Bus,
    Truck,
    Ambulance,
    MilTruck,
    Jeep,
    // Urban scatter
    Mailbox,
    Trash,
    Lamp,
    Dumpster,
    Bench,
    Sign,
    Blood,
    Debris,
    // Industrial
    Container,
    Barrels,
    Pallet,
    Oil,
    Crane,
    Forklift,
    Crate,
    // Hospital
    Gurney,
    Playground,
    BodyBag,
    // Military
    SandbagH,
    SandbagV,
    RazorH,
    RazorV,
    Crater,
    Flag,
}

#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub seg_id: u8,
    pub kind: PropKind,
    /// Local tile coords inside the segment.
    pub x: i32,
    pub y: i32,
    /// Footprint in tiles (most are 1×1; hedges, vehicles, sandbags larger).
    pub w: i32,
    pub h: i32,
}

// ──── Gates between segments ────────────────────────────────────────────

/// `Tunnel` isn't currently emitted by the world generator but exists
/// so a future `GATES` re-bake can use it without changing this enum.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum GateKind {
    /// Old river bridge: arched span over water.
    Bridge,
    /// Wall breach: rubble-walled passage.
    Breach,
    /// Underground service overpass / tunnel.
    Tunnel,
    /// Military checkpoint with barriers.
    Gate,
}

#[derive(Clone, Copy, Debug)]
pub struct Gate {
    pub from_seg: u8,
    pub to_seg: u8,
    pub kind: GateKind,
    /// Cost in dollars to unlock (= deduct from shared `Score`).
    pub cost: u32,
    /// Human-readable name baked for future tooltip / UI use.
    #[allow(dead_code)]
    pub label: &'static str,
}

/// Four gates connecting the 5 segments.  Player spends $$$ at each gate
/// to push deeper into the world — same model as before, applied to the
/// new layout.
pub const GATES: &[Gate] = &[
    Gate {
        from_seg: 1,
        to_seg: 2,
        kind: GateKind::Bridge,
        cost: 400,
        label: "OLD RIVER BRIDGE",
    },
    Gate {
        from_seg: 2,
        to_seg: 3,
        kind: GateKind::Breach,
        cost: 800,
        label: "WALL BREACH",
    },
    Gate {
        from_seg: 3,
        to_seg: 4,
        kind: GateKind::Bridge,
        cost: 1500,
        label: "SERVICE OVERPASS",
    },
    Gate {
        from_seg: 4,
        to_seg: 5,
        kind: GateKind::Gate,
        cost: 3000,
        label: "CHECKPOINT ALPHA",
    },
];

// (legacy `BuildingKind`/`FurnitureKind`/`StreetDecorKind` shims removed —
// the village/saloon/diner content was never instantiated in the metro map
// and nothing outside this file referenced the types.)

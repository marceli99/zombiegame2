# zombiegame2 — World Map Specification

This document is the **handoff spec** for the world map of `zombiegame2`. It is intended for Claude Code (or any developer) to **re-implement** the map system in the Rust game — *not* to blindly copy the prototype. The HTML file (`zombiegame2-map.html`) is a visual reference + interactive design tool; this `.md` is the contract.

---

## 1. High-level concept

The game world is a **horizontal strip** of **5 segments**, each segment a square 2D top-down map of **48 × 48 tiles** (tile = 32 × 32 px). Total world size: **240 × 48 tiles** (7680 × 1536 px). The player progresses **west → east**, unlocking each segment by satisfying its `unlock_condition`. Adjacent segments are connected by a **gate** object that gates traversal until the unlock flag is set.

```
[ Suburbs ] -bridge- [ Downtown ] -breach- [ Industrial ] -bridge- [ Hospital ] -gate- [ Military ]
   seg 1                seg 2                  seg 3                 seg 4              seg 5
```

### Per-segment summary

| ID  | Name (EN)            | Name (PL)             | Difficulty | Theme       | Unlock                                       |
|-----|----------------------|-----------------------|------------|-------------|----------------------------------------------|
| 1   | Suburbs              | Przedmieścia          | ★☆☆☆☆     | suburb      | (start)                                      |
| 2   | Downtown             | Centrum miasta        | ★★☆☆☆     | downtown    | Clear segment 1 (kill 80% zombies)           |
| 3   | Industrial           | Dzielnica przemysłowa | ★★★☆☆     | industrial  | Find factory keycard in segment 2            |
| 4   | Hospital & Park      | Szpital i Park        | ★★★★☆     | hospital    | Repair generator in segment 3                |
| 5   | Military Checkpoint  | Wojskowy checkpoint   | ★★★★★     | military    | Obtain military access codes in segment 4    |

### Visual style

- 2D **top-down retro pixel-art**, GTA1 / Hotline Miami feeling.
- Ground is **always green grass** with per-segment hue variation + dithered tufts (no gray/brown bases).
- Each segment has a **road grid**: one horizontal road and one vertical road meeting in a central intersection, with sidewalks framing the asphalt.
- Buildings sit on plots **set back from the road** with front yards / parking. They have **pitched gable roofs (with chunky tile rows + chimneys)**, **flat roofs with parapet + AC units**, **saw-tooth factory roofs**, **apartment blocks with balcony grids**, etc.

---

## 2. Coordinate system & tile grid

- World: **240 × 48 tiles**, tile size **32 × 32 px**.
- Each segment occupies columns `(seg_id − 1) * 48 .. seg_id * 48` (exclusive), rows `0..48`.
- **Roads** within each segment:
  - Horizontal road: rows **22..25** (4 tiles wide).
  - Vertical road: cols **22..25** (4 tiles wide).
  - **Sidewalks**: 1-tile rim immediately outside each road (rows 21 and 26; cols 21 and 26).
- The 4 building **quadrants** (relative to each segment's local origin):
  - NW: `(0,0)` size `21×21`
  - NE: `(26,0)` size `22×21`
  - SW: `(0,26)` size `21×22`
  - SE: `(26,26)` size `22×22`

In Rust, segment-local coords convert to world coords with:

```rust
fn world_xy(seg_id: u8, local_x: i32, local_y: i32) -> (i32, i32) {
    ((seg_id as i32 - 1) * 48 + local_x, local_y)
}
```

---

## 3. Tileset

The map's tilelayer references 9 base tile IDs:

| ID  | Name           | Walkable | Notes                                       |
|-----|----------------|----------|---------------------------------------------|
| 1   | grass          | yes      | base ground (per-segment hue)               |
| 2   | road           | yes      | asphalt, horizontal/vertical roads          |
| 3   | sidewalk       | yes      | 1-tile rim around each road                 |
| 4   | building_wall  | no       | exterior wall of building footprint         |
| 5   | building_floor | yes      | interior floor (only when inside building)  |
| 6   | water          | no       | reserved (not currently used by generator)  |
| 7   | debris         | yes      | decorative ground variant (used in industrial base in legacy seeds; current generator keeps grass everywhere) |
| 8   | gate           | conditional | placed under gate objects                |
| 9   | door           | conditional | walkable only if building unlocked       |

> **Implementation note for Rust:** the prototype's tilelayer is a coarse footprint — a real Rust implementation will want a richer tileset for visual variety (multiple grass variants, asphalt orientation tiles, sidewalk corner tiles, building corner/edge tiles per archetype). Treat the prototype's 9 IDs as **collision/semantic categories**, not as final art.

---

## 4. Building system

### 4.1 Archetypes (per theme)

Each archetype defines: `type` (semantic id), `name` (display), `w`/`h` size range in tiles, weight (relative spawn frequency), `roof` style.

```
suburb:    house, bungalow, garden_shed, garage, corner_mart
downtown:  city_hall, church, apartment_block, tenement, shop, pharmacy, bakery,
           market_hall, bank, office_block, gas_station(*)
industrial:factory_hall, warehouse, truck_depot, generator_shed, storage_tank
hospital:  hospital_wing, hospital_annex, morgue, ambulance_bay, park_pavilion, toilet_block
military:  bunker, command_tent, barracks_tent, supply_tent, watchtower, helipad, ammo_crate
```

(*) **Gas Station** is special: hard-pinned to the **NE corner of the intersection in segment 2 (Downtown)**. Its forecourt (canopy + 2 pumps) sits **south of the store**, between the store and the horizontal road, in the strip of grass/sidewalk that would otherwise be empty.

### 4.2 Roof styles

| Style    | Used by                                  | Visual                                                  |
|----------|------------------------------------------|---------------------------------------------------------|
| `gable`  | houses, sheds, churches, market hall, park pavilion, tents (variant) | two trapezoid slopes, ridge line, chunky pixel tile rows, brick chimney |
| `flat`   | shops, civic, bank, depot, garage, hospital, gas, etc. | parapet rim + tar-grid pattern, AC unit, pixel-art highlight NW / shadow SE |
| `apt`    | apartment blocks, tenements              | flat roof with grid of dark balcony rectangles + central elevator/stair box |
| `saw`    | factories                                | alternating dark/light strips with skylights + chimney stacks |
| `round`  | storage tanks                            | ellipse fill                                            |
| `tent`   | military tents                           | pyramid (two triangles, dark + light)                   |
| `pad`    | helipad                                  | flat concrete + circle + yellow "H"                     |

### 4.3 Type-specific roof extras

- `church` → bell tower with cross at front
- `hospital` → red cross painted on roof
- `gas` → red roof stripe + yellow "SHOP" sign + canopy + 2 pumps to the south
- `civic` → 5 columns at the front facade
- `bank` → "$" symbol
- `tower` → smaller dark square inside (vent/elevator)
- `tank` → fuel symbol

### 4.4 Plot layout

Each quadrant is sub-divided into **1–2 columns × 1–2 rows of cells** depending on size (≥14 tiles → 2 cells along that axis). Each cell either gets **one building** (centered with margin = front yard) or stays empty (≈18% chance). The building's **door** is placed on the side of the building **facing the nearest road**.

---

## 5. Props (decorative + interactable)

Props live on top of the tilelayer and are placed by a deterministic occupancy-grid algorithm. They are rendered as small SVG glyphs in the prototype, but in Rust they should be **entities** with their own sprite.

Per-theme prop catalogue (kind → typical count per segment):

```
suburb:     tree×36 bush×28 hedge_h×8 hedge_v×8 car×7(yard+road) wreck×2 mailbox×8
            trash×6 blood×10 lamp×10
downtown:   tree×14 car×8(road) wreck×5(road) bus×1 dumpster×12 lamp×18
            bench×8 trash×14 debris×16 blood×16 sign×6 planter×6
industrial: truck×7 container×13 barrels×22 pallet×18 debris×26 blood×12
            oil×8 crane×1 forklift×3 crate×14
hospital:   tree×38 bush×22 hedge_h×6 hedge_v×5 ambulance×5 car×4(road)
            gurney×8 bench×7 playground×1 blood×18 body_bag×6 lamp×12 trash×5
military:   sandbag_h×12 sandbag_v×8 mil_truck×6 jeep×5 crate×24 barrels×14
            razor_h×5 razor_v×4 blood×18 debris×18 crater×5 flag×2 tree×8
```

`where = "yard"` props go on free grass (not on roads, not overlapping buildings).
`where = "road"` props (cars, wrecks, buses, trucks) go on asphalt tiles.

Lamps, signs, and flags render **above** buildings; everything else renders **under** buildings.

---

## 6. Spawns

Each segment has:

- **Player spawn**: only segment 1 has one, at local `(4, 32)`.
- **Zombie spawns**: count = `3 + difficulty * 2` (so segment 1 has 5, segment 5 has 13). Random free-tile placement.
- **Loot spawns**: `8–12` per segment, plus **key items** for progression:
  - seg 2 → `Factory Keycard` (rare)
  - seg 3 → `Generator Parts` (epic)
  - seg 4 → `Military Access Codes` (rare)
  - seg 5 → `Helicopter Keys` (epic)

Loot rarity weights (shift toward rare/epic with difficulty):

```
common   = 0.50 − difficulty*0.05
uncommon = 0.30
rare     = 0.15 + difficulty*0.03
epic     = 0.05 + difficulty*0.02
```

### Loot tables per theme

```
suburb:     Bandage, Canned Beans, Pistol Ammo, Bottle Water, Crowbar, Hammer, Jerry Can
downtown:   Painkillers, Bread, Apples, Lockpick Set, Holy Water, Newspaper,
            Shotgun Shells, Factory Keycard
industrial: Wrench Set, Power Tool, Scrap Metal, Welding Mask, Truck Battery,
            Generator Parts, Oil Can, Toolbox
hospital:   Surgical Kit, Adrenaline Shot, Antibiotics, Morphine, Defibrillator,
            Stretcher, Snack Bar, Military Access Codes
military:   Assault Rifle, Body Armor, Field Radio, MRE Ration, Frag Grenade,
            Helicopter Keys, Sniper Scope, Night Vision, Tactical Vest
```

---

## 7. POIs (Points of Interest)

Each segment has 2–3 POIs with a name, emoji icon, world position, and short description (used for in-game hints / map markers).

```
seg 1: Family Home, Tool Shed
seg 2: City Hall, Gas Station, Market Square
seg 3: Factory Floor, Generator
seg 4: Hospital ICU, Park Memorial
seg 5: Helicopter (endgame), Command Bunker
```

---

## 8. Gates

Gates connect adjacent segments. Each gate has:

- `id`: unique string (e.g. `g1_2_a`)
- `type`: `bridge` | `breach` | `tunnel` | `gate`
- `connects_segments`: `[from, to]`
- `between`: the seg_id whose east edge the gate sits on (so gate is at `world_x = between * 48`)
- `label`: human display name

```
g1_2_a  bridge  [1,2]  Old River Bridge       (between segments 1 & 2)
g2_3_a  breach  [2,3]  Wall Breach
g3_4_a  bridge  [3,4]  Service Overpass
g4_5_a  gate    [4,5]  Checkpoint Alpha
```

A gate is **traversable** iff the destination segment's unlock flag is set.

---

## 9. File format — Tiled JSON

The HTML prototype exposes a **single Tiled-compatible JSON** via the "📋 Copy Tiled JSON" button. Schema highlights:

```jsonc
{
  "type": "map", "version": "1.10", "tiledversion": "1.10.2",
  "orientation": "orthogonal", "renderorder": "right-down",
  "width": 240, "height": 48, "tilewidth": 32, "tileheight": 32,
  "infinite": false,
  "properties": [
    { "name": "segments_meta", "type": "string",
      "value": "[{...id, name, pl_name, difficulty, status, unlock_condition, theme_color, theme...}]" }
  ],
  "tilesets": [{ "firstgid": 1, "name": "zombiegame2_tiles", "tilecount": 9, ... }],
  "layers": [
    // 5 tilelayers, one per segment, offset on x by seg_id * 48
    { "id": 1, "name": "segment_1_suburb", "type": "tilelayer",
      "width": 48, "height": 48, "x": 0, "y": 0, "data": [/* 2304 ints */] },
    // ... segments 2..5
    // 1 objectgroup with all spawns/buildings/POIs/gates
    { "id": 99, "name": "objects", "type": "objectgroup", "objects": [
      { "id": 1, "name": "player_spawn_seg1", "type": "player_spawn", "x": 128, "y": 1024,
        "width": 32, "height": 32, "properties": [{"name":"segment_id","type":"int","value":1}] },
      { "id": 2, "name": "zombie_seg1_0", "type": "zombie_spawn", ... },
      { "id": ?, "name": "loot_factory_keycard", "type": "loot_spawn",
        "properties": [{"name":"rarity","type":"string","value":"rare"},
                       {"name":"item","type":"string","value":"Factory Keycard"}, ...] },
      { "id": ?, "name": "Hospital ICU", "type": "poi",
        "properties": [{"name":"description","type":"string","value":"..."},
                       {"name":"icon","type":"string","value":"🏥"}, ...] },
      { "id": ?, "name": "House", "type": "building",
        "properties": [{"name":"building_type","type":"string","value":"house"},
                       {"name":"building_id","type":"string","value":"b_..."}, ...] },
      { "id": ?, "name": "Old River Bridge", "type": "gate",
        "properties": [{"name":"gate_id","type":"string","value":"g1_2_a"},
                       {"name":"gate_type","type":"string","value":"bridge"},
                       {"name":"connects_segments","type":"string","value":"[1,2]"}] }
    ]}
  ]
}
```

Object positions are in **pixels**, not tiles (Tiled convention). Convert with `tile = px / 32`.

---

## 10. Rust integration plan

### 10.1 Recommended crates

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tiled = "0.13"          # parses Tiled JSON
glam = "0.27"           # Vec2, IVec2 for world coords
```

### 10.2 Domain model (suggested)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SegmentId(pub u8);

#[derive(Debug, Clone)]
pub struct Segment {
    pub id: SegmentId,
    pub name: String,
    pub pl_name: String,
    pub difficulty: u8,
    pub theme: Theme,
    pub unlock_condition: UnlockCondition,
    pub status: SegmentStatus,
    pub buildings: Vec<Building>,
    pub spawns: Spawns,
    pub pois: Vec<Poi>,
    pub tile_grid: Vec<TileId>, // 48*48
}

#[derive(Debug, Clone, Copy)]
pub enum Theme { Suburb, Downtown, Industrial, Hospital, Military }

#[derive(Debug, Clone)]
pub struct Building {
    pub id: String,
    pub name: String,
    pub building_type: BuildingType,
    pub roof: RoofStyle,
    pub local_rect: IRect,        // tile coords inside its segment
    pub doors: Vec<IVec2>,        // tile coords inside its segment
}

#[derive(Debug, Clone)]
pub struct Spawns {
    pub player: Option<IVec2>,
    pub zombies: Vec<IVec2>,
    pub loot: Vec<LootDrop>,
}

#[derive(Debug, Clone)]
pub struct LootDrop { pub pos: IVec2, pub rarity: Rarity, pub item: String }

#[derive(Debug, Clone, Copy)]
pub enum Rarity { Common, Uncommon, Rare, Epic }

#[derive(Debug, Clone)]
pub enum UnlockCondition {
    Start,
    ClearSegment { id: SegmentId, percent: u8 },
    HasItem(String),         // e.g. "Factory Keycard"
    InteractWith(String),    // e.g. "Generator"
    HasItem2(String, String) // codes etc.
}

#[derive(Debug, Clone)]
pub struct Gate {
    pub id: String,
    pub kind: GateKind,
    pub from: SegmentId,
    pub to:   SegmentId,
    pub world_rect: IRect,
}
```

### 10.3 Loading

```rust
let map: serde_json::Value = serde_json::from_str(&fs::read_to_string("world.tmj")?)?;

// Extract per-segment meta
let segments_meta: Vec<SegmentMeta> = serde_json::from_str(
    map["properties"]
       .as_array().unwrap().iter()
       .find(|p| p["name"] == "segments_meta").unwrap()["value"].as_str().unwrap()
)?;

// Extract tilelayers (one per segment)
for layer in map["layers"].as_array().unwrap() {
    if layer["type"] == "tilelayer" {
        let seg_id = parse_seg_id(&layer["name"]);
        let tiles: Vec<u32> = serde_json::from_value(layer["data"].clone())?;
        // ...
    }
}

// Extract objectgroup
let objects = &map["layers"].as_array().unwrap()
    .iter().find(|l| l["type"] == "objectgroup").unwrap()["objects"];
for obj in objects.as_array().unwrap() {
    match obj["type"].as_str() {
        Some("player_spawn") => /* ... */,
        Some("zombie_spawn") => /* ... */,
        Some("loot_spawn") => /* read rarity + item from properties */,
        Some("building") => /* read building_type, building_id */,
        Some("poi") => /* read description, icon */,
        Some("gate") => /* read gate_id, gate_type, connects_segments */,
        _ => {}
    }
}
```

### 10.4 Streaming / activation

The world is small enough (240×48 tiles ≈ 11k tiles + a few hundred entities) to **fit in memory**. But for performance:

1. Spawn zombie/loot/POI entities **only for the active segment + adjacent unlocked segments**.
2. Despawn or freeze entities in segments outside of `(active − 1) ..= (active + 1)`.
3. Use the `segment_id` property on every object to bucket them at load time into `HashMap<SegmentId, Vec<Entity>>`.

### 10.5 Collision grid

Build a `[bool; 240*48]` from the tilelayers using these rules:

| Tile id          | Walkable                                       |
|------------------|------------------------------------------------|
| 1 grass          | yes                                            |
| 2 road           | yes                                            |
| 3 sidewalk       | yes                                            |
| 4 building_wall  | **no**                                         |
| 5 building_floor | yes (only reachable via door)                  |
| 6 water          | no                                             |
| 7 debris         | yes (slow tile, optional speed multiplier)     |
| 8 gate           | conditional on `gate.is_open()`                |
| 9 door           | conditional on `building.is_unlocked()`        |

Buildings should also expose **AABB rects** so AI can do line-of-sight checks against whole structures rather than per-tile.

### 10.6 Progression flags

Use a single `WorldFlags` resource:

```rust
pub struct WorldFlags {
    pub seg_cleared: [u8; 5],         // % of zombies killed per segment
    pub has_factory_keycard: bool,
    pub generator_repaired: bool,
    pub has_military_codes: bool,
    pub has_helicopter_keys: bool,
}

impl WorldFlags {
    pub fn segment_unlocked(&self, id: SegmentId) -> bool {
        match id.0 {
            1 => true,
            2 => self.seg_cleared[0] >= 80,
            3 => self.has_factory_keycard,
            4 => self.generator_repaired,
            5 => self.has_military_codes,
            _ => false,
        }
    }
    pub fn gate_open(&self, gate: &Gate) -> bool {
        self.segment_unlocked(gate.to)
    }
}
```

### 10.7 What to recreate vs copy

**Copy as-is from the prototype's Tiled JSON:**
- The 5×48×48 tile grids (collision footprint).
- Zombie / loot / POI spawn coordinates (good enough for v1).
- Gate positions and connections.
- Building footprints (id, name, type, rect, door positions).
- The `unlock_condition` strings as the source of truth for the progression graph.

**Recreate / improve in Rust:**
- The **art assets** — the prototype uses inline SVG primitives; Rust should use real pixel-art sprites at 32×32 with proper animation frames (zombies shamble, lamps flicker).
- The **prop placement algorithm** — rerun procedural generation in Rust at world-build time using the same per-theme catalogues + counts in §5. Use a deterministic seed (`segment_id * 9173`) to keep the layout stable across runs.
- The **building roof tile rows** — port to per-archetype tilemap variants (e.g. 5 house-roof tiles, 4 factory-saw-roof tiles) instead of generated geometry.
- A **richer ground tilemap** — multiple grass variants, sidewalk corner tiles, road junction tiles for visual variety.
- **Day/night, weather, fog of war** — not in the prototype, design fresh in Rust.

---

## 11. Asset checklist for Claude Code

For each of the following, the Rust port should produce / import a sprite (32×32 unless noted):

```
TILES    grass_a, grass_b, grass_c, sidewalk_h, sidewalk_v, sidewalk_corner_NE/NW/SE/SW,
         road_h, road_v, road_intersection, road_dash_h, road_dash_v, crosswalk
ROOFS    roof_gable_horiz_top, roof_gable_horiz_bot (per house palette ×2)
         roof_flat_corner_NW/NE/SW/SE, roof_flat_edge_N/E/S/W, roof_flat_center_ac,
         roof_apt_balcony, roof_apt_center_lift, roof_saw_strip, roof_tent
BUILDINGS  house_wall_brick, shop_wall_stucco, factory_wall_metal, hospital_wall_white,
           bunker_wall_concrete, tent_canvas
DOORS    door_wood, door_metal, door_glass
PROPS    tree, bush, hedge_seg, car_red, car_blue, car_grey, wreck, bus,
         truck, ambulance, mil_truck, jeep, container_red, container_blue,
         barrel, pallet, dumpster, trash_can, debris_chunk, blood_splat (4 variants),
         lamp_post, bench, mailbox, sign_yellow, planter, oil_slick, crane, forklift,
         gurney, body_bag, playground, sandbag_seg, razor_wire, crater, flag
ENTITIES player, zombie_walker, zombie_runner (animated)
POI      poi_marker_glow (with icon overlay)
```

---

## 12. Open questions for the design owner

1. Should buildings have **enterable interiors** (separate indoor scenes) or stay as solid blockers with on-screen loot popups when adjacent? (The prototype currently treats them as solid silhouettes; the door tile is a visual marker.)
2. **Day/night cycle?** If yes, lamps should emit light volumes — useful semantic data to include in the export.
3. Do zombies **respawn** within a cleared segment, or is it permanently safe once cleared?
4. Should the **gas station forecourt** at the segment-2 intersection be a *playable encounter* (e.g. survivor cache + zombie ambush) or pure scenery?

---

*Generated from `zombiegame2-map.html` v3 (May 2026).*

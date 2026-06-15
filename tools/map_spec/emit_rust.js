// Emits the generated world data as Rust const arrays for `map_data.rs`.
// Runs after gen.js and reads world.json.

const fs = require('fs');
const data = JSON.parse(fs.readFileSync('/tmp/zombiegame2_spec/world.json', 'utf8'));

const themeEnum = {
  suburb: "Suburb",
  downtown: "Downtown",
  industrial: "Industrial",
  hospital: "Hospital",
  military: "Military",
};

const buildingTypeEnum = {
  house: "House",
  shed: "Shed",
  garage: "Garage",
  shop: "Shop",
  apartment: "Apartment",
  civic: "Civic",
  church: "Church",
  market: "Market",
  bank: "Bank",
  tower: "Tower",
  factory: "Factory",
  warehouse: "Warehouse",
  depot: "Depot",
  tank: "Tank",
  hospital: "Hospital",
  morgue: "Morgue",
  park: "Park",
  bunker: "Bunker",
  tent: "Tent",
  helipad: "Helipad",
  gas: "Gas",
};

const roofEnum = {
  gable: "Gable",
  flat: "Flat",
  apt: "Apt",
  saw: "Saw",
  round: "Round",
  tent: "Tent",
  pad: "Pad",
};

const propEnum = {
  tree: "Tree",
  bush: "Bush",
  hedge_h: "HedgeH",
  hedge_v: "HedgeV",
  car: "Car",
  wreck: "Wreck",
  bus: "Bus",
  mailbox: "Mailbox",
  trash: "Trash",
  blood: "Blood",
  lamp: "Lamp",
  dumpster: "Dumpster",
  bench: "Bench",
  debris: "Debris",
  sign: "Sign",
  planter: "Planter",
  truck: "Truck",
  container: "Container",
  barrels: "Barrels",
  pallet: "Pallet",
  oil: "Oil",
  crane: "Crane",
  forklift: "Forklift",
  crate: "Crate",
  ambulance: "Ambulance",
  gurney: "Gurney",
  playground: "Playground",
  body_bag: "BodyBag",
  sandbag_h: "SandbagH",
  sandbag_v: "SandbagV",
  mil_truck: "MilTruck",
  jeep: "Jeep",
  razor_h: "RazorH",
  razor_v: "RazorV",
  crater: "Crater",
  flag: "Flag",
};

let out = "";
out += "// AUTO-GENERATED from /tmp/zombiegame2_spec/gen.js — do not edit by hand.\n";
out += "// Re-run `node gen.js | emit_rust.js` to regenerate after spec changes.\n";
out += "\n";
out += "use crate::map_data::{Building, BuildingType, Prop, PropKind, RoofStyle, Segment, Theme};\n";
out += "use bevy::math::IVec2;\n";
out += "\n";

out += `pub const SEGMENTS: [Segment; 5] = [\n`;
for (const seg of data.segments) {
  out += `    Segment {\n`;
  out += `        id: ${seg.id},\n`;
  out += `        name: ${JSON.stringify(seg.name)},\n`;
  out += `        pl_name: ${JSON.stringify(seg.pl_name)},\n`;
  out += `        difficulty: ${seg.difficulty},\n`;
  out += `        theme: Theme::${themeEnum[seg.theme]},\n`;
  out += `    },\n`;
}
out += `];\n\n`;

let allBuildings = [];
let allProps = [];
let allZombies = [];
for (const seg of data.segments) {
  for (const b of seg.buildings) allBuildings.push({ ...b, seg_id: seg.id });
  for (const p of seg.props) allProps.push({ ...p, seg_id: seg.id });
  for (const z of seg.zombies) allZombies.push({ x: z.x, y: z.y, seg_id: seg.id });
}

out += `pub const BUILDINGS: &[Building] = &[\n`;
for (const b of allBuildings) {
  out += `    Building { seg_id: ${b.seg_id}, kind: BuildingType::${buildingTypeEnum[b.type]}, roof: RoofStyle::${roofEnum[b.roof]}, x: ${b.x}, y: ${b.y}, w: ${b.w}, h: ${b.h}, door: IVec2::new(${b.door_x}, ${b.door_y}) },\n`;
}
out += `];\n\n`;

out += `pub const PROPS: &[Prop] = &[\n`;
for (const p of allProps) {
  out += `    Prop { seg_id: ${p.seg_id}, kind: PropKind::${propEnum[p.kind]}, x: ${p.x}, y: ${p.y}, w: ${p.w}, h: ${p.h} },\n`;
}
out += `];\n\n`;

out += `pub const ZOMBIE_SPAWNS: &[(u8, IVec2)] = &[\n`;
for (const z of allZombies) {
  out += `    (${z.seg_id}, IVec2::new(${z.x}, ${z.y})),\n`;
}
out += `];\n`;

console.log(out);

// Extracted from zombiegame2-map.html — runs the deterministic generator
// and dumps a Rust-friendly JSON (positions in tile coords, not pixels).

const SEGMENTS_META = [
  { id: 1, name: "Suburbs",             pl_name: "Przedmieścia",          difficulty: 1, theme: "suburb"   },
  { id: 2, name: "Downtown",            pl_name: "Centrum miasta",        difficulty: 2, theme: "downtown" },
  { id: 3, name: "Industrial",          pl_name: "Dzielnica przemysłowa", difficulty: 3, theme: "industrial"},
  { id: 4, name: "Hospital & Park",     pl_name: "Szpital i Park",        difficulty: 4, theme: "hospital" },
  { id: 5, name: "Military Checkpoint", pl_name: "Wojskowy checkpoint",   difficulty: 5, theme: "military" },
];

const mulberry32 = (a) => () => {
  a = (a + 0x6d2b79f5) | 0;
  let t = a;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const ROAD_H_TOP = 22, ROAD_H_BOT = 25;
const ROAD_V_LEFT = 22, ROAD_V_RIGHT = 25;

const QUADRANTS = [
  { x: 0,  y: 0,  w: 21, h: 21 },
  { x: 26, y: 0,  w: 22, h: 21 },
  { x: 0,  y: 26, w: 21, h: 22 },
  { x: 26, y: 26, w: 22, h: 22 },
];

const ARCHETYPES = {
  suburb: [
    { type: "house",    name: "House",             w: [7, 9],   h: [6, 8],   weight: 6, roof: "gable" },
    { type: "house",    name: "Bungalow",          w: [6, 8],   h: [5, 6],   weight: 4, roof: "gable" },
    { type: "shed",     name: "Garden Shed",       w: [3, 4],   h: [3, 4],   weight: 3, roof: "gable" },
    { type: "garage",   name: "Garage",            w: [4, 5],   h: [4, 5],   weight: 2, roof: "flat" },
    { type: "shop",     name: "Corner Mart",       w: [5, 7],   h: [6, 7],   weight: 1, roof: "flat" },
  ],
  downtown: [
    { type: "civic",    name: "City Hall",         w: [10, 14], h: [9, 11],  weight: 1, roof: "flat" },
    { type: "church",   name: "Church",            w: [8, 10],  h: [12, 14], weight: 1, roof: "gable" },
    { type: "apartment",name: "Apartment Block",   w: [9, 12],  h: [10, 13], weight: 4, roof: "apt" },
    { type: "apartment",name: "Tenement",          w: [8, 10],  h: [9, 11],  weight: 3, roof: "apt" },
    { type: "shop",     name: "Shop",              w: [5, 7],   h: [6, 7],   weight: 3, roof: "flat" },
    { type: "shop",     name: "Pharmacy",          w: [6, 8],   h: [6, 8],   weight: 2, roof: "flat" },
    { type: "shop",     name: "Bakery",            w: [5, 7],   h: [5, 7],   weight: 2, roof: "flat" },
    { type: "market",   name: "Market Hall",       w: [10, 12], h: [7, 9],   weight: 1, roof: "gable" },
    { type: "bank",     name: "Bank",              w: [6, 8],   h: [7, 9],   weight: 1, roof: "flat" },
    { type: "tower",    name: "Office Block",      w: [7, 9],   h: [7, 9],   weight: 2, roof: "flat" },
  ],
  industrial: [
    { type: "factory",  name: "Factory Hall",      w: [14, 18], h: [11, 14], weight: 2, roof: "saw" },
    { type: "warehouse",name: "Warehouse",         w: [10, 14], h: [8, 11],  weight: 3, roof: "flat" },
    { type: "depot",    name: "Truck Depot",       w: [12, 16], h: [9, 11],  weight: 1, roof: "flat" },
    { type: "shed",     name: "Generator Shed",    w: [4, 5],   h: [4, 5],   weight: 1, roof: "flat" },
    { type: "tank",     name: "Storage Tank",      w: [5, 6],   h: [5, 6],   weight: 2, roof: "round" },
  ],
  hospital: [
    { type: "hospital", name: "Hospital Wing",     w: [12, 18], h: [10, 14], weight: 2, roof: "flat" },
    { type: "hospital", name: "Hospital Annex",    w: [9, 12],  h: [7, 9],   weight: 2, roof: "flat" },
    { type: "morgue",   name: "Morgue",            w: [6, 7],   h: [5, 7],   weight: 1, roof: "flat" },
    { type: "garage",   name: "Ambulance Bay",     w: [8, 10],  h: [5, 7],   weight: 1, roof: "flat" },
    { type: "park",     name: "Park Pavilion",     w: [5, 7],   h: [4, 6],   weight: 2, roof: "gable" },
    { type: "shed",     name: "Toilet Block",      w: [3, 4],   h: [3, 4],   weight: 1, roof: "flat" },
  ],
  military: [
    { type: "bunker",   name: "Bunker",            w: [10, 14], h: [10, 14], weight: 1, roof: "flat" },
    { type: "tent",     name: "Command Tent",      w: [5, 7],   h: [5, 7],   weight: 1, roof: "tent" },
    { type: "tent",     name: "Barracks Tent",     w: [5, 7],   h: [5, 7],   weight: 2, roof: "tent" },
    { type: "tent",     name: "Supply Tent",       w: [5, 7],   h: [5, 7],   weight: 1, roof: "tent" },
    { type: "tower",    name: "Watchtower",        w: [4, 5],   h: [4, 5],   weight: 2, roof: "flat" },
    { type: "helipad",  name: "Helipad",           w: [9, 11],  h: [8, 10],  weight: 1, roof: "pad" },
    { type: "shed",     name: "Ammo Crate",        w: [3, 4],   h: [3, 4],   weight: 2, roof: "flat" },
  ],
};

function packQuadrant(quad, theme, rng) {
  const archetypes = ARCHETYPES[theme];
  const buildings = [];
  const colCount = quad.w >= 14 ? 2 : 1;
  const rowCount = quad.h >= 14 ? 2 : 1;
  const cellW = Math.floor(quad.w / colCount);
  const cellH = Math.floor(quad.h / rowCount);
  for (let cy = 0; cy < rowCount; cy++) {
    for (let cx = 0; cx < colCount; cx++) {
      if (rng() < 0.18) continue;
      const cellX = quad.x + cx * cellW;
      const cellY = quad.y + cy * cellH;
      const totalW = archetypes.reduce((a,b) => a + b.weight, 0);
      let r = rng() * totalW;
      let arch = archetypes[0];
      for (const a of archetypes) { r -= a.weight; if (r <= 0) { arch = a; break; } }
      const w = Math.min(cellW - 2, arch.w[0] + Math.floor(rng() * (arch.w[1] - arch.w[0] + 1)));
      const h = Math.min(cellH - 2, arch.h[0] + Math.floor(rng() * (arch.h[1] - arch.h[0] + 1)));
      if (w < 3 || h < 3) continue;
      const marginX = Math.max(1, Math.floor((cellW - w) / 2));
      const marginY = Math.max(1, Math.floor((cellH - h) / 2));
      const bx = cellX + marginX;
      const by = cellY + marginY;
      let doorX = bx + Math.floor(w/2);
      let doorY = by + h - 1;
      const distTop = by;
      const distBot = 47 - (by + h);
      const distLeft = bx;
      const distRight = 47 - (bx + w);
      const minD = Math.min(distTop, distBot, distLeft, distRight);
      if (minD === distTop)      { doorY = by;          doorX = bx + Math.floor(w/2); }
      else if (minD === distBot) { doorY = by + h - 1;  doorX = bx + Math.floor(w/2); }
      else if (minD === distLeft){ doorX = bx;          doorY = by + Math.floor(h/2); }
      else                       { doorX = bx + w - 1;  doorY = by + Math.floor(h/2); }
      buildings.push({
        name: arch.name,
        type: arch.type,
        roof: arch.roof,
        x: bx, y: by, w, h,
        door_x: doorX, door_y: doorY,
      });
    }
  }
  return buildings;
}

function generateSegment(meta) {
  const rng = mulberry32(meta.id * 9173);
  const buildings = [];
  for (const q of QUADRANTS) {
    const bs = packQuadrant(q, meta.theme, rng);
    for (const b of bs) buildings.push(b);
  }
  if (meta.theme === "downtown") {
    for (let i = buildings.length - 1; i >= 0; i--) {
      if (buildings[i].type === "gas") buildings.splice(i, 1);
    }
    const gx = ROAD_V_RIGHT + 2;
    const gw = 10, gh = 5;
    const gy = ROAD_H_TOP - 1 - 4 - gh;
    for (let i = buildings.length - 1; i >= 0; i--) {
      const b = buildings[i];
      const overlap = !(b.x + b.w + 1 < gx || b.x > gx + gw + 1 || b.y + b.h + 1 < gy || b.y > gy + gh + 4 + 1);
      if (overlap) buildings.splice(i, 1);
    }
    buildings.push({
      name: "Gas Station",
      type: "gas",
      roof: "flat",
      x: gx, y: gy, w: gw, h: gh,
      door_x: gx, door_y: gy + gh - 1,
    });
  }

  const occ = new Uint8Array(48 * 48);
  for (let x = 0; x < 48; x++) {
    for (let y = ROAD_H_TOP - 1; y <= ROAD_H_BOT + 1; y++) occ[y*48+x] = 1;
  }
  for (let y = 0; y < 48; y++) {
    for (let x = ROAD_V_LEFT - 1; x <= ROAD_V_RIGHT + 1; x++) occ[y*48+x] = 1;
  }
  for (const b of buildings) {
    for (let dy = -1; dy <= b.h; dy++) {
      for (let dx = -1; dx <= b.w; dx++) {
        const xx = b.x + dx, yy = b.y + dy;
        if (xx >= 0 && xx < 48 && yy >= 0 && yy < 48) occ[yy*48+xx] = 1;
      }
    }
  }

  const props = [];
  const tryPlace = (kind, w = 1, h = 1, count = 1, where = "yard") => {
    let placed = 0, tries = 0;
    while (placed < count && tries < 300) {
      tries++;
      let x, y;
      if (where === "road") {
        if (rng() < 0.5) {
          x = Math.floor(rng() * (48 - w));
          y = ROAD_H_TOP + Math.floor(rng() * 4);
        } else {
          x = ROAD_V_LEFT + Math.floor(rng() * 4);
          y = Math.floor(rng() * (48 - h));
        }
      } else {
        x = Math.floor(rng() * (48 - w));
        y = Math.floor(rng() * (48 - h));
      }
      let ok = true;
      for (let dy = 0; dy < h && ok; dy++) for (let dx = 0; dx < w && ok; dx++) {
        const v = occ[(y+dy)*48 + (x+dx)];
        if (where === "yard" && v !== 0) ok = false;
        if (where === "road") {
          const onRoad = ((y+dy) >= ROAD_H_TOP && (y+dy) <= ROAD_H_BOT) ||
                         ((x+dx) >= ROAD_V_LEFT && (x+dx) <= ROAD_V_RIGHT);
          if (v === 2) ok = false;
          if (!onRoad) ok = false;
        }
      }
      if (ok) {
        for (let dy = 0; dy < h; dy++) for (let dx = 0; dx < w; dx++) {
          if (x+dx < 48 && y+dy < 48) occ[(y+dy)*48 + (x+dx)] = 2;
        }
        props.push({ kind, x, y, w, h });
        placed++;
      }
    }
  };

  if (meta.theme === "suburb") {
    // No civilian cars/wrecks — they don't fit the gloomy vibe.
    // Bumped tree/bush counts for denser suburbia.
    tryPlace("tree", 1, 1, 56);
    tryPlace("bush", 1, 1, 42);
    tryPlace("hedge_h", 3, 1, 10);
    tryPlace("hedge_v", 1, 3, 10);
    tryPlace("mailbox", 1, 1, 8);
    tryPlace("trash", 1, 1, 6);
    tryPlace("blood", 1, 1, 10);
    tryPlace("lamp", 1, 1, 10);
  } else if (meta.theme === "downtown") {
    // Civilian cars/wrecks removed; bus stays as the iconic abandoned vehicle.
    tryPlace("tree", 1, 1, 22);
    tryPlace("bush", 1, 1, 12);
    tryPlace("planter", 1, 1, 10);
    tryPlace("bus", 3, 1, 1, "road");
    tryPlace("dumpster", 1, 1, 12);
    tryPlace("lamp", 1, 1, 18);
    tryPlace("bench", 2, 1, 8);
    tryPlace("trash", 1, 1, 14);
    tryPlace("debris", 1, 1, 16);
    tryPlace("blood", 1, 1, 16);
    tryPlace("sign", 1, 1, 6);
  } else if (meta.theme === "industrial") {
    // Industrial vehicles stay (truck/forklift/crane fit the theme).
    // Added a touch of vegetation creeping through the concrete.
    tryPlace("tree", 1, 1, 8);
    tryPlace("bush", 1, 1, 6);
    tryPlace("truck", 3, 2, 4, "road");
    tryPlace("truck", 3, 2, 3, "yard");
    tryPlace("container", 3, 2, 8);
    tryPlace("container", 2, 3, 5);
    tryPlace("barrels", 1, 1, 22);
    tryPlace("pallet", 1, 1, 18);
    tryPlace("debris", 1, 1, 26);
    tryPlace("blood", 1, 1, 12);
    tryPlace("oil", 1, 1, 8);
    tryPlace("crane", 2, 2, 1);
    tryPlace("forklift", 2, 1, 3);
    tryPlace("crate", 1, 1, 14);
  } else if (meta.theme === "hospital") {
    // Civilian "car" removed; ambulances stay (theme-appropriate).
    tryPlace("tree", 1, 1, 56);
    tryPlace("bush", 1, 1, 32);
    tryPlace("hedge_h", 3, 1, 8);
    tryPlace("hedge_v", 1, 3, 8);
    tryPlace("ambulance", 2, 1, 3, "road");
    tryPlace("ambulance", 2, 1, 2, "yard");
    tryPlace("gurney", 1, 2, 8);
    tryPlace("bench", 2, 1, 7);
    tryPlace("playground", 2, 2, 1);
    tryPlace("blood", 1, 1, 18);
    tryPlace("body_bag", 1, 2, 6);
    tryPlace("lamp", 1, 1, 12);
    tryPlace("trash", 1, 1, 5);
  } else { // military
    // Bumped tree/bush counts; military vehicles stay.
    tryPlace("tree", 1, 1, 16);
    tryPlace("bush", 1, 1, 6);
    tryPlace("sandbag_h", 3, 1, 12);
    tryPlace("sandbag_v", 1, 3, 8);
    tryPlace("mil_truck", 3, 2, 4, "road");
    tryPlace("mil_truck", 3, 2, 2, "yard");
    tryPlace("jeep", 2, 1, 5, "road");
    tryPlace("crate", 1, 1, 24);
    tryPlace("barrels", 1, 1, 14);
    tryPlace("razor_h", 4, 1, 5);
    tryPlace("razor_v", 1, 4, 4);
    tryPlace("blood", 1, 1, 18);
    tryPlace("debris", 1, 1, 18);
    tryPlace("crater", 1, 1, 5);
    tryPlace("flag", 1, 1, 2);
  }

  const spawnRng = mulberry32(meta.id * 31337);
  const findFreeTile = (tries = 100) => {
    for (let i = 0; i < tries; i++) {
      const x = Math.floor(spawnRng() * 48);
      const y = Math.floor(spawnRng() * 48);
      if (occ[y*48+x] === 0) return { x, y };
    }
    return { x: 5, y: 5 };
  };

  const zCount = 3 + meta.difficulty * 2;
  const zombies = [];
  for (let i = 0; i < zCount; i++) zombies.push(findFreeTile());

  return { id: meta.id, theme: meta.theme, name: meta.name, pl_name: meta.pl_name, difficulty: meta.difficulty, buildings, props, zombies };
}

const SEGMENTS = SEGMENTS_META.map(generateSegment);
const GATES = [
  { id: "g1_2", type: "bridge", from: 1, to: 2, label: "Old River Bridge" },
  { id: "g2_3", type: "breach", from: 2, to: 3, label: "Wall Breach" },
  { id: "g3_4", type: "bridge", from: 3, to: 4, label: "Service Overpass" },
  { id: "g4_5", type: "gate",   from: 4, to: 5, label: "Checkpoint Alpha" },
];

console.log(JSON.stringify({ segments: SEGMENTS, gates: GATES }, null, 2));

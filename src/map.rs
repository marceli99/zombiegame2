use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::pixelart::{Canvas, Rgba};

pub const TILE_SIZE: f32 = 64.0;
pub const MAP_COLS: i32 = 33;
pub const MAP_ROWS: i32 = 21;
pub const MAP_WIDTH: f32 = MAP_COLS as f32 * TILE_SIZE;
pub const MAP_HEIGHT: f32 = MAP_ROWS as f32 * TILE_SIZE;

#[derive(Clone, Copy)]
pub struct Obstacle {
    pub pos: Vec2,
    pub radius: f32,
}

#[derive(Resource, Default)]
pub struct MapObstacles {
    pub list: Vec<Obstacle>,
}

impl MapObstacles {
    pub fn resolve(&self, pos: &mut Vec2, own_radius: f32) {
        for o in &self.list {
            let delta = *pos - o.pos;
            let min_dist = o.radius + own_radius;
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
    }

    pub fn hits(&self, pos: Vec2, own_radius: f32) -> bool {
        for o in &self.list {
            let min_dist = o.radius + own_radius;
            if pos.distance_squared(o.pos) < min_dist * min_dist {
                return true;
            }
        }
        false
    }
}

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapObstacles>()
            .add_systems(Startup, spawn_map);
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut obstacles: ResMut<MapObstacles>,
) {
    let tile_mesh = meshes.add(Rectangle::new(TILE_SIZE, TILE_SIZE));
    let tuft_mesh = meshes.add(Rectangle::new(6.0, 6.0));
    let blade_mesh = meshes.add(Rectangle::new(2.0, 5.0));
    let pebble_mesh = meshes.add(Rectangle::new(4.0, 4.0));

    let grass_mats = [
        materials.add(Color::srgb(0.18, 0.42, 0.14)),
        materials.add(Color::srgb(0.22, 0.48, 0.16)),
        materials.add(Color::srgb(0.15, 0.36, 0.11)),
        materials.add(Color::srgb(0.24, 0.46, 0.17)),
        materials.add(Color::srgb(0.19, 0.40, 0.12)),
    ];
    let path_mats = [
        materials.add(Color::srgb(0.58, 0.52, 0.40)),
        materials.add(Color::srgb(0.52, 0.46, 0.34)),
        materials.add(Color::srgb(0.64, 0.56, 0.42)),
        materials.add(Color::srgb(0.55, 0.49, 0.37)),
    ];
    let grass_detail_mats = [
        materials.add(Color::srgb(0.30, 0.58, 0.20)),
        materials.add(Color::srgb(0.36, 0.66, 0.24)),
        materials.add(Color::srgb(0.22, 0.46, 0.14)),
        materials.add(Color::srgb(0.42, 0.72, 0.28)),
    ];
    let pebble_mats = [
        materials.add(Color::srgb(0.72, 0.68, 0.56)),
        materials.add(Color::srgb(0.60, 0.56, 0.45)),
        materials.add(Color::srgb(0.82, 0.78, 0.64)),
    ];

    let tree_image = images.add(build_tree_image());
    let bush_image = images.add(build_bush_image());
    let bench_image = images.add(build_bench_image());
    let fountain_image = images.add(build_fountain_image());
    let lamp_image = images.add(build_lamp_image());
    let flower_image = images.add(build_flower_image());

    let mut rng = StdRng::seed_from_u64(42);

    let start_x = -(MAP_COLS as f32 / 2.0) * TILE_SIZE + TILE_SIZE / 2.0;
    let start_y = -(MAP_ROWS as f32 / 2.0) * TILE_SIZE + TILE_SIZE / 2.0;
    let path_row = MAP_ROWS / 2;
    let path_col = MAP_COLS / 2;

    for row in 0..MAP_ROWS {
        for col in 0..MAP_COLS {
            let x = start_x + col as f32 * TILE_SIZE;
            let y = start_y + row as f32 * TILE_SIZE;
            let is_path = row == path_row || col == path_col;
            let material = if is_path {
                path_mats[rng.gen_range(0..path_mats.len())].clone()
            } else {
                grass_mats[rng.gen_range(0..grass_mats.len())].clone()
            };
            commands.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(tile_mesh.clone()),
                material,
                transform: Transform::from_xyz(x, y, -10.0),
                ..default()
            });
        }
    }

    let half_w = (MAP_COLS as f32 / 2.0) * TILE_SIZE - 10.0;
    let half_h = (MAP_ROWS as f32 / 2.0) * TILE_SIZE - 10.0;
    let path_y_min = start_y + path_row as f32 * TILE_SIZE - TILE_SIZE / 2.0;
    let path_y_max = path_y_min + TILE_SIZE;
    let path_x_min = start_x + path_col as f32 * TILE_SIZE - TILE_SIZE / 2.0;
    let path_x_max = path_x_min + TILE_SIZE;

    let is_on_path = |p: Vec2| -> bool {
        (p.y >= path_y_min && p.y <= path_y_max) || (p.x >= path_x_min && p.x <= path_x_max)
    };

    let near_origin = |p: Vec2| -> bool { p.length_squared() < 140.0 * 140.0 };

    let rand_grass_pos = |rng: &mut StdRng| -> Vec2 {
        loop {
            let p = Vec2::new(
                rng.gen_range(-half_w..half_w),
                rng.gen_range(-half_h..half_h),
            );
            if !is_on_path(p) {
                return p;
            }
        }
    };

    for _ in 0..360 {
        let p = rand_grass_pos(&mut rng);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(tuft_mesh.clone()),
            material: grass_detail_mats[rng.gen_range(0..grass_detail_mats.len())].clone(),
            transform: Transform::from_xyz(p.x, p.y, -9.4),
            ..default()
        });
    }
    for _ in 0..520 {
        let p = rand_grass_pos(&mut rng);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(blade_mesh.clone()),
            material: grass_detail_mats[rng.gen_range(0..grass_detail_mats.len())].clone(),
            transform: Transform::from_xyz(p.x, p.y, -9.35),
            ..default()
        });
    }

    let rand_path_pos = |rng: &mut StdRng| -> Vec2 {
        loop {
            let p = Vec2::new(
                rng.gen_range(-half_w..half_w),
                rng.gen_range(-half_h..half_h),
            );
            if is_on_path(p) {
                return p;
            }
        }
    };
    for _ in 0..220 {
        let p = rand_path_pos(&mut rng);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(pebble_mesh.clone()),
            material: pebble_mats[rng.gen_range(0..pebble_mats.len())].clone(),
            transform: Transform::from_xyz(p.x, p.y, -9.3),
            ..default()
        });
    }

    let flower_tints = [
        Color::srgb(1.0, 0.55, 0.78),
        Color::srgb(1.0, 0.90, 0.35),
        Color::srgb(0.96, 0.96, 1.0),
        Color::srgb(0.85, 0.45, 0.95),
        Color::srgb(1.0, 0.42, 0.38),
        Color::srgb(0.55, 0.75, 1.0),
    ];
    for _ in 0..420 {
        let p = rand_grass_pos(&mut rng);
        let tint = flower_tints[rng.gen_range(0..flower_tints.len())];
        commands.spawn(SpriteBundle {
            texture: flower_image.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(12.0, 12.0)),
                color: tint,
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -9.0),
            ..default()
        });
    }

    // Bushes (obstacles)
    let mut bush_count = 0;
    let mut attempts = 0;
    while bush_count < 48 && attempts < 500 {
        attempts += 1;
        let p = rand_grass_pos(&mut rng);
        if near_origin(p) {
            continue;
        }
        if obstacles.hits(p, 30.0) {
            continue;
        }
        commands.spawn(SpriteBundle {
            texture: bush_image.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(44.0, 44.0)),
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -8.0),
            ..default()
        });
        obstacles.list.push(Obstacle { pos: p, radius: 16.0 });
        bush_count += 1;
    }

    // Trees (obstacles, perimeter)
    let tree_positions: Vec<Vec2> = {
        let mut v = Vec::new();
        let tree_half_w = (MAP_COLS as f32 / 2.0 - 0.8) * TILE_SIZE;
        let tree_half_h = (MAP_ROWS as f32 / 2.0 - 0.8) * TILE_SIZE;
        for i in 0..10 {
            let t = (i as f32 + 0.5) / 10.0;
            let x = -tree_half_w + t * 2.0 * tree_half_w;
            v.push(Vec2::new(
                x + rng.gen_range(-14.0..14.0),
                tree_half_h + rng.gen_range(-14.0..14.0),
            ));
            v.push(Vec2::new(
                x + rng.gen_range(-14.0..14.0),
                -tree_half_h + rng.gen_range(-14.0..14.0),
            ));
        }
        let side_ys = [-520.0, -320.0, -120.0, 120.0, 320.0, 520.0];
        for y_base in side_ys {
            v.push(Vec2::new(
                -tree_half_w + rng.gen_range(-14.0..14.0),
                y_base + rng.gen_range(-12.0..12.0),
            ));
            v.push(Vec2::new(
                tree_half_w + rng.gen_range(-14.0..14.0),
                y_base + rng.gen_range(-12.0..12.0),
            ));
        }
        v
    };
    for p in &tree_positions {
        if is_on_path(*p) {
            continue;
        }
        commands.spawn(SpriteBundle {
            texture: tree_image.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(96.0, 96.0)),
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -4.0),
            ..default()
        });
        obstacles.list.push(Obstacle { pos: *p, radius: 30.0 });
    }

    // Benches (obstacles)
    let bench_positions = [
        Vec2::new(-260.0, path_y_max + 30.0),
        Vec2::new(260.0, path_y_max + 30.0),
        Vec2::new(-260.0, path_y_min - 30.0),
        Vec2::new(260.0, path_y_min - 30.0),
        Vec2::new(-520.0, path_y_max + 30.0),
        Vec2::new(520.0, path_y_max + 30.0),
        Vec2::new(-520.0, path_y_min - 30.0),
        Vec2::new(520.0, path_y_min - 30.0),
    ];
    for p in bench_positions {
        commands.spawn(SpriteBundle {
            texture: bench_image.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(72.0, 30.0)),
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -6.0),
            ..default()
        });
        obstacles.list.push(Obstacle {
            pos: Vec2::new(p.x - 22.0, p.y),
            radius: 16.0,
        });
        obstacles.list.push(Obstacle {
            pos: Vec2::new(p.x + 22.0, p.y),
            radius: 16.0,
        });
    }

    // Lamp posts (obstacles)
    let lamp_positions = [
        Vec2::new(path_x_min - 30.0, path_y_max + 30.0),
        Vec2::new(path_x_max + 30.0, path_y_max + 30.0),
        Vec2::new(path_x_min - 30.0, path_y_min - 30.0),
        Vec2::new(path_x_max + 30.0, path_y_min - 30.0),
        Vec2::new(-380.0, path_y_max + 30.0),
        Vec2::new(380.0, path_y_max + 30.0),
        Vec2::new(-380.0, path_y_min - 30.0),
        Vec2::new(380.0, path_y_min - 30.0),
    ];
    for p in lamp_positions {
        commands.spawn(SpriteBundle {
            texture: lamp_image.clone(),
            sprite: Sprite {
                custom_size: Some(Vec2::new(36.0, 36.0)),
                ..default()
            },
            transform: Transform::from_xyz(p.x, p.y, -5.0),
            ..default()
        });
        obstacles.list.push(Obstacle { pos: p, radius: 7.0 });
    }

    // Central fountain (obstacle)
    commands.spawn(SpriteBundle {
        texture: fountain_image.clone(),
        sprite: Sprite {
            custom_size: Some(Vec2::new(96.0, 96.0)),
            ..default()
        },
        transform: Transform::from_xyz(0.0, 0.0, -6.5),
        ..default()
    });
    obstacles.list.push(Obstacle {
        pos: Vec2::ZERO,
        radius: 42.0,
    });
}

fn build_tree_image() -> Image {
    let outline: Rgba = [10, 22, 8, 255];
    let leaf_dark: Rgba = [22, 55, 18, 255];
    let leaf_main: Rgba = [42, 94, 28, 255];
    let leaf_light: Rgba = [72, 132, 42, 255];
    let leaf_top: Rgba = [108, 172, 58, 255];
    let trunk: Rgba = [58, 34, 16, 255];

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

fn build_bush_image() -> Image {
    let outline: Rgba = [10, 22, 8, 255];
    let leaf_dark: Rgba = [30, 72, 22, 255];
    let leaf_main: Rgba = [52, 112, 36, 255];
    let leaf_light: Rgba = [88, 154, 52, 255];

    let mut c = Canvas::new(14, 14);

    c.fill_circle(7, 7, 6, outline);
    c.fill_circle(7, 7, 5, leaf_dark);
    c.fill_circle(6, 8, 4, leaf_main);
    c.fill_circle(5, 9, 2, leaf_light);
    c.put(9, 6, leaf_main);
    c.put(10, 8, leaf_dark);
    c.put(8, 4, leaf_main);
    c.put(4, 6, leaf_dark);

    c.into_image()
}

fn build_bench_image() -> Image {
    let outline: Rgba = [18, 10, 4, 255];
    let wood_dark: Rgba = [72, 40, 16, 255];
    let wood_main: Rgba = [112, 66, 28, 255];
    let wood_light: Rgba = [150, 92, 42, 255];
    let metal: Rgba = [52, 52, 58, 255];

    let mut c = Canvas::new(22, 9);

    c.fill_rect(1, 6, 20, 3, outline);
    c.fill_rect(2, 6, 18, 1, wood_dark);
    c.fill_rect(2, 7, 18, 1, wood_main);
    c.fill_rect(2, 8, 18, 1, wood_dark);

    c.fill_rect(1, 3, 20, 2, outline);
    c.fill_rect(2, 3, 18, 1, wood_main);
    c.fill_rect(2, 4, 18, 1, wood_light);

    c.fill_rect(2, 0, 2, 3, outline);
    c.fill_rect(18, 0, 2, 3, outline);
    c.put(3, 1, metal);
    c.put(19, 1, metal);

    c.into_image()
}

fn build_fountain_image() -> Image {
    let outline: Rgba = [28, 26, 30, 255];
    let stone_dark: Rgba = [98, 92, 84, 255];
    let stone_main: Rgba = [140, 134, 122, 255];
    let stone_light: Rgba = [182, 176, 162, 255];
    let water_dark: Rgba = [28, 78, 138, 255];
    let water_main: Rgba = [62, 132, 202, 255];
    let water_light: Rgba = [148, 206, 240, 255];
    let spray: Rgba = [225, 240, 255, 255];

    let mut c = Canvas::new(24, 24);

    c.fill_circle(12, 12, 11, outline);
    c.fill_circle(12, 12, 10, stone_dark);
    c.fill_circle(12, 12, 9, stone_main);
    c.put(5, 10, stone_light);
    c.put(6, 9, stone_light);
    c.put(17, 14, stone_light);
    c.put(18, 13, stone_light);

    c.fill_circle(12, 12, 8, outline);
    c.fill_circle(12, 12, 7, water_dark);
    c.fill_circle(12, 12, 6, water_main);
    c.put(8, 14, water_light);
    c.put(9, 15, water_light);
    c.put(14, 9, water_light);
    c.put(15, 10, water_light);
    c.put(10, 9, water_light);

    c.fill_circle(12, 12, 3, outline);
    c.fill_circle(12, 12, 2, stone_main);
    c.put(12, 12, stone_light);

    c.put(12, 15, spray);
    c.put(12, 16, spray);
    c.put(11, 17, spray);
    c.put(13, 17, spray);
    c.put(10, 18, water_light);
    c.put(14, 18, water_light);
    c.put(12, 18, spray);

    c.into_image()
}

fn build_lamp_image() -> Image {
    let outline: Rgba = [8, 8, 10, 255];
    let post: Rgba = [42, 42, 48, 255];
    let post_light: Rgba = [72, 72, 80, 255];
    let base: Rgba = [28, 28, 32, 255];
    let glass_dark: Rgba = [222, 180, 60, 255];
    let glass: Rgba = [255, 222, 110, 255];
    let glow: Rgba = [255, 250, 205, 255];

    let mut c = Canvas::new(11, 11);

    c.fill_rect(3, 0, 5, 1, outline);
    c.fill_rect(4, 0, 3, 1, base);

    c.fill_rect(4, 1, 3, 6, outline);
    c.fill_rect(5, 1, 1, 6, post);
    c.put(5, 3, post_light);
    c.put(5, 5, post_light);

    c.fill_rect(3, 7, 5, 3, outline);
    c.fill_rect(4, 7, 3, 3, glass_dark);
    c.put(5, 8, glass);
    c.put(5, 9, glow);

    c.fill_rect(4, 10, 3, 1, outline);

    c.into_image()
}

fn build_flower_image() -> Image {
    let petal: Rgba = [255, 255, 255, 255];
    let center: Rgba = [255, 232, 130, 255];

    let mut c = Canvas::new(5, 5);

    c.put(2, 0, petal);
    c.put(1, 1, petal);
    c.put(2, 1, petal);
    c.put(3, 1, petal);
    c.put(0, 2, petal);
    c.put(1, 2, petal);
    c.put(2, 2, center);
    c.put(3, 2, petal);
    c.put(4, 2, petal);
    c.put(1, 3, petal);
    c.put(2, 3, petal);
    c.put(3, 3, petal);
    c.put(2, 4, petal);

    c.into_image()
}

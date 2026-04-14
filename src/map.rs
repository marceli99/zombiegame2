use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

const TILE_SIZE: f32 = 64.0;
const MAP_COLS: i32 = 22;
const MAP_ROWS: i32 = 14;

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_map);
    }
}

fn spawn_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let tile_mesh = meshes.add(Rectangle::new(TILE_SIZE, TILE_SIZE));
    let plank_mesh = meshes.add(Rectangle::new(TILE_SIZE, 6.0));
    let tuft_mesh = meshes.add(Rectangle::new(6.0, 6.0));
    let blade_mesh = meshes.add(Rectangle::new(2.0, 5.0));
    let stone_mesh = meshes.add(Rectangle::new(14.0, 10.0));
    let pebble_mesh = meshes.add(Rectangle::new(4.0, 4.0));
    let blood_mesh = meshes.add(Rectangle::new(18.0, 10.0));
    let blood_drop_mesh = meshes.add(Rectangle::new(4.0, 3.0));
    let crack_mesh = meshes.add(Rectangle::new(26.0, 2.0));
    let plank_tile_mesh = meshes.add(Rectangle::new(30.0, 6.0));

    let ground_mats = [
        materials.add(Color::srgb(0.14, 0.13, 0.10)),
        materials.add(Color::srgb(0.11, 0.11, 0.09)),
        materials.add(Color::srgb(0.13, 0.16, 0.09)),
        materials.add(Color::srgb(0.17, 0.15, 0.11)),
        materials.add(Color::srgb(0.10, 0.12, 0.08)),
    ];
    let road_mat = materials.add(Color::srgb(0.19, 0.18, 0.16));
    let road_edge_mat = materials.add(Color::srgb(0.12, 0.11, 0.09));

    let grass_mats = [
        materials.add(Color::srgb(0.18, 0.28, 0.11)),
        materials.add(Color::srgb(0.22, 0.32, 0.12)),
        materials.add(Color::srgb(0.14, 0.22, 0.08)),
    ];
    let stone_mats = [
        materials.add(Color::srgb(0.30, 0.29, 0.26)),
        materials.add(Color::srgb(0.24, 0.23, 0.21)),
        materials.add(Color::srgb(0.36, 0.34, 0.30)),
    ];
    let blood_mat = materials.add(Color::srgb(0.24, 0.05, 0.05));
    let blood_dark_mat = materials.add(Color::srgb(0.14, 0.03, 0.03));
    let crack_mat = materials.add(Color::srgb(0.05, 0.05, 0.04));
    let plank_mat = materials.add(Color::srgb(0.22, 0.14, 0.07));
    let plank_dark_mat = materials.add(Color::srgb(0.14, 0.09, 0.04));

    let mut rng = StdRng::seed_from_u64(7);

    let start_x = -(MAP_COLS as f32 / 2.0) * TILE_SIZE + TILE_SIZE / 2.0;
    let start_y = -(MAP_ROWS as f32 / 2.0) * TILE_SIZE + TILE_SIZE / 2.0;
    let road_row = MAP_ROWS / 2;

    for row in 0..MAP_ROWS {
        for col in 0..MAP_COLS {
            let x = start_x + col as f32 * TILE_SIZE;
            let y = start_y + row as f32 * TILE_SIZE;
            let material = if row == road_row {
                road_mat.clone()
            } else {
                ground_mats[rng.gen_range(0..ground_mats.len())].clone()
            };
            commands.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(tile_mesh.clone()),
                material,
                transform: Transform::from_xyz(x, y, -10.0),
                ..default()
            });
        }
    }

    for col in 0..MAP_COLS {
        let x = start_x + col as f32 * TILE_SIZE;
        let y_top = start_y + road_row as f32 * TILE_SIZE + TILE_SIZE / 2.0 - 3.0;
        let y_bot = start_y + road_row as f32 * TILE_SIZE - TILE_SIZE / 2.0 + 3.0;
        for y in [y_top, y_bot] {
            commands.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(plank_mesh.clone()),
                material: road_edge_mat.clone(),
                transform: Transform::from_xyz(x, y, -9.9),
                ..default()
            });
        }
    }

    let half_w = (MAP_COLS as f32 / 2.0) * TILE_SIZE - 10.0;
    let half_h = (MAP_ROWS as f32 / 2.0) * TILE_SIZE - 10.0;
    let road_y_min = start_y + road_row as f32 * TILE_SIZE - TILE_SIZE / 2.0;
    let road_y_max = road_y_min + TILE_SIZE;

    let rand_pos = |rng: &mut StdRng| -> Vec2 {
        loop {
            let p = Vec2::new(
                rng.gen_range(-half_w..half_w),
                rng.gen_range(-half_h..half_h),
            );
            if p.y < road_y_min || p.y > road_y_max {
                return p;
            }
        }
    };

    for _ in 0..110 {
        let p = rand_pos(&mut rng);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(tuft_mesh.clone()),
            material: grass_mats[rng.gen_range(0..grass_mats.len())].clone(),
            transform: Transform::from_xyz(p.x, p.y, -9.4),
            ..default()
        });
    }
    for _ in 0..160 {
        let p = rand_pos(&mut rng);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(blade_mesh.clone()),
            material: grass_mats[rng.gen_range(0..grass_mats.len())].clone(),
            transform: Transform::from_xyz(p.x, p.y, -9.35),
            ..default()
        });
    }

    for _ in 0..36 {
        let p = rand_pos(&mut rng);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(stone_mesh.clone()),
            material: stone_mats[rng.gen_range(0..stone_mats.len())].clone(),
            transform: Transform::from_xyz(p.x, p.y, -9.2),
            ..default()
        });
    }
    for _ in 0..80 {
        let p = rand_pos(&mut rng);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(pebble_mesh.clone()),
            material: stone_mats[rng.gen_range(0..stone_mats.len())].clone(),
            transform: Transform::from_xyz(p.x, p.y, -9.25),
            ..default()
        });
    }

    for _ in 0..25 {
        let p = Vec2::new(
            rng.gen_range(-half_w..half_w),
            rng.gen_range(-half_h..half_h),
        );
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(crack_mesh.clone()),
            material: crack_mat.clone(),
            transform: Transform::from_xyz(p.x, p.y, -9.6)
                .with_rotation(Quat::from_rotation_z(angle)),
            ..default()
        });
    }

    for _ in 0..14 {
        let p = Vec2::new(
            rng.gen_range(-half_w..half_w),
            rng.gen_range(-half_h..half_h),
        );
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(blood_mesh.clone()),
            material: blood_dark_mat.clone(),
            transform: Transform::from_xyz(p.x, p.y, -9.5),
            ..default()
        });
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(blood_mesh.clone()),
            material: blood_mat.clone(),
            transform: Transform::from_xyz(p.x + 2.0, p.y + 1.0, -9.45),
            ..default()
        });
        for _ in 0..5 {
            let dx = rng.gen_range(-22.0..22.0);
            let dy = rng.gen_range(-14.0..14.0);
            commands.spawn(MaterialMesh2dBundle {
                mesh: Mesh2dHandle(blood_drop_mesh.clone()),
                material: blood_mat.clone(),
                transform: Transform::from_xyz(p.x + dx, p.y + dy, -9.44),
                ..default()
            });
        }
    }

    for _ in 0..18 {
        let cx = rng.gen_range(-half_w..half_w);
        let cy = rng.gen_range(-half_h..half_h);
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let mat = if rng.gen_bool(0.5) {
            plank_mat.clone()
        } else {
            plank_dark_mat.clone()
        };
        commands.spawn(MaterialMesh2dBundle {
            mesh: Mesh2dHandle(plank_tile_mesh.clone()),
            material: mat,
            transform: Transform::from_xyz(cx, cy, -9.3)
                .with_rotation(Quat::from_rotation_z(angle)),
            ..default()
        });
    }
}

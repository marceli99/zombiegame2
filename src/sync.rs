use bevy::prelude::*;
use std::collections::HashSet;

use crate::bullet::{
    spawn_bullet_entity, spawn_explosion_entity, Bullet, BulletAssets, Explosion,
    EXPLOSION_LIFETIME,
};
use crate::net::{
    broadcast, is_host, is_net_client, ClientInEvent, ClientMsg, LocalInput, NetBulletState,
    NetContext, NetEntities, NetExplosionState, NetMode, NetPickupState, NetPlayerState,
    NetSnapshot, NetZombieState, RemoteInputs, ServerEvent, ServerMsg,
};
use crate::player::{spawn_player_entity, Player, PlayerAssets};
use crate::wave::WaveState;
use crate::weapon::{
    spawn_health_entity, spawn_pickup_entity, HealthPickup, HealthPickupAssets, Weapon,
    WeaponAssets, WeaponPickup, HEALTH_PICKUP_KIND,
};
use crate::zombie::{spawn_zombie_entity, Zombie, ZombieAssets, ZombieKind};
use crate::{GameState, Score};

pub struct NetSyncPlugin;

impl Plugin for NetSyncPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (server_receive_inputs, server_broadcast_snapshot)
                .run_if(in_state(GameState::Playing))
                .run_if(is_host),
        )
        .add_systems(
            FixedUpdate,
            client_send_input
                .run_if(in_state(GameState::Playing))
                .run_if(is_net_client),
        )
        .add_systems(
            Update,
            client_apply_snapshots
                .run_if(in_state(GameState::Playing))
                .run_if(is_net_client),
        );
    }
}

fn server_receive_inputs(ctx: Res<NetContext>, mut remote: ResMut<RemoteInputs>) {
    let Some(host) = ctx.host.as_ref() else {
        return;
    };
    let events_arc = host.events.clone();
    let Ok(rx) = events_arc.lock() else {
        return;
    };
    while let Ok(e) = rx.try_recv() {
        match e {
            ServerEvent::Input { id, input } => {
                remote.0.insert(id, input);
            }
            ServerEvent::Connected { id } => {
                info!("Client {} connected mid-game (not spawning)", id);
            }
            ServerEvent::Disconnected { id } => {
                remote.0.remove(&id);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn server_broadcast_snapshot(
    ctx: Res<NetContext>,
    players: Query<(&Transform, &Player)>,
    zombies: Query<(&Transform, &crate::net::NetId, &Zombie)>,
    bullets: Query<(&Transform, &crate::net::NetId, &Bullet)>,
    pickups: Query<(&Transform, &crate::net::NetId, &WeaponPickup)>,
    health_pickups: Query<(&Transform, &crate::net::NetId), With<HealthPickup>>,
    explosions: Query<(&Transform, &crate::net::NetId, &Explosion)>,
    score: Res<Score>,
    wave: Res<WaveState>,
    game_state: Res<State<GameState>>,
    mut tick: Local<u64>,
) {
    let Some(host) = ctx.host.as_ref() else {
        return;
    };
    *tick += 1;

    let player_states: Vec<NetPlayerState> = players
        .iter()
        .map(|(t, p)| NetPlayerState {
            id: p.id,
            x: t.translation.x,
            y: t.translation.y,
            rot: t.rotation.to_euler(EulerRot::ZYX).0,
            hp: p.hp,
            weapon: p.active_weapon().as_u8(),
        })
        .collect();

    let zombie_states: Vec<NetZombieState> = zombies
        .iter()
        .map(|(t, id, z)| NetZombieState {
            id: id.0,
            x: t.translation.x,
            y: t.translation.y,
            rot: t.rotation.to_euler(EulerRot::ZYX).0,
            kind: z.kind.as_u8(),
        })
        .collect();

    let bullet_states: Vec<NetBulletState> = bullets
        .iter()
        .map(|(t, id, b)| NetBulletState {
            id: id.0,
            x: t.translation.x,
            y: t.translation.y,
            rot: t.rotation.to_euler(EulerRot::ZYX).0,
            is_rocket: b.is_rocket,
        })
        .collect();

    let mut pickup_states: Vec<NetPickupState> = pickups
        .iter()
        .map(|(t, id, pk)| NetPickupState {
            id: id.0,
            x: t.translation.x,
            y: t.translation.y,
            kind: pk.kind.as_u8(),
        })
        .collect();
    for (t, id) in &health_pickups {
        pickup_states.push(NetPickupState {
            id: id.0,
            x: t.translation.x,
            y: t.translation.y,
            kind: HEALTH_PICKUP_KIND,
        });
    }

    let explosion_states: Vec<NetExplosionState> = explosions
        .iter()
        .map(|(t, id, exp)| NetExplosionState {
            id: id.0,
            x: t.translation.x,
            y: t.translation.y,
            radius: exp.radius,
            remaining: exp.lifetime,
        })
        .collect();

    let snap = NetSnapshot {
        tick: *tick,
        players: player_states,
        zombies: zombie_states,
        bullets: bullet_states,
        pickups: pickup_states,
        explosions: explosion_states,
        score: score.0,
        wave: wave.current_wave,
        in_break: wave.in_break,
        break_secs: wave.break_timer.remaining_secs(),
        zombies_to_spawn: wave.zombies_to_spawn,
        game_over: *game_state.get() == GameState::GameOver,
    };

    broadcast(host, &ServerMsg::Snapshot(Box::new(snap)));
}

fn client_send_input(ctx: Res<NetContext>, local: Res<LocalInput>) {
    if let Some(client) = ctx.client.as_ref() {
        let _ = client.sender.send(ClientMsg::Input(local.0));
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn client_apply_snapshots(
    mut commands: Commands,
    ctx: ResMut<NetContext>,
    mut mode: ResMut<NetMode>,
    mut net_entities: ResMut<NetEntities>,
    player_assets: Res<PlayerAssets>,
    zombie_assets: Res<ZombieAssets>,
    bullet_assets: Res<BulletAssets>,
    weapon_assets: Res<WeaponAssets>,
    health_assets: Res<HealthPickupAssets>,
    mut players: Query<
        (&mut Transform, &mut Player),
        (Without<Zombie>, Without<Bullet>, Without<Explosion>),
    >,
    mut zombies: Query<
        &mut Transform,
        (With<Zombie>, Without<Player>, Without<Bullet>, Without<Explosion>),
    >,
    mut bullets: Query<
        &mut Transform,
        (With<Bullet>, Without<Player>, Without<Zombie>, Without<Explosion>),
    >,
    mut explosions: Query<
        (&mut Explosion, &mut Sprite),
        (Without<Player>, Without<Zombie>, Without<Bullet>),
    >,
    mut score: ResMut<Score>,
    mut wave: ResMut<WaveState>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let Some(client) = ctx.client.as_ref() else {
        return;
    };
    let events_arc = client.events.clone();

    let mut latest: Option<Box<NetSnapshot>> = None;
    let mut disconnect = false;
    {
        let Ok(rx) = events_arc.lock() else {
            return;
        };
        while let Ok(e) = rx.try_recv() {
            match e {
                ClientInEvent::Snapshot(s) => {
                    latest = Some(s);
                }
                ClientInEvent::Disconnected | ClientInEvent::FullLobby => {
                    disconnect = true;
                }
                _ => {}
            }
        }
    }

    if disconnect {
        let mut ctx = ctx;
        ctx.disconnect();
        *mode = NetMode::SinglePlayer;
        net_entities.clear();
        next_state.set(GameState::Menu);
        return;
    }

    let Some(snap) = latest else {
        return;
    };

    score.0 = snap.score;
    wave.current_wave = snap.wave;
    wave.in_break = snap.in_break;
    wave.zombies_to_spawn = snap.zombies_to_spawn;
    wave.break_timer = Timer::from_seconds(snap.break_secs.max(0.01), TimerMode::Once);

    let mut seen_players: HashSet<u8> = HashSet::new();
    for np in &snap.players {
        seen_players.insert(np.id);
        match net_entities.players.get(&np.id).copied() {
            Some(ent) => {
                if let Ok((mut t, mut p)) = players.get_mut(ent) {
                    t.translation.x = np.x;
                    t.translation.y = np.y;
                    t.rotation = Quat::from_rotation_z(np.rot);
                    p.hp = np.hp;
                    let w = Weapon::from_u8(np.weapon);
                    p.slots[0] = Some(w);
                    p.active_slot = 0;
                }
            }
            None => {
                let ent = spawn_player_entity(
                    &mut commands,
                    &player_assets,
                    np.id,
                    Vec2::new(np.x, np.y),
                );
                net_entities.players.insert(np.id, ent);
            }
        }
    }
    let stale_players: Vec<u8> = net_entities
        .players
        .keys()
        .filter(|k| !seen_players.contains(k))
        .copied()
        .collect();
    for k in stale_players {
        if let Some(ent) = net_entities.players.remove(&k) {
            commands.entity(ent).despawn_recursive();
        }
    }

    let mut seen_zombies: HashSet<u32> = HashSet::new();
    for nz in &snap.zombies {
        seen_zombies.insert(nz.id);
        let kind = ZombieKind::from_u8(nz.kind);
        match net_entities.zombies.get(&nz.id).copied() {
            Some(ent) => {
                if let Ok(mut t) = zombies.get_mut(ent) {
                    t.translation.x = nz.x;
                    t.translation.y = nz.y;
                    t.rotation = Quat::from_rotation_z(nz.rot);
                }
            }
            None => {
                let ent = spawn_zombie_entity(
                    &mut commands,
                    &zombie_assets,
                    Vec2::new(nz.x, nz.y),
                    nz.id,
                    kind.base_hp(),
                    kind.base_speed(),
                    kind,
                );
                net_entities.zombies.insert(nz.id, ent);
            }
        }
    }
    let stale_zombies: Vec<u32> = net_entities
        .zombies
        .keys()
        .filter(|k| !seen_zombies.contains(k))
        .copied()
        .collect();
    for k in stale_zombies {
        if let Some(ent) = net_entities.zombies.remove(&k) {
            commands.entity(ent).despawn_recursive();
        }
    }

    let mut seen_bullets: HashSet<u32> = HashSet::new();
    for nb in &snap.bullets {
        seen_bullets.insert(nb.id);
        match net_entities.bullets.get(&nb.id).copied() {
            Some(ent) => {
                if let Ok(mut t) = bullets.get_mut(ent) {
                    t.translation.x = nb.x;
                    t.translation.y = nb.y;
                    t.rotation = Quat::from_rotation_z(nb.rot);
                }
            }
            None => {
                let ent = spawn_bullet_entity(
                    &mut commands,
                    &bullet_assets,
                    Vec2::new(nb.x, nb.y),
                    Vec2::new(nb.rot.cos(), nb.rot.sin()),
                    0.0,
                    0,
                    nb.id,
                    nb.is_rocket,
                );
                net_entities.bullets.insert(nb.id, ent);
            }
        }
    }
    let stale_bullets: Vec<u32> = net_entities
        .bullets
        .keys()
        .filter(|k| !seen_bullets.contains(k))
        .copied()
        .collect();
    for k in stale_bullets {
        if let Some(ent) = net_entities.bullets.remove(&k) {
            commands.entity(ent).despawn_recursive();
        }
    }

    let mut seen_pickups: HashSet<u32> = HashSet::new();
    for np in &snap.pickups {
        seen_pickups.insert(np.id);
        use std::collections::hash_map::Entry;
        if let Entry::Vacant(entry) = net_entities.pickups.entry(np.id) {
            let ent = if np.kind == HEALTH_PICKUP_KIND {
                spawn_health_entity(
                    &mut commands,
                    &health_assets,
                    Vec2::new(np.x, np.y),
                    np.id,
                )
            } else {
                let kind = Weapon::from_u8(np.kind);
                spawn_pickup_entity(
                    &mut commands,
                    &weapon_assets,
                    Vec2::new(np.x, np.y),
                    kind,
                    np.id,
                )
            };
            entry.insert(ent);
        }
    }
    let stale_pickups: Vec<u32> = net_entities
        .pickups
        .keys()
        .filter(|k| !seen_pickups.contains(k))
        .copied()
        .collect();
    for k in stale_pickups {
        if let Some(ent) = net_entities.pickups.remove(&k) {
            commands.entity(ent).despawn_recursive();
        }
    }

    let mut seen_explosions: HashSet<u32> = HashSet::new();
    for ne in &snap.explosions {
        seen_explosions.insert(ne.id);
        match net_entities.explosions.get(&ne.id).copied() {
            Some(ent) => {
                if let Ok((mut exp, mut sprite)) = explosions.get_mut(ent) {
                    exp.lifetime = ne.remaining;
                    exp.radius = ne.radius;
                    let t = (ne.remaining / EXPLOSION_LIFETIME).clamp(0.0, 1.0);
                    let phase = 1.0 - t;
                    let scale = 1.1 + phase * 1.0;
                    sprite.custom_size = Some(Vec2::splat(ne.radius * scale));
                }
            }
            None => {
                let ent = spawn_explosion_entity(
                    &mut commands,
                    &bullet_assets,
                    Vec2::new(ne.x, ne.y),
                    ne.radius,
                    ne.id,
                );
                net_entities.explosions.insert(ne.id, ent);
            }
        }
    }
    let stale_explosions: Vec<u32> = net_entities
        .explosions
        .keys()
        .filter(|k| !seen_explosions.contains(k))
        .copied()
        .collect();
    for k in stale_explosions {
        if let Some(ent) = net_entities.explosions.remove(&k) {
            commands.entity(ent).despawn_recursive();
        }
    }

    if snap.game_over {
        next_state.set(GameState::GameOver);
    }
}

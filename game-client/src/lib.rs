use bevy::prelude::*;
use bevy::window::WindowResolution;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use wasm_bindgen::prelude::*;

const TILE_SIZE: f32 = 32.0;
const PLAYER_SIZE: f32 = 22.0;
const MOVE_SPEED: f32 = 220.0;
const REMOTE_LERP_RATE: f32 = 16.0;
const SNAPSHOT_Z: f32 = 4.0;
const FLOOR_Z: f32 = 0.0;

static INBOUND_SNAPSHOTS: Lazy<Mutex<Vec<SnapshotPayload>>> = Lazy::new(|| Mutex::new(Vec::new()));
static OUTBOUND_MOVES: Lazy<Mutex<Vec<MoveEvent>>> = Lazy::new(|| Mutex::new(Vec::new()));
static NEXT_PLAYER_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlayerState {
    id: String,
    x: f32,
    y: f32,
    connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotPayload {
    players: Vec<PlayerState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MoveEvent {
    x: f32,
    y: f32,
}

#[derive(Component)]
struct Actor {
    id: String,
}

#[derive(Component)]
struct LocalActor;

#[derive(Component)]
struct RemoteActor;

#[derive(Component)]
struct RemoteTarget(Vec2);

#[derive(Resource, Default)]
struct CurrentPlayerId(Option<String>);

#[derive(Resource, Default)]
struct LastSentPosition(Option<Vec2>);

#[wasm_bindgen]
pub fn boot_game(canvas_id: String) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let mut started = STARTED.lock().map_err(|_| JsValue::from_str("mutex poisoned"))?;
    if *started {
        return Ok(());
    }
    *started = true;
    drop(started);

    let primary_window = Window {
        canvas: Some(format!("#{canvas_id}")),
        fit_canvas_to_parent: true,
        prevent_default_event_handling: false,
        resolution: WindowResolution::new(1280.0, 720.0),
        ..default()
    };

    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb_u8(3, 10, 22)))
        .insert_resource(CurrentPlayerId::default())
        .insert_resource(LastSentPosition::default())
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(primary_window),
            ..default()
        }))
        .add_systems(Startup, setup_world)
        .add_systems(
            Update,
            (
                sync_player_id,
                local_movement,
                follow_camera,
                apply_latest_snapshot,
                smooth_remote_motion,
                emit_move_updates,
            ),
        );

    app.run();
    Ok(())
}

#[wasm_bindgen]
pub fn set_player_id(player_id: String) {
    if let Ok(mut pending_id) = NEXT_PLAYER_ID.lock() {
        *pending_id = Some(player_id);
    }
}

#[wasm_bindgen]
pub fn push_snapshot(snapshot_json: String) -> Result<(), JsValue> {
    let snapshot = serde_json::from_str::<SnapshotPayload>(&snapshot_json)
        .map_err(|error| JsValue::from_str(&format!("invalid snapshot payload: {error}")))?;

    let mut queue = INBOUND_SNAPSHOTS
        .lock()
        .map_err(|_| JsValue::from_str("snapshot queue mutex poisoned"))?;
    queue.push(snapshot);
    if queue.len() > 8 {
        let overflow = queue.len() - 8;
        queue.drain(0..overflow);
    }

    Ok(())
}

#[wasm_bindgen]
pub fn drain_move_events() -> String {
    let mut queue = match OUTBOUND_MOVES.lock() {
        Ok(queue) => queue,
        Err(_) => return "[]".to_string(),
    };

    if queue.is_empty() {
        return "[]".to_string();
    }

    let drained: Vec<MoveEvent> = queue.drain(..).collect();
    serde_json::to_string(&drained).unwrap_or_else(|_| "[]".to_string())
}

fn setup_world(mut commands: Commands) {
    commands.spawn(Camera2dBundle::default());

    for x in -25..=25 {
        for y in -25..=25 {
            let tint = if (x + y) % 2 == 0 {
                Color::srgb_u8(24, 44, 33)
            } else {
                Color::srgb_u8(21, 35, 27)
            };

            commands.spawn(SpriteBundle {
                sprite: Sprite {
                    color: tint,
                    custom_size: Some(Vec2::splat(TILE_SIZE - 1.0)),
                    ..default()
                },
                transform: Transform::from_xyz(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE, FLOOR_Z),
                ..default()
            });
        }
    }

    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: Color::srgb_u8(34, 211, 238),
                custom_size: Some(Vec2::splat(PLAYER_SIZE)),
                ..default()
            },
            transform: Transform::from_xyz(0.0, 0.0, SNAPSHOT_Z),
            ..default()
        },
        Actor {
            id: "local-pending".to_string(),
        },
        LocalActor,
    ));
}

fn sync_player_id(
    mut current_player_id: ResMut<CurrentPlayerId>,
    mut local_actor_query: Query<&mut Actor, With<LocalActor>>,
) {
    let next_id = match NEXT_PLAYER_ID.lock() {
        Ok(mut pending) => pending.take(),
        Err(_) => None,
    };

    if let Some(player_id) = next_id {
        current_player_id.0 = Some(player_id.clone());

        if let Ok(mut local_actor) = local_actor_query.get_single_mut() {
            local_actor.id = player_id;
        }
    }
}

fn local_movement(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut local_transform_query: Query<&mut Transform, With<LocalActor>>,
) {
    let mut direction = Vec2::ZERO;

    if input.pressed(KeyCode::KeyW) || input.pressed(KeyCode::ArrowUp) {
        direction.y += 1.0;
    }
    if input.pressed(KeyCode::KeyS) || input.pressed(KeyCode::ArrowDown) {
        direction.y -= 1.0;
    }
    if input.pressed(KeyCode::KeyA) || input.pressed(KeyCode::ArrowLeft) {
        direction.x -= 1.0;
    }
    if input.pressed(KeyCode::KeyD) || input.pressed(KeyCode::ArrowRight) {
        direction.x += 1.0;
    }

    if direction == Vec2::ZERO {
        return;
    }

    let Ok(mut transform) = local_transform_query.get_single_mut() else {
        return;
    };

    let delta = direction.normalize() * MOVE_SPEED * time.delta_seconds();
    transform.translation.x += delta.x;
    transform.translation.y += delta.y;
}

fn follow_camera(
    local_transform_query: Query<&Transform, (With<LocalActor>, Without<Camera2d>)>,
    mut camera_query: Query<&mut Transform, (With<Camera2d>, Without<LocalActor>)>,
) {
    let Ok(local_transform) = local_transform_query.get_single() else {
        return;
    };
    let Ok(mut camera_transform) = camera_query.get_single_mut() else {
        return;
    };

    camera_transform.translation.x = local_transform.translation.x;
    camera_transform.translation.y = local_transform.translation.y;
}

fn apply_latest_snapshot(
    mut commands: Commands,
    current_player_id: Res<CurrentPlayerId>,
    remote_query: Query<(Entity, &Actor), (With<RemoteActor>, Without<LocalActor>)>,
) {
    let latest_snapshot = {
        let mut queue = match INBOUND_SNAPSHOTS.lock() {
            Ok(queue) => queue,
            Err(_) => return,
        };

        let mut latest = None;
        for snapshot in queue.drain(..) {
            latest = Some(snapshot);
        }
        latest
    };

    let Some(snapshot) = latest_snapshot else {
        return;
    };

    let mut remote_entities: HashMap<String, Entity> = remote_query
        .iter()
        .map(|(entity, actor)| (actor.id.clone(), entity))
        .collect();

    for player in snapshot.players.into_iter().filter(|state| state.connected) {
        let is_local = current_player_id
            .0
            .as_deref()
            .is_some_and(|player_id| player_id == player.id);

        if is_local {
            if let Some(entity) = remote_entities.remove(&player.id) {
                commands.entity(entity).despawn_recursive();
            }

            continue;
        }

        if let Some(entity) = remote_entities.remove(&player.id) {
            commands
                .entity(entity)
                .insert(RemoteTarget(Vec2::new(player.x, player.y)));
        } else {
            spawn_remote_actor(&mut commands, &player);
        }
    }

    for entity in remote_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }
}

fn smooth_remote_motion(
    time: Res<Time>,
    mut remote_query: Query<(&mut Transform, &RemoteTarget), With<RemoteActor>>,
) {
    let blend = 1.0 - (-REMOTE_LERP_RATE * time.delta_seconds()).exp();
    let blend = blend.clamp(0.0, 1.0);

    for (mut transform, target) in &mut remote_query {
        let current = transform.translation.truncate();
        let next = current.lerp(target.0, blend);
        transform.translation.x = next.x;
        transform.translation.y = next.y;
    }
}

fn emit_move_updates(
    mut last_sent_position: ResMut<LastSentPosition>,
    local_transform_query: Query<&Transform, With<LocalActor>>,
) {
    let Ok(local_transform) = local_transform_query.get_single() else {
        return;
    };

    let current_position = local_transform.translation.truncate();
    let should_send = last_sent_position
        .0
        .is_none_or(|last| last.distance_squared(current_position) > 0.01);

    if !should_send {
        return;
    }

    last_sent_position.0 = Some(current_position);

    if let Ok(mut queue) = OUTBOUND_MOVES.lock() {
        queue.push(MoveEvent {
            x: current_position.x,
            y: current_position.y,
        });

        if queue.len() > 64 {
            let overflow = queue.len() - 64;
            queue.drain(0..overflow);
        }
    }
}

fn spawn_remote_actor(commands: &mut Commands, player: &PlayerState) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: Color::srgb_u8(245, 158, 11),
                custom_size: Some(Vec2::splat(PLAYER_SIZE)),
                ..default()
            },
            transform: Transform::from_xyz(player.x, player.y, SNAPSHOT_Z),
            ..default()
        },
        Actor {
            id: player.id.clone(),
        },
        RemoteActor,
        RemoteTarget(Vec2::new(player.x, player.y)),
    ));
}

use bevy::prelude::*;
use bevy::window::WindowResolution;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sim_core::{movement_step, InputState as CoreInputState};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use uuid::Uuid;
use wasm_bindgen::prelude::*;

const TILE_SIZE: f32 = 32.0;
const PLAYER_SIZE: f32 = 22.0;
const STRUCTURE_SIZE: f32 = 18.0;
const PROJECTILE_SIZE: f32 = 8.0;
const MAP_LIMIT: f32 = 5000.0;
const MOVE_SPEED: f32 = 220.0;
const PROJECTILE_SPEED: f32 = 760.0;
const PROJECTILE_TTL_SECONDS: f32 = 1.8;
const CLIENT_SIM_HZ: f32 = 60.0;
const CLIENT_SIM_DT: f32 = 1.0 / CLIENT_SIM_HZ;
const MAX_SIM_STEPS_PER_FRAME: usize = 8;
const MAX_INPUT_HISTORY: usize = 512;
const MAX_OUTBOUND_INPUTS: usize = 256;
const MAX_OUTBOUND_FEATURE_COMMANDS: usize = 128;
const REMOTE_LERP_RATE: f32 = 18.0;
const PROJECTILE_RECONCILE_BLEND_RATE: f32 = 10.0;
const PROJECTILE_RECONCILE_HARD_SNAP_DISTANCE: f32 = 140.0;
const SNAPSHOT_Z: f32 = 4.0;
const STRUCTURE_Z: f32 = 3.0;
const PROJECTILE_Z: f32 = 5.0;
const FLOOR_Z: f32 = 0.0;

static INBOUND_SNAPSHOTS: Lazy<Mutex<Vec<SnapshotPayload>>> = Lazy::new(|| Mutex::new(Vec::new()));
static OUTBOUND_INPUTS: Lazy<Mutex<Vec<InputCommand>>> = Lazy::new(|| Mutex::new(Vec::new()));
static OUTBOUND_FEATURE_COMMANDS: Lazy<Mutex<Vec<OutboundFeatureCommand>>> =
    Lazy::new(|| Mutex::new(Vec::new()));
static NEXT_PLAYER_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlayerState {
    id: String,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StructureState {
    id: String,
    x: f32,
    y: f32,
    kind: String,
    #[serde(rename = "ownerId")]
    owner_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectileState {
    id: String,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    #[serde(rename = "ownerId")]
    owner_id: String,
    #[serde(rename = "clientProjectileId")]
    client_projectile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotPayload {
    #[serde(rename = "serverTick")]
    server_tick: u32,
    #[serde(rename = "simRateHz")]
    sim_rate_hz: u32,
    #[serde(rename = "localAckSeq")]
    local_ack_seq: u32,
    #[serde(rename = "renderDelayMs", default)]
    render_delay_ms: f32,
    players: Vec<PlayerState>,
    #[serde(default)]
    structures: Vec<StructureState>,
    #[serde(default)]
    projectiles: Vec<ProjectileState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct InputState {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InputCommand {
    seq: u32,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutboundFeatureCommand {
    feature: String,
    action: String,
    payload: Value,
}

#[derive(Clone)]
struct InputHistoryEntry {
    seq: u32,
    state: InputState,
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

#[derive(Component)]
struct StructureActor {
    id: String,
}

#[derive(Component)]
struct ProjectileActor {
    id: String,
}

#[derive(Component)]
struct PredictedProjectileActor {
    client_projectile_id: String,
}

#[derive(Component)]
struct PredictedProjectileVelocity(Vec2);

#[derive(Component)]
struct PredictedProjectileLifetime(f32);

#[derive(Component)]
struct PredictedProjectileTarget {
    has_target: bool,
    position: Vec2,
}

#[derive(Resource, Default)]
struct CurrentPlayerId(Option<String>);

#[derive(Resource, Default)]
struct SimAccumulator(f32);

#[derive(Resource)]
struct NextInputSeq(u32);

impl Default for NextInputSeq {
    fn default() -> Self {
        Self(1)
    }
}

#[derive(Resource, Default)]
struct InputHistory(VecDeque<InputHistoryEntry>);

#[wasm_bindgen]
pub fn boot_game(canvas_id: String) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let mut started = STARTED
        .lock()
        .map_err(|_| JsValue::from_str("mutex poisoned"))?;
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
        .insert_resource(SimAccumulator::default())
        .insert_resource(NextInputSeq::default())
        .insert_resource(InputHistory::default())
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(primary_window),
            ..default()
        }))
        .add_systems(Startup, setup_world)
        .add_systems(
            Update,
            (
                sync_player_id,
                simulate_local_player,
                emit_projectile_fire_command,
                simulate_predicted_projectiles,
                apply_latest_snapshot,
                smooth_remote_motion,
                follow_camera,
            )
                .chain(),
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
    if queue.len() > 12 {
        let overflow = queue.len() - 12;
        queue.drain(0..overflow);
    }

    Ok(())
}

#[wasm_bindgen]
pub fn drain_input_events() -> String {
    let mut queue = match OUTBOUND_INPUTS.lock() {
        Ok(queue) => queue,
        Err(_) => return "[]".to_string(),
    };

    if queue.is_empty() {
        return "[]".to_string();
    }

    let drained: Vec<InputCommand> = queue.drain(..).collect();
    serde_json::to_string(&drained).unwrap_or_else(|_| "[]".to_string())
}

#[wasm_bindgen]
pub fn drain_feature_commands() -> String {
    let mut queue = match OUTBOUND_FEATURE_COMMANDS.lock() {
        Ok(queue) => queue,
        Err(_) => return "[]".to_string(),
    };

    if queue.is_empty() {
        return "[]".to_string();
    }

    let drained: Vec<OutboundFeatureCommand> = queue.drain(..).collect();
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

fn sample_input_state(input: &ButtonInput<KeyCode>) -> InputState {
    InputState {
        up: input.pressed(KeyCode::KeyW) || input.pressed(KeyCode::ArrowUp),
        down: input.pressed(KeyCode::KeyS) || input.pressed(KeyCode::ArrowDown),
        left: input.pressed(KeyCode::KeyA) || input.pressed(KeyCode::ArrowLeft),
        right: input.pressed(KeyCode::KeyD) || input.pressed(KeyCode::ArrowRight),
    }
}

fn to_core_input(input: &InputState) -> CoreInputState {
    CoreInputState {
        up: input.up,
        down: input.down,
        left: input.left,
        right: input.right,
    }
}

fn movement_direction_from_input(input: &ButtonInput<KeyCode>) -> Vec2 {
    let mut dx = 0.0;
    let mut dy = 0.0;

    if input.pressed(KeyCode::KeyD) || input.pressed(KeyCode::ArrowRight) {
        dx += 1.0;
    }
    if input.pressed(KeyCode::KeyA) || input.pressed(KeyCode::ArrowLeft) {
        dx -= 1.0;
    }
    if input.pressed(KeyCode::KeyW) || input.pressed(KeyCode::ArrowUp) {
        dy += 1.0;
    }
    if input.pressed(KeyCode::KeyS) || input.pressed(KeyCode::ArrowDown) {
        dy -= 1.0;
    }

    let direction = Vec2::new(dx, dy);
    if direction.length_squared() <= f32::EPSILON {
        Vec2::X
    } else {
        direction.normalize()
    }
}

fn queue_feature_command(feature: &str, action: &str, payload: Value) {
    if let Ok(mut queue) = OUTBOUND_FEATURE_COMMANDS.lock() {
        queue.push(OutboundFeatureCommand {
            feature: feature.to_string(),
            action: action.to_string(),
            payload,
        });

        if queue.len() > MAX_OUTBOUND_FEATURE_COMMANDS {
            let overflow = queue.len() - MAX_OUTBOUND_FEATURE_COMMANDS;
            queue.drain(0..overflow);
        }
    }
}

fn simulate_local_player(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut accumulator: ResMut<SimAccumulator>,
    mut next_input_seq: ResMut<NextInputSeq>,
    mut input_history: ResMut<InputHistory>,
    mut local_transform_query: Query<&mut Transform, With<LocalActor>>,
) {
    let Ok(mut transform) = local_transform_query.get_single_mut() else {
        return;
    };

    accumulator.0 += time.delta_seconds();
    let mut steps = 0;

    while accumulator.0 >= CLIENT_SIM_DT && steps < MAX_SIM_STEPS_PER_FRAME {
        accumulator.0 -= CLIENT_SIM_DT;
        steps += 1;

        let state = sample_input_state(&input);
        let step = movement_step(
            transform.translation.x,
            transform.translation.y,
            to_core_input(&state),
            CLIENT_SIM_DT,
            MOVE_SPEED,
            MAP_LIMIT,
        );
        transform.translation.x = step.x;
        transform.translation.y = step.y;

        let seq = next_input_seq.0;
        next_input_seq.0 = next_input_seq.0.saturating_add(1);

        input_history.0.push_back(InputHistoryEntry {
            seq,
            state: state.clone(),
        });

        if input_history.0.len() > MAX_INPUT_HISTORY {
            let overflow = input_history.0.len() - MAX_INPUT_HISTORY;
            for _ in 0..overflow {
                input_history.0.pop_front();
            }
        }

        if let Ok(mut outbound) = OUTBOUND_INPUTS.lock() {
            outbound.push(InputCommand {
                seq,
                up: state.up,
                down: state.down,
                left: state.left,
                right: state.right,
            });

            if outbound.len() > MAX_OUTBOUND_INPUTS {
                let overflow = outbound.len() - MAX_OUTBOUND_INPUTS;
                outbound.drain(0..overflow);
            }
        }
    }

    if steps == MAX_SIM_STEPS_PER_FRAME && accumulator.0 >= CLIENT_SIM_DT {
        accumulator.0 = 0.0;
    }
}

fn emit_projectile_fire_command(
    mut commands: Commands,
    input: Res<ButtonInput<KeyCode>>,
    local_transform_query: Query<&Transform, With<LocalActor>>,
) {
    if !input.just_pressed(KeyCode::Space) {
        return;
    }

    let Ok(local_transform) = local_transform_query.get_single() else {
        return;
    };

    let direction = movement_direction_from_input(&input);
    let velocity = direction * PROJECTILE_SPEED;
    let client_projectile_id = format!("proj_{}", Uuid::new_v4());

    queue_feature_command(
        "projectile",
        "fire",
        json!({
            "x": local_transform.translation.x,
            "y": local_transform.translation.y,
            "vx": velocity.x,
            "vy": velocity.y,
            "clientProjectileId": client_projectile_id,
        }),
    );

    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: Color::srgb_u8(255, 214, 98),
                custom_size: Some(Vec2::splat(PROJECTILE_SIZE)),
                ..default()
            },
            transform: Transform::from_xyz(
                local_transform.translation.x,
                local_transform.translation.y,
                PROJECTILE_Z + 0.2,
            ),
            ..default()
        },
        PredictedProjectileActor {
            client_projectile_id,
        },
        PredictedProjectileVelocity(velocity),
        PredictedProjectileLifetime(PROJECTILE_TTL_SECONDS),
        PredictedProjectileTarget {
            has_target: false,
            position: Vec2::ZERO,
        },
    ));
}

fn simulate_predicted_projectiles(
    time: Res<Time>,
    mut commands: Commands,
    mut predicted_query: Query<
        (
            Entity,
            &mut Transform,
            &PredictedProjectileVelocity,
            &mut PredictedProjectileLifetime,
            &PredictedProjectileTarget,
        ),
        With<PredictedProjectileActor>,
    >,
) {
    let dt = time.delta_seconds();
    for (entity, mut transform, velocity, mut ttl, target) in &mut predicted_query {
        transform.translation.x =
            (transform.translation.x + velocity.0.x * dt).clamp(-MAP_LIMIT, MAP_LIMIT);
        transform.translation.y =
            (transform.translation.y + velocity.0.y * dt).clamp(-MAP_LIMIT, MAP_LIMIT);

        let current = transform.translation.truncate();
        let error = target.position - current;
        if target.has_target {
            if error.length() > PROJECTILE_RECONCILE_HARD_SNAP_DISTANCE {
                transform.translation.x = target.position.x;
                transform.translation.y = target.position.y;
            } else {
                let blend = 1.0 - (-PROJECTILE_RECONCILE_BLEND_RATE * dt).exp();
                let corrected = current.lerp(target.position, blend.clamp(0.0, 1.0));
                transform.translation.x = corrected.x;
                transform.translation.y = corrected.y;
            }
        }

        ttl.0 -= dt;
        if ttl.0 <= 0.0 {
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn apply_latest_snapshot(
    mut commands: Commands,
    current_player_id: Res<CurrentPlayerId>,
    mut input_history: ResMut<InputHistory>,
    mut local_query: Query<(&mut Transform, &mut Actor), (With<LocalActor>, Without<RemoteActor>)>,
    remote_query: Query<(Entity, &Actor), (With<RemoteActor>, Without<LocalActor>)>,
    structure_query: Query<(Entity, &StructureActor)>,
    projectile_query: Query<(Entity, &ProjectileActor)>,
    predicted_projectile_query: Query<(Entity, &PredictedProjectileActor)>,
    mut predicted_target_query: Query<&mut PredictedProjectileTarget>,
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
            if let Ok((mut local_transform, mut local_actor)) = local_query.get_single_mut() {
                local_actor.id = player.id.clone();
                reconcile_local_transform(
                    &mut local_transform,
                    Vec2::new(player.x, player.y),
                    snapshot.local_ack_seq,
                    &mut input_history,
                );
            }

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

    let mut structure_entities: HashMap<String, Entity> = structure_query
        .iter()
        .map(|(entity, structure)| (structure.id.clone(), entity))
        .collect();

    for structure in snapshot.structures {
        if let Some(entity) = structure_entities.remove(&structure.id) {
            commands.entity(entity).insert(Transform::from_xyz(
                structure.x,
                structure.y,
                STRUCTURE_Z,
            ));
        } else {
            spawn_structure_actor(&mut commands, &structure);
        }
    }

    for entity in structure_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }

    let mut projectile_entities: HashMap<String, Entity> = projectile_query
        .iter()
        .map(|(entity, projectile)| (projectile.id.clone(), entity))
        .collect();
    let mut predicted_projectile_entities: HashMap<String, Entity> = predicted_projectile_query
        .iter()
        .map(|(entity, predicted)| (predicted.client_projectile_id.clone(), entity))
        .collect();
    let local_player_id = current_player_id.0.clone();

    for projectile in snapshot.projectiles {
        if local_player_id
            .as_deref()
            .is_some_and(|player_id| player_id == projectile.owner_id)
        {
            if let Some(client_projectile_id) = projectile.client_projectile_id.as_deref() {
                if let Some(predicted_entity) =
                    predicted_projectile_entities.remove(client_projectile_id)
                {
                    if let Ok(mut target) = predicted_target_query.get_mut(predicted_entity) {
                        let projected_x =
                            projectile.x + projectile.vx * (snapshot.render_delay_ms / 1000.0);
                        let projected_y =
                            projectile.y + projectile.vy * (snapshot.render_delay_ms / 1000.0);
                        target.has_target = true;
                        target.position = Vec2::new(projected_x, projected_y);
                    }

                    if let Some(authoritative_entity) = projectile_entities.remove(&projectile.id) {
                        commands.entity(authoritative_entity).despawn_recursive();
                    }
                    continue;
                }
            }
        }

        if let Some(entity) = projectile_entities.remove(&projectile.id) {
            commands.entity(entity).insert(Transform::from_xyz(
                projectile.x,
                projectile.y,
                PROJECTILE_Z,
            ));
        } else {
            spawn_projectile_actor(&mut commands, &projectile);
        }
    }

    for entity in projectile_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }
}

fn reconcile_local_transform(
    local_transform: &mut Transform,
    authoritative_position: Vec2,
    local_ack_seq: u32,
    input_history: &mut InputHistory,
) {
    while input_history
        .0
        .front()
        .is_some_and(|entry| entry.seq <= local_ack_seq)
    {
        input_history.0.pop_front();
    }

    let mut replay_position = authoritative_position;
    for entry in input_history.0.iter() {
        let step = movement_step(
            replay_position.x,
            replay_position.y,
            to_core_input(&entry.state),
            CLIENT_SIM_DT,
            MOVE_SPEED,
            MAP_LIMIT,
        );
        replay_position.x = step.x;
        replay_position.y = step.y;
    }

    local_transform.translation.x = replay_position.x;
    local_transform.translation.y = replay_position.y;
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

fn spawn_structure_actor(commands: &mut Commands, structure: &StructureState) {
    let color = match structure.kind.as_str() {
        "beacon" => Color::srgb_u8(99, 210, 255),
        "miner" => Color::srgb_u8(167, 139, 250),
        "assembler" => Color::srgb_u8(74, 222, 128),
        _ => Color::srgb_u8(255, 255, 255),
    };

    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color,
                custom_size: Some(Vec2::splat(STRUCTURE_SIZE)),
                ..default()
            },
            transform: Transform::from_xyz(structure.x, structure.y, STRUCTURE_Z),
            ..default()
        },
        StructureActor {
            id: structure.id.clone(),
        },
    ));
}

fn spawn_projectile_actor(commands: &mut Commands, projectile: &ProjectileState) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: Color::srgb_u8(248, 250, 134),
                custom_size: Some(Vec2::splat(PROJECTILE_SIZE)),
                ..default()
            },
            transform: Transform::from_xyz(projectile.x, projectile.y, PROJECTILE_Z),
            ..default()
        },
        ProjectileActor {
            id: projectile.id.clone(),
        },
    ));
}

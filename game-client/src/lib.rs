use bevy::prelude::*;
use bevy::render::texture::ImagePlugin;
use bevy::window::{PrimaryWindow, WindowResolution};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sim_core::{
    movement_step_with_obstacles, InputState as CoreInputState, StructureObstacle,
    PLAYER_COLLIDER_RADIUS, STRUCTURE_COLLIDER_HALF_EXTENT,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use uuid::Uuid;
use wasm_bindgen::prelude::*;

const TILE_SIZE: f32 = 32.0;
const CHARACTER_FRAME_SIZE: f32 = 48.0;
const CHARACTER_ANIMATION_FPS: f32 = 12.0;
const CHARACTER_ANIMATION_FRAMES: usize = 4;
const CHARACTER_SCALE: f32 = 1.75;
const FOOTSTEP_TRIGGER_SPEED: f32 = 18.0;
const FOOTSTEP_INTERVAL_SECONDS: f32 = 0.30;
const FOOTSTEP_BASE_VOLUME: f32 = 0.52;
const PLACEMENT_VOLUME: f32 = 0.55;
const FOOTSTEP_VOLUME_VARIATION: [f32; 6] = [0.88, 1.0, 0.94, 1.06, 0.9, 1.02];
const FOOTSTEP_SPEED_VARIATION: [f32; 6] = [0.96, 1.03, 0.99, 1.05, 0.97, 1.01];
const STRUCTURE_SIZE: f32 = 18.0;
const PROJECTILE_SIZE: f32 = 8.0;
const MAP_LIMIT: f32 = 5000.0;
const BUILD_GRID_SIZE: f32 = 32.0;
const BUILD_PREVIEW_SEND_INTERVAL_SECONDS: f32 = 0.08;
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
const CHARACTER_DIRECTION_EPSILON: f32 = 0.001;
const SNAPSHOT_Z: f32 = 4.0;
const STRUCTURE_Z: f32 = 3.0;
const PROJECTILE_Z: f32 = 5.0;
const FLOOR_Z: f32 = 0.0;
const BUILD_PREVIEW_Z: f32 = 3.6;

static INBOUND_SNAPSHOTS: Lazy<Mutex<Vec<SnapshotPayload>>> = Lazy::new(|| Mutex::new(Vec::new()));
static OUTBOUND_INPUTS: Lazy<Mutex<Vec<InputCommand>>> = Lazy::new(|| Mutex::new(Vec::new()));
static OUTBOUND_FEATURE_COMMANDS: Lazy<Mutex<Vec<OutboundFeatureCommand>>> =
    Lazy::new(|| Mutex::new(Vec::new()));
static NEXT_PLAYER_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));
static PENDING_SESSION_RESET: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

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
struct BuildPreviewState {
    #[serde(rename = "playerId")]
    player_id: String,
    x: f32,
    y: f32,
    kind: String,
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
    previews: Vec<BuildPreviewState>,
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

#[derive(Component, Default)]
struct ActorVelocity(Vec2);

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
struct BuildPreviewActor {
    player_id: String,
}

#[derive(Component)]
struct LocalBuildGhost;

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

#[derive(Clone, Copy)]
enum FacingDirection {
    Down,
    Left,
    Right,
    Up,
}

impl FacingDirection {
    fn row_index(self) -> usize {
        match self {
            // This sheet's row order is: up(back), right, down(front), left.
            FacingDirection::Up => 0,
            FacingDirection::Right => 1,
            FacingDirection::Down => 2,
            FacingDirection::Left => 3,
        }
    }
}

#[derive(Component)]
struct CharacterAnimator {
    facing: FacingDirection,
    frame: usize,
    timer: Timer,
}

impl Default for CharacterAnimator {
    fn default() -> Self {
        Self {
            facing: FacingDirection::Down,
            frame: 0,
            timer: Timer::from_seconds(1.0 / CHARACTER_ANIMATION_FPS, TimerMode::Repeating),
        }
    }
}

#[derive(Resource, Clone)]
struct CharacterAtlasHandles {
    texture: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
}

#[derive(Resource, Clone)]
struct SfxAudioHandles {
    footstep_clips: Vec<Handle<AudioSource>>,
    placement_clip: Handle<AudioSource>,
}

#[derive(Resource)]
struct FootstepState {
    timer: Timer,
    was_moving: bool,
    clip_cursor: usize,
    variation_cursor: usize,
}

impl Default for FootstepState {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(FOOTSTEP_INTERVAL_SECONDS, TimerMode::Repeating),
            was_moving: false,
            clip_cursor: 0,
            variation_cursor: 0,
        }
    }
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

#[derive(Resource)]
struct BuildPlacementState {
    active: bool,
    kind: &'static str,
    last_sent_cell: Option<IVec2>,
    send_cooldown: f32,
}

impl Default for BuildPlacementState {
    fn default() -> Self {
        Self {
            active: false,
            kind: "beacon",
            last_sent_cell: None,
            send_cooldown: 0.0,
        }
    }
}

fn clear_protocol_queues() {
    if let Ok(mut queue) = INBOUND_SNAPSHOTS.lock() {
        queue.clear();
    }
    if let Ok(mut queue) = OUTBOUND_INPUTS.lock() {
        queue.clear();
    }
    if let Ok(mut queue) = OUTBOUND_FEATURE_COMMANDS.lock() {
        queue.clear();
    }
}

fn take_pending_session_reset() -> bool {
    match PENDING_SESSION_RESET.lock() {
        Ok(mut pending) => {
            let was_pending = *pending;
            *pending = false;
            was_pending
        }
        Err(_) => false,
    }
}

#[wasm_bindgen]
pub fn boot_game(canvas_id: String) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    clear_protocol_queues();

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
        .insert_resource(BuildPlacementState::default())
        .insert_resource(FootstepState::default())
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: "/".to_string(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(primary_window),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .add_systems(Startup, setup_world)
        .add_systems(
            Update,
            (
                apply_pending_session_reset,
                sync_player_id,
                simulate_local_player,
                emit_footstep_audio,
                handle_build_placement_controls,
                emit_projectile_fire_command,
                simulate_predicted_projectiles,
                apply_latest_snapshot,
                smooth_remote_motion,
                animate_character_sprites,
                follow_camera,
            )
                .chain(),
        );

    app.run();
    Ok(())
}

#[wasm_bindgen]
pub fn reset_session_state() {
    clear_protocol_queues();
    if let Ok(mut pending) = PENDING_SESSION_RESET.lock() {
        *pending = true;
    }
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

fn setup_world(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut atlas_layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    commands.insert_resource(SfxAudioHandles {
        footstep_clips: vec![
            asset_server.load("audio/footstep-a.mp3"),
            asset_server.load("audio/footstep-b.mp3"),
        ],
        placement_clip: asset_server.load("audio/place-object-subtle.mp3"),
    });

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

    let texture = asset_server.load("sprites/factorio-character-sheet.png");
    let layout = atlas_layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(CHARACTER_FRAME_SIZE as u32),
        CHARACTER_ANIMATION_FRAMES as u32,
        4,
        None,
        None,
    ));

    commands.insert_resource(CharacterAtlasHandles {
        texture: texture.clone(),
        layout: layout.clone(),
    });

    commands.spawn((
        SpriteBundle {
            texture,
            transform: Transform::from_xyz(0.0, 0.0, SNAPSHOT_Z)
                .with_scale(Vec3::splat(CHARACTER_SCALE)),
            ..default()
        },
        TextureAtlas { layout, index: 0 },
        Actor {
            id: "local-pending".to_string(),
        },
        ActorVelocity::default(),
        CharacterAnimator::default(),
        LocalActor,
    ));

    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: structure_preview_color("beacon", true),
                custom_size: Some(Vec2::splat(STRUCTURE_SIZE)),
                ..default()
            },
            transform: Transform::from_xyz(0.0, 0.0, BUILD_PREVIEW_Z),
            visibility: Visibility::Hidden,
            ..default()
        },
        LocalBuildGhost,
    ));
}

fn apply_pending_session_reset(
    mut commands: Commands,
    mut current_player_id: ResMut<CurrentPlayerId>,
    mut accumulator: ResMut<SimAccumulator>,
    mut next_input_seq: ResMut<NextInputSeq>,
    mut input_history: ResMut<InputHistory>,
    mut placement: ResMut<BuildPlacementState>,
    mut footstep_state: ResMut<FootstepState>,
    mut local_query: Query<
        (
            &mut Transform,
            &mut Actor,
            &mut ActorVelocity,
            &mut CharacterAnimator,
            &mut TextureAtlas,
        ),
        (With<LocalActor>, Without<LocalBuildGhost>),
    >,
    mut local_build_ghost_query: Query<
        (&mut Visibility, &mut Transform),
        (With<LocalBuildGhost>, Without<LocalActor>),
    >,
    remote_query: Query<Entity, With<RemoteActor>>,
    structure_query: Query<Entity, With<StructureActor>>,
    preview_query: Query<Entity, With<BuildPreviewActor>>,
    projectile_query: Query<Entity, With<ProjectileActor>>,
    predicted_projectile_query: Query<Entity, With<PredictedProjectileActor>>,
) {
    if !take_pending_session_reset() {
        return;
    }

    clear_protocol_queues();

    current_player_id.0 = None;
    accumulator.0 = 0.0;
    next_input_seq.0 = 1;
    input_history.0.clear();
    *placement = BuildPlacementState::default();
    *footstep_state = FootstepState::default();

    if let Ok((mut transform, mut actor, mut velocity, mut animator, mut atlas)) =
        local_query.get_single_mut()
    {
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
        velocity.0 = Vec2::ZERO;
        actor.id = "local-pending".to_string();
        animator.facing = FacingDirection::Down;
        animator.frame = 0;
        animator.timer.reset();
        atlas.index = FacingDirection::Down.row_index() * CHARACTER_ANIMATION_FRAMES;
    }

    if let Ok((mut visibility, mut transform)) = local_build_ghost_query.get_single_mut() {
        *visibility = Visibility::Hidden;
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
    }

    for entity in &remote_query {
        commands.entity(entity).despawn_recursive();
    }
    for entity in &structure_query {
        commands.entity(entity).despawn_recursive();
    }
    for entity in &preview_query {
        commands.entity(entity).despawn_recursive();
    }
    for entity in &projectile_query {
        commands.entity(entity).despawn_recursive();
    }
    for entity in &predicted_projectile_query {
        commands.entity(entity).despawn_recursive();
    }
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

fn snap_world_to_build_grid(world: Vec2) -> (IVec2, Vec2) {
    let grid_x = (world.x / BUILD_GRID_SIZE).round() as i32;
    let grid_y = (world.y / BUILD_GRID_SIZE).round() as i32;
    let snapped = Vec2::new(
        (grid_x as f32 * BUILD_GRID_SIZE).clamp(-MAP_LIMIT, MAP_LIMIT),
        (grid_y as f32 * BUILD_GRID_SIZE).clamp(-MAP_LIMIT, MAP_LIMIT),
    );
    (IVec2::new(grid_x, grid_y), snapped)
}

fn set_local_build_ghost_visible(
    ghost_query: &mut Query<(&mut Transform, &mut Visibility, &mut Sprite), With<LocalBuildGhost>>,
    visible: bool,
    position: Vec2,
    kind: &str,
) {
    if let Ok((mut transform, mut visibility, mut sprite)) = ghost_query.get_single_mut() {
        transform.translation.x = position.x;
        transform.translation.y = position.y;
        *visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        sprite.color = structure_preview_color(kind, true);
    }
}

fn disable_build_mode(placement: &mut BuildPlacementState) {
    placement.active = false;
    placement.last_sent_cell = None;
    placement.send_cooldown = 0.0;
    queue_feature_command(
        "build",
        "preview",
        json!({
            "active": false,
        }),
    );
}

fn handle_build_placement_controls(
    mut commands: Commands,
    time: Res<Time>,
    input: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    sfx_handles: Res<SfxAudioHandles>,
    mut placement: ResMut<BuildPlacementState>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut ghost_query: Query<(&mut Transform, &mut Visibility, &mut Sprite), With<LocalBuildGhost>>,
) {
    if input.just_pressed(KeyCode::KeyQ) {
        if placement.active {
            disable_build_mode(&mut placement);
            set_local_build_ghost_visible(&mut ghost_query, false, Vec2::ZERO, placement.kind);
            return;
        }

        placement.active = true;
        placement.last_sent_cell = None;
        placement.send_cooldown = BUILD_PREVIEW_SEND_INTERVAL_SECONDS;
    }

    if input.just_pressed(KeyCode::Escape) && placement.active {
        disable_build_mode(&mut placement);
        set_local_build_ghost_visible(&mut ghost_query, false, Vec2::ZERO, placement.kind);
        return;
    }

    if !placement.active {
        set_local_build_ghost_visible(&mut ghost_query, false, Vec2::ZERO, placement.kind);
        return;
    }

    let Ok(window) = window_query.get_single() else {
        set_local_build_ghost_visible(&mut ghost_query, false, Vec2::ZERO, placement.kind);
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        set_local_build_ghost_visible(&mut ghost_query, false, Vec2::ZERO, placement.kind);
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.get_single() else {
        set_local_build_ghost_visible(&mut ghost_query, false, Vec2::ZERO, placement.kind);
        return;
    };
    let Some(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else {
        set_local_build_ghost_visible(&mut ghost_query, false, Vec2::ZERO, placement.kind);
        return;
    };

    let (cell, snapped) = snap_world_to_build_grid(world_pos);
    set_local_build_ghost_visible(&mut ghost_query, true, snapped, placement.kind);

    placement.send_cooldown += time.delta_seconds();
    let cell_changed = placement.last_sent_cell != Some(cell);
    if cell_changed || placement.send_cooldown >= BUILD_PREVIEW_SEND_INTERVAL_SECONDS {
        placement.last_sent_cell = Some(cell);
        placement.send_cooldown = 0.0;
        queue_feature_command(
            "build",
            "preview",
            json!({
                "active": true,
                "x": snapped.x,
                "y": snapped.y,
                "kind": placement.kind,
            }),
        );
    }

    if mouse_buttons.just_pressed(MouseButton::Left) {
        commands.spawn(AudioBundle {
            source: sfx_handles.placement_clip.clone(),
            settings: bevy::audio::PlaybackSettings::DESPAWN
                .with_volume(bevy::audio::Volume::new(PLACEMENT_VOLUME)),
            ..default()
        });

        queue_feature_command(
            "build",
            "place",
            json!({
                "x": snapped.x,
                "y": snapped.y,
                "kind": placement.kind,
                "clientBuildId": format!("build_{}", Uuid::new_v4()),
            }),
        );
    }
}

fn simulate_local_player(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut accumulator: ResMut<SimAccumulator>,
    mut next_input_seq: ResMut<NextInputSeq>,
    mut input_history: ResMut<InputHistory>,
    structure_query: Query<&Transform, (With<StructureActor>, Without<LocalActor>)>,
    mut local_transform_query: Query<
        (&mut Transform, &mut ActorVelocity),
        (With<LocalActor>, Without<StructureActor>),
    >,
) {
    let Ok((mut transform, mut velocity)) = local_transform_query.get_single_mut() else {
        return;
    };

    accumulator.0 += time.delta_seconds();
    let mut steps = 0;
    let structure_obstacles: Vec<StructureObstacle> = structure_query
        .iter()
        .map(|structure| StructureObstacle {
            x: structure.translation.x,
            y: structure.translation.y,
            half_extent: STRUCTURE_COLLIDER_HALF_EXTENT,
        })
        .collect();

    while accumulator.0 >= CLIENT_SIM_DT && steps < MAX_SIM_STEPS_PER_FRAME {
        accumulator.0 -= CLIENT_SIM_DT;
        steps += 1;

        let state = sample_input_state(&input);
        let step = movement_step_with_obstacles(
            transform.translation.x,
            transform.translation.y,
            to_core_input(&state),
            CLIENT_SIM_DT,
            MOVE_SPEED,
            MAP_LIMIT,
            &structure_obstacles,
            PLAYER_COLLIDER_RADIUS,
        );
        transform.translation.x = step.x;
        transform.translation.y = step.y;
        velocity.0 = Vec2::new(step.vx, step.vy);

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

fn emit_footstep_audio(
    mut commands: Commands,
    time: Res<Time>,
    sfx_handles: Res<SfxAudioHandles>,
    mut footstep_state: ResMut<FootstepState>,
    local_velocity_query: Query<&ActorVelocity, With<LocalActor>>,
) {
    let Ok(local_velocity) = local_velocity_query.get_single() else {
        footstep_state.was_moving = false;
        footstep_state.timer.reset();
        return;
    };

    let moving =
        local_velocity.0.length_squared() >= FOOTSTEP_TRIGGER_SPEED * FOOTSTEP_TRIGGER_SPEED;
    if !moving {
        footstep_state.was_moving = false;
        footstep_state.timer.reset();
        return;
    }
    if sfx_handles.footstep_clips.is_empty() {
        return;
    }

    let play_footstep =
        |commands: &mut Commands, handles: &SfxAudioHandles, state: &mut FootstepState| {
            let clip_index = state.clip_cursor % handles.footstep_clips.len();
            let variation_index = state.variation_cursor % FOOTSTEP_VOLUME_VARIATION.len();
            let clip = handles.footstep_clips[clip_index].clone();
            let volume = FOOTSTEP_BASE_VOLUME * FOOTSTEP_VOLUME_VARIATION[variation_index];
            let speed = FOOTSTEP_SPEED_VARIATION[variation_index % FOOTSTEP_SPEED_VARIATION.len()];

            commands.spawn(AudioBundle {
                source: clip,
                settings: bevy::audio::PlaybackSettings::DESPAWN
                    .with_volume(bevy::audio::Volume::new(volume))
                    .with_speed(speed),
                ..default()
            });

            state.clip_cursor = (state.clip_cursor + 1) % handles.footstep_clips.len();
            state.variation_cursor = (state.variation_cursor + 1) % FOOTSTEP_VOLUME_VARIATION.len();
        };

    if !footstep_state.was_moving {
        footstep_state.was_moving = true;
        footstep_state.timer.reset();
        play_footstep(&mut commands, &sfx_handles, &mut footstep_state);
        return;
    }

    footstep_state.timer.tick(time.delta());
    if footstep_state.timer.just_finished() {
        play_footstep(&mut commands, &sfx_handles, &mut footstep_state);
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
    character_atlas: Res<CharacterAtlasHandles>,
    current_player_id: Res<CurrentPlayerId>,
    mut input_history: ResMut<InputHistory>,
    mut local_query: Query<(&mut Transform, &mut Actor), (With<LocalActor>, Without<RemoteActor>)>,
    remote_query: Query<(Entity, &Actor), (With<RemoteActor>, Without<LocalActor>)>,
    structure_query: Query<(Entity, &StructureActor)>,
    preview_query: Query<(Entity, &BuildPreviewActor)>,
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
    let SnapshotPayload {
        local_ack_seq,
        render_delay_ms,
        players,
        structures,
        previews,
        projectiles,
        ..
    } = snapshot;
    let structure_obstacles: Vec<StructureObstacle> = structures
        .iter()
        .map(|structure| StructureObstacle {
            x: structure.x,
            y: structure.y,
            half_extent: STRUCTURE_COLLIDER_HALF_EXTENT,
        })
        .collect();
    let local_player_id = current_player_id.0.clone();

    let mut remote_entities: HashMap<String, Entity> = remote_query
        .iter()
        .map(|(entity, actor)| (actor.id.clone(), entity))
        .collect();

    for player in players.into_iter().filter(|state| state.connected) {
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
                    local_ack_seq,
                    &mut input_history,
                    &structure_obstacles,
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
                .insert(RemoteTarget(Vec2::new(player.x, player.y)))
                .insert(ActorVelocity(Vec2::new(player.vx, player.vy)));
        } else {
            spawn_remote_actor(&mut commands, &player, &character_atlas);
        }
    }

    for entity in remote_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }

    let mut structure_entities: HashMap<String, Entity> = structure_query
        .iter()
        .map(|(entity, structure)| (structure.id.clone(), entity))
        .collect();

    for structure in structures {
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

    let mut preview_entities: HashMap<String, Entity> = preview_query
        .iter()
        .map(|(entity, preview)| (preview.player_id.clone(), entity))
        .collect();

    for preview in previews {
        if local_player_id
            .as_deref()
            .is_some_and(|player_id| player_id == preview.player_id)
        {
            if let Some(entity) = preview_entities.remove(&preview.player_id) {
                commands.entity(entity).despawn_recursive();
            }
            continue;
        }

        if let Some(entity) = preview_entities.remove(&preview.player_id) {
            commands.entity(entity).insert(Transform::from_xyz(
                preview.x,
                preview.y,
                BUILD_PREVIEW_Z,
            ));
        } else {
            spawn_build_preview_actor(&mut commands, &preview);
        }
    }

    for entity in preview_entities.values() {
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
    for projectile in projectiles {
        if local_player_id
            .as_deref()
            .is_some_and(|player_id| player_id == projectile.owner_id)
        {
            if let Some(client_projectile_id) = projectile.client_projectile_id.as_deref() {
                if let Some(predicted_entity) =
                    predicted_projectile_entities.remove(client_projectile_id)
                {
                    if let Ok(mut target) = predicted_target_query.get_mut(predicted_entity) {
                        let projected_x = projectile.x + projectile.vx * (render_delay_ms / 1000.0);
                        let projected_y = projectile.y + projectile.vy * (render_delay_ms / 1000.0);
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
    structure_obstacles: &[StructureObstacle],
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
        let step = movement_step_with_obstacles(
            replay_position.x,
            replay_position.y,
            to_core_input(&entry.state),
            CLIENT_SIM_DT,
            MOVE_SPEED,
            MAP_LIMIT,
            structure_obstacles,
            PLAYER_COLLIDER_RADIUS,
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

fn facing_from_velocity(velocity: Vec2, fallback: FacingDirection) -> FacingDirection {
    if velocity.length_squared() < CHARACTER_DIRECTION_EPSILON {
        return fallback;
    }

    if velocity.y.abs() > velocity.x.abs() {
        if velocity.y >= 0.0 {
            FacingDirection::Up
        } else {
            FacingDirection::Down
        }
    } else if velocity.x >= 0.0 {
        FacingDirection::Right
    } else {
        FacingDirection::Left
    }
}

fn animate_character_sprites(
    time: Res<Time>,
    mut query: Query<(&ActorVelocity, &mut CharacterAnimator, &mut TextureAtlas), With<Actor>>,
) {
    for (velocity, mut animator, mut atlas) in &mut query {
        animator.facing = facing_from_velocity(velocity.0, animator.facing);
        let is_moving = velocity.0.length_squared() > CHARACTER_DIRECTION_EPSILON;

        if is_moving {
            animator.timer.tick(time.delta());
            if animator.timer.just_finished() {
                animator.frame = (animator.frame + 1) % CHARACTER_ANIMATION_FRAMES;
            }
        } else {
            animator.frame = 0;
            animator.timer.reset();
        }

        atlas.index = animator.facing.row_index() * CHARACTER_ANIMATION_FRAMES + animator.frame;
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

fn spawn_remote_actor(
    commands: &mut Commands,
    player: &PlayerState,
    character_atlas: &CharacterAtlasHandles,
) {
    commands.spawn((
        SpriteBundle {
            texture: character_atlas.texture.clone(),
            transform: Transform::from_xyz(player.x, player.y, SNAPSHOT_Z)
                .with_scale(Vec3::splat(CHARACTER_SCALE)),
            ..default()
        },
        TextureAtlas {
            layout: character_atlas.layout.clone(),
            index: 0,
        },
        Actor {
            id: player.id.clone(),
        },
        ActorVelocity(Vec2::new(player.vx, player.vy)),
        CharacterAnimator::default(),
        RemoteActor,
        RemoteTarget(Vec2::new(player.x, player.y)),
    ));
}

fn structure_color(kind: &str) -> Color {
    match kind {
        "beacon" => Color::srgb_u8(99, 210, 255),
        "miner" => Color::srgb_u8(167, 139, 250),
        "assembler" => Color::srgb_u8(74, 222, 128),
        _ => Color::srgb_u8(255, 255, 255),
    }
}

fn structure_preview_color(kind: &str, is_local: bool) -> Color {
    let base = structure_color(kind).to_srgba();
    let alpha = if is_local { 0.55 } else { 0.35 };
    Color::srgba(base.red, base.green, base.blue, alpha)
}

fn spawn_structure_actor(commands: &mut Commands, structure: &StructureState) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: structure_color(structure.kind.as_str()),
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

fn spawn_build_preview_actor(commands: &mut Commands, preview: &BuildPreviewState) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: structure_preview_color(preview.kind.as_str(), false),
                custom_size: Some(Vec2::splat(STRUCTURE_SIZE)),
                ..default()
            },
            transform: Transform::from_xyz(preview.x, preview.y, BUILD_PREVIEW_Z),
            ..default()
        },
        BuildPreviewActor {
            player_id: preview.player_id.clone(),
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

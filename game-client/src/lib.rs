use bevy::asset::AssetMetaCheck;
use bevy::prelude::*;
use bevy::render::texture::ImagePlugin;
use bevy::window::{PrimaryWindow, WindowResolution};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sim_core::{
    movement_step_with_obstacles, sample_terrain, InputState as CoreInputState, StructureObstacle,
    TerrainBaseKind, TerrainResourceKind, PLAYER_COLLIDER_RADIUS, STRUCTURE_COLLIDER_HALF_EXTENT,
    TERRAIN_GENERATOR_VERSION, TERRAIN_TILE_SIZE,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use uuid::Uuid;
use wasm_bindgen::prelude::*;

const DEFAULT_TERRAIN_TILE_SIZE: f32 = TERRAIN_TILE_SIZE as f32;
const CHARACTER_FRAME_SIZE: f32 = 48.0;
const CHARACTER_ANIMATION_FPS: f32 = 12.0;
const CHARACTER_ANIMATION_FRAMES: usize = 4;
const CHARACTER_SCALE: f32 = 1.75;
const MAX_NAME_LABEL_LENGTH: usize = 24;
const NAME_LABEL_FONT_SIZE: f32 = 13.0;
const NAME_LABEL_OFFSET_Y: f32 = CHARACTER_FRAME_SIZE * CHARACTER_SCALE * 0.6;
const NAME_LABEL_Z_OFFSET: f32 = 0.15;
const DEFAULT_CHARACTER_SPRITE_ID: &str = "engineer-default";
const CHARACTER_SPRITE_VARIANTS: [(&str, &str); 3] = [
    ("engineer-default", "sprites/character-engineer-default.png"),
    ("surveyor-cyan", "sprites/character-surveyor-cyan.png"),
    ("machinist-rose", "sprites/character-machinist-rose.png"),
];
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
const ENEMY_Z: f32 = 3.25;
const FLOOR_Z: f32 = 0.0;
const TERRAIN_RESOURCE_OVERLAY_Z: f32 = 0.15;
const BUILD_PREVIEW_Z: f32 = 3.6;
const MINING_NODE_Z: f32 = 2.7;
const MINING_PROGRESS_Z: f32 = 2.95;
const MINING_NODE_BASE_SIZE: f32 = 13.0;
const MINING_PROGRESS_BAR_WIDTH: f32 = 24.0;
const MINING_PROGRESS_BAR_HEIGHT: f32 = 4.0;
const MINING_INTERACT_RADIUS: f32 = 20.0;
const DROP_Z: f32 = 2.58;
const DROP_BASE_SIZE: f32 = 10.0;
const ENEMY_BASE_SIZE: f32 = 22.0;
const ENEMY_HEALTH_BAR_HEIGHT: f32 = 3.0;
const ENEMY_HEALTH_BAR_OFFSET_Y: f32 = 5.0;
const ENEMY_HEALTH_BAR_Z: f32 = ENEMY_Z + 0.08;
const COMBAT_POPUP_Z: f32 = ENEMY_Z + 0.25;
const COMBAT_POPUP_TTL_SECONDS: f32 = 0.72;
const COMBAT_POPUP_RISE_SPEED: f32 = 34.0;
const PLAYER_DAMAGE_POPUP_OFFSET_Y: f32 = 42.0;
const DROP_PICKUP_INTERACT_RADIUS: f32 = 84.0;
const TERRAIN_RENDER_RADIUS_TILES: i32 = 24;
const CRAFT_QUEUE_COUNT_PER_PRESS: u32 = 1;
const RECIPE_SMELT_IRON_PLATE: &str = "smelt_iron_plate";
const RECIPE_SMELT_COPPER_PLATE: &str = "smelt_copper_plate";
const RECIPE_CRAFT_GEAR: &str = "craft_gear";

static INBOUND_SNAPSHOTS: Lazy<Mutex<Vec<SnapshotPayload>>> = Lazy::new(|| Mutex::new(Vec::new()));
static OUTBOUND_INPUTS: Lazy<Mutex<Vec<InputCommand>>> = Lazy::new(|| Mutex::new(Vec::new()));
static OUTBOUND_FEATURE_COMMANDS: Lazy<Mutex<Vec<OutboundFeatureCommand>>> =
    Lazy::new(|| Mutex::new(Vec::new()));
static NEXT_PLAYER_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static LAST_LOCAL_COMBAT_HEALTH: Lazy<Mutex<Option<(String, u32)>>> =
    Lazy::new(|| Mutex::new(None));
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
    #[serde(rename = "canPlace", default = "default_preview_can_place")]
    can_place: bool,
    #[serde(default)]
    reason: Option<String>,
}

fn default_preview_can_place() -> bool {
    true
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
#[serde(rename_all = "camelCase")]
struct TerrainSnapshotState {
    seed: String,
    generator_version: u32,
    tile_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MiningNodeSnapshotState {
    id: String,
    kind: String,
    x: f32,
    y: f32,
    remaining: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MiningProgressSnapshotState {
    #[serde(rename = "playerId")]
    player_id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    #[serde(rename = "startedAt")]
    started_at: i64,
    #[serde(rename = "completesAt")]
    completes_at: i64,
    progress: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MiningSnapshotState {
    nodes: Vec<MiningNodeSnapshotState>,
    active: Vec<MiningProgressSnapshotState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DropSnapshotState {
    id: String,
    resource: String,
    amount: u32,
    x: f32,
    y: f32,
    #[serde(rename = "spawnedAt")]
    _spawned_at: i64,
    #[serde(rename = "expiresAt")]
    _expires_at: i64,
    #[serde(rename = "ownerPlayerId")]
    _owner_player_id: Option<String>,
    #[serde(rename = "ownerExpiresAt")]
    _owner_expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DropsSnapshotState {
    drops: Vec<DropSnapshotState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CharacterProfileSnapshotState {
    #[serde(rename = "playerId")]
    player_id: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "spriteId")]
    sprite_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CharacterSnapshotState {
    players: Vec<CharacterProfileSnapshotState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnemySnapshotState {
    id: String,
    kind: String,
    x: f32,
    y: f32,
    health: u32,
    #[serde(rename = "maxHealth")]
    max_health: u32,
    #[serde(rename = "targetPlayerId")]
    target_player_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerCombatSnapshotState {
    #[serde(rename = "playerId")]
    player_id: String,
    health: u32,
    #[serde(rename = "maxHealth")]
    max_health: u32,
    #[serde(rename = "attackPower")]
    _attack_power: u32,
    #[serde(rename = "armor")]
    _armor: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CombatSnapshotState {
    #[serde(default)]
    enemies: Vec<EnemySnapshotState>,
    #[serde(default)]
    players: Vec<PlayerCombatSnapshotState>,
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
    #[serde(default)]
    terrain: Option<TerrainSnapshotState>,
    #[serde(default)]
    mining: Option<MiningSnapshotState>,
    #[serde(default)]
    drops: Option<DropsSnapshotState>,
    #[serde(default)]
    combat: Option<CombatSnapshotState>,
    #[serde(default)]
    character: Option<CharacterSnapshotState>,
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
struct CharacterSpriteId(String);

#[derive(Component)]
struct ActorNameLabelEntity(Entity);

#[derive(Component)]
struct ActorNameLabel;

#[derive(Component)]
struct RemoteTarget(Vec2);

#[derive(Component)]
struct StructureActor {
    id: String,
}

#[derive(Component)]
struct TerrainTileActor {
    grid_x: i32,
    grid_y: i32,
}

#[derive(Component)]
struct TerrainResourceOverlayActor {
    grid_x: i32,
    grid_y: i32,
}

#[derive(Component)]
struct MiningNodeActor {
    id: String,
    kind: String,
    remaining: u32,
    active: bool,
}

#[derive(Component)]
struct MiningProgressActor {
    player_id: String,
    node_id: String,
    progress: f32,
}

#[derive(Component)]
struct DropActor {
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
struct EnemyActor {
    id: String,
    health: u32,
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

#[derive(Component)]
struct EnemyHealthBarActor {
    enemy_id: String,
}

#[derive(Component)]
struct CombatPopupActor {
    ttl: f32,
    max_ttl: f32,
    rise_speed: f32,
    base_color: Color,
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

#[derive(Clone)]
struct CharacterAtlasHandle {
    texture: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
}

#[derive(Resource, Clone)]
struct CharacterAtlasHandles {
    default_sprite_id: String,
    by_sprite_id: HashMap<String, CharacterAtlasHandle>,
}

impl CharacterAtlasHandles {
    fn resolve(&self, sprite_id: &str) -> (CharacterAtlasHandle, String) {
        if let Some(entry) = self.by_sprite_id.get(sprite_id) {
            return (entry.clone(), sprite_id.to_string());
        }

        let fallback_id = self.default_sprite_id.clone();
        let fallback = self
            .by_sprite_id
            .get(fallback_id.as_str())
            .expect("default character sprite atlas missing")
            .clone();
        (fallback, fallback_id)
    }
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

#[derive(Resource, Default)]
struct MiningInteractionState {
    active_node_id: Option<String>,
}

#[derive(Resource)]
struct TerrainRenderState {
    seed: u64,
    tile_size: f32,
    generator_version: u32,
    render_radius_tiles: i32,
    last_center_cell: Option<IVec2>,
    needs_refresh: bool,
}

impl Default for TerrainRenderState {
    fn default() -> Self {
        Self {
            seed: 0,
            tile_size: DEFAULT_TERRAIN_TILE_SIZE,
            generator_version: TERRAIN_GENERATOR_VERSION,
            render_radius_tiles: TERRAIN_RENDER_RADIUS_TILES,
            last_center_cell: None,
            needs_refresh: true,
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
    if let Ok(mut state) = LAST_LOCAL_COMBAT_HEALTH.lock() {
        *state = None;
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
        .insert_resource(MiningInteractionState::default())
        .insert_resource(TerrainRenderState::default())
        .insert_resource(FootstepState::default())
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: "/".to_string(),
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(primary_window),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .add_systems(Startup, setup_world)
        .add_systems(Update, apply_pending_session_reset)
        .add_systems(Update, sync_player_id.after(apply_pending_session_reset))
        .add_systems(Update, simulate_local_player.after(sync_player_id))
        .add_systems(Update, emit_footstep_audio.after(simulate_local_player))
        .add_systems(
            Update,
            handle_build_placement_controls.after(emit_footstep_audio),
        )
        .add_systems(
            Update,
            handle_mining_controls.after(handle_build_placement_controls),
        )
        .add_systems(
            Update,
            handle_drop_pickup_controls.after(handle_mining_controls),
        )
        .add_systems(
            Update,
            handle_crafting_controls.after(handle_drop_pickup_controls),
        )
        .add_systems(
            Update,
            emit_projectile_fire_command.after(handle_crafting_controls),
        )
        .add_systems(
            Update,
            simulate_predicted_projectiles.after(emit_projectile_fire_command),
        )
        .add_systems(
            Update,
            apply_latest_snapshot.after(simulate_predicted_projectiles),
        )
        .add_systems(Update, smooth_remote_motion.after(apply_latest_snapshot))
        .add_systems(
            Update,
            animate_character_sprites.after(smooth_remote_motion),
        )
        .add_systems(Update, animate_mining_effects.after(apply_latest_snapshot))
        .add_systems(Update, animate_combat_popups.after(apply_latest_snapshot))
        .add_systems(Update, follow_camera.after(animate_combat_popups));
    app.add_systems(Update, sync_terrain_tiles.after(apply_latest_snapshot));

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

fn build_character_atlas_handles(
    asset_server: &AssetServer,
    atlas_layouts: &mut Assets<TextureAtlasLayout>,
) -> CharacterAtlasHandles {
    let layout = atlas_layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(CHARACTER_FRAME_SIZE as u32),
        CHARACTER_ANIMATION_FRAMES as u32,
        4,
        None,
        None,
    ));

    let mut by_sprite_id = HashMap::new();
    for (sprite_id, texture_path) in CHARACTER_SPRITE_VARIANTS {
        by_sprite_id.insert(
            sprite_id.to_string(),
            CharacterAtlasHandle {
                texture: asset_server.load(texture_path),
                layout: layout.clone(),
            },
        );
    }

    CharacterAtlasHandles {
        default_sprite_id: DEFAULT_CHARACTER_SPRITE_ID.to_string(),
        by_sprite_id,
    }
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

    let atlas_handles = build_character_atlas_handles(&asset_server, &mut atlas_layouts);
    let (default_atlas, resolved_sprite_id) = atlas_handles.resolve(DEFAULT_CHARACTER_SPRITE_ID);
    let local_actor = commands
        .spawn((
            SpriteBundle {
                texture: default_atlas.texture.clone(),
                transform: Transform::from_xyz(0.0, 0.0, SNAPSHOT_Z)
                    .with_scale(Vec3::splat(CHARACTER_SCALE)),
                ..default()
            },
            TextureAtlas {
                layout: default_atlas.layout.clone(),
                index: 0,
            },
            Actor {
                id: "local-pending".to_string(),
            },
            ActorVelocity::default(),
            CharacterAnimator::default(),
            CharacterSpriteId(resolved_sprite_id),
            LocalActor,
        ))
        .id();
    let local_name_label = spawn_actor_name_label(&mut commands, "local-pending");
    commands.entity(local_actor).add_child(local_name_label);
    commands
        .entity(local_actor)
        .insert(ActorNameLabelEntity(local_name_label));

    commands.insert_resource(atlas_handles);

    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: structure_preview_color("beacon", true, true),
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
    mut mining_interaction: ResMut<MiningInteractionState>,
    mut terrain_state: ResMut<TerrainRenderState>,
    mut footstep_state: ResMut<FootstepState>,
    mut local_query: Query<
        (
            &mut Transform,
            &mut Actor,
            &mut ActorVelocity,
            &mut CharacterAnimator,
            &mut TextureAtlas,
            &mut CharacterSpriteId,
            &ActorNameLabelEntity,
        ),
        (With<LocalActor>, Without<LocalBuildGhost>),
    >,
    mut label_query: Query<&mut Text, With<ActorNameLabel>>,
    mut local_build_ghost_query: Query<
        (&mut Visibility, &mut Transform),
        (With<LocalBuildGhost>, Without<LocalActor>),
    >,
    resettable_query: Query<
        Entity,
        Or<(
            With<RemoteActor>,
            With<StructureActor>,
            With<MiningNodeActor>,
            With<MiningProgressActor>,
            With<DropActor>,
            With<BuildPreviewActor>,
            With<ProjectileActor>,
            With<EnemyActor>,
            With<EnemyHealthBarActor>,
            With<CombatPopupActor>,
            With<PredictedProjectileActor>,
            With<TerrainTileActor>,
            With<TerrainResourceOverlayActor>,
        )>,
    >,
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
    *mining_interaction = MiningInteractionState::default();
    terrain_state.last_center_cell = None;
    terrain_state.needs_refresh = true;
    *footstep_state = FootstepState::default();

    if let Ok((
        mut transform,
        mut actor,
        mut velocity,
        mut animator,
        mut atlas,
        mut sprite_id,
        label_entity,
    )) = local_query.get_single_mut()
    {
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
        velocity.0 = Vec2::ZERO;
        actor.id = "local-pending".to_string();
        animator.facing = FacingDirection::Down;
        animator.frame = 0;
        animator.timer.reset();
        atlas.index = FacingDirection::Down.row_index() * CHARACTER_ANIMATION_FRAMES;
        sprite_id.0 = DEFAULT_CHARACTER_SPRITE_ID.to_string();
        set_actor_name_label_text(
            label_entity.0,
            fallback_display_name("local-pending").as_str(),
            &mut label_query,
        );
    }

    if let Ok((mut visibility, mut transform)) = local_build_ghost_query.get_single_mut() {
        *visibility = Visibility::Hidden;
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
    }

    for entity in &resettable_query {
        commands.entity(entity).despawn_recursive();
    }
}

fn sync_player_id(
    mut current_player_id: ResMut<CurrentPlayerId>,
    mut local_actor_query: Query<(&mut Actor, &ActorNameLabelEntity), With<LocalActor>>,
    mut label_query: Query<&mut Text, With<ActorNameLabel>>,
) {
    let next_id = match NEXT_PLAYER_ID.lock() {
        Ok(mut pending) => pending.take(),
        Err(_) => None,
    };

    if let Some(player_id) = next_id {
        current_player_id.0 = Some(player_id.clone());

        if let Ok((mut local_actor, label_entity)) = local_actor_query.get_single_mut() {
            local_actor.id = player_id;
            set_actor_name_label_text(
                label_entity.0,
                fallback_display_name(local_actor.id.as_str()).as_str(),
                &mut label_query,
            );
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
        sprite.color = structure_preview_color(kind, true, true);
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

fn handle_mining_controls(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    placement: Res<BuildPlacementState>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    node_query: Query<(&MiningNodeActor, &Transform)>,
    mut mining_interaction: ResMut<MiningInteractionState>,
) {
    let cancel_active = |node_id: &str| {
        queue_feature_command(
            "mining",
            "cancel",
            json!({
                "nodeId": node_id,
            }),
        );
    };

    if placement.active {
        if let Some(active_node_id) = mining_interaction.active_node_id.take() {
            cancel_active(active_node_id.as_str());
        }
        return;
    }

    let hovered_node_id = if let Ok(window) = window_query.get_single() {
        if let Some(cursor_pos) = window.cursor_position() {
            if let Ok((camera, camera_transform)) = camera_query.get_single() {
                if let Some(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) {
                    let mut hovered: Option<(String, f32)> = None;
                    for (node, transform) in &node_query {
                        if node.remaining == 0 {
                            continue;
                        }
                        let distance_sq =
                            world_pos.distance_squared(transform.translation.truncate());
                        if distance_sq > MINING_INTERACT_RADIUS * MINING_INTERACT_RADIUS {
                            continue;
                        }

                        match hovered {
                            Some((_, best_distance_sq)) if distance_sq >= best_distance_sq => {}
                            _ => {
                                hovered = Some((node.id.clone(), distance_sq));
                            }
                        }
                    }
                    hovered.map(|(node_id, _)| node_id)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    if mouse_buttons.just_released(MouseButton::Left) {
        if let Some(active_node_id) = mining_interaction.active_node_id.take() {
            cancel_active(active_node_id.as_str());
        }
        return;
    }

    if !mouse_buttons.pressed(MouseButton::Left) {
        return;
    }

    match hovered_node_id {
        Some(hovered_node_id) => {
            if mining_interaction.active_node_id.as_deref() == Some(hovered_node_id.as_str()) {
                return;
            }

            if let Some(active_node_id) = mining_interaction.active_node_id.take() {
                cancel_active(active_node_id.as_str());
            }

            queue_feature_command(
                "mining",
                "start",
                json!({
                    "nodeId": hovered_node_id,
                }),
            );
            mining_interaction.active_node_id = Some(hovered_node_id);
        }
        None => {
            if let Some(active_node_id) = mining_interaction.active_node_id.take() {
                cancel_active(active_node_id.as_str());
            }
        }
    }
}

fn handle_drop_pickup_controls(
    input: Res<ButtonInput<KeyCode>>,
    placement: Res<BuildPlacementState>,
    current_player_id: Res<CurrentPlayerId>,
    local_transform_query: Query<&Transform, With<LocalActor>>,
    drop_query: Query<(&DropActor, &Transform)>,
) {
    if placement.active || !input.just_pressed(KeyCode::KeyE) {
        return;
    }

    if current_player_id.0.is_none() {
        return;
    }
    let Ok(local_transform) = local_transform_query.get_single() else {
        return;
    };

    let local_position = local_transform.translation.truncate();
    let mut nearest_drop: Option<(&DropActor, f32)> = None;
    for (drop, transform) in &drop_query {
        let distance_sq = local_position.distance_squared(transform.translation.truncate());
        if distance_sq > DROP_PICKUP_INTERACT_RADIUS * DROP_PICKUP_INTERACT_RADIUS {
            continue;
        }

        match nearest_drop {
            Some((_, best_distance_sq)) if distance_sq >= best_distance_sq => {}
            _ => {
                nearest_drop = Some((drop, distance_sq));
            }
        }
    }

    let Some((drop, _)) = nearest_drop else {
        return;
    };

    queue_feature_command(
        "drops",
        "pickup",
        json!({
            "dropId": drop.id,
        }),
    );
}

fn queue_crafting_recipe(recipe: &str) {
    queue_feature_command(
        "crafting",
        "queue",
        json!({
            "recipe": recipe,
            "count": CRAFT_QUEUE_COUNT_PER_PRESS,
        }),
    );
}

fn handle_crafting_controls(input: Res<ButtonInput<KeyCode>>) {
    if input.just_pressed(KeyCode::Digit1) || input.just_pressed(KeyCode::Numpad1) {
        queue_crafting_recipe(RECIPE_SMELT_IRON_PLATE);
    }

    if input.just_pressed(KeyCode::Digit2) || input.just_pressed(KeyCode::Numpad2) {
        queue_crafting_recipe(RECIPE_SMELT_COPPER_PLATE);
    }

    if input.just_pressed(KeyCode::Digit3) || input.just_pressed(KeyCode::Numpad3) {
        queue_crafting_recipe(RECIPE_CRAFT_GEAR);
    }

    if input.just_pressed(KeyCode::KeyX) {
        queue_feature_command(
            "crafting",
            "cancel",
            json!({
                "clear": true,
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
    mut mining_interaction: ResMut<MiningInteractionState>,
    mut terrain_state: ResMut<TerrainRenderState>,
    mut input_history: ResMut<InputHistory>,
    mut local_query: Query<
        (
            &mut Transform,
            &mut Actor,
            &mut Handle<Image>,
            &mut TextureAtlas,
            &mut CharacterSpriteId,
            &ActorNameLabelEntity,
        ),
        (With<LocalActor>, Without<RemoteActor>),
    >,
    mut remote_sprite_query: Query<
        (
            Entity,
            &Actor,
            &ActorNameLabelEntity,
            &mut Handle<Image>,
            &mut TextureAtlas,
            &mut CharacterSpriteId,
        ),
        With<RemoteActor>,
    >,
    mut name_label_query: Query<&mut Text, With<ActorNameLabel>>,
    world_object_query: Query<
        (
            Entity,
            Option<&StructureActor>,
            Option<&EnemyActor>,
            Option<&EnemyHealthBarActor>,
        ),
        Or<(
            With<StructureActor>,
            With<EnemyActor>,
            With<EnemyHealthBarActor>,
        )>,
    >,
    mining_node_query: Query<(Entity, &MiningNodeActor)>,
    mining_progress_query: Query<(Entity, &MiningProgressActor)>,
    drop_query: Query<(Entity, &DropActor)>,
    preview_query: Query<(Entity, &BuildPreviewActor)>,
    projectile_query: Query<(Entity, &ProjectileActor)>,
    mut predicted_queries: ParamSet<(
        Query<(Entity, &PredictedProjectileActor)>,
        Query<&mut PredictedProjectileTarget>,
    )>,
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
        terrain,
        mining,
        drops,
        combat,
        character,
        ..
    } = snapshot;

    let mut character_sprite_by_player = HashMap::new();
    let mut character_name_by_player = HashMap::new();
    if let Some(snapshot) = character {
        for profile in snapshot.players {
            character_sprite_by_player.insert(profile.player_id.clone(), profile.sprite_id);
            character_name_by_player.insert(profile.player_id, profile.name);
        }
    }

    if let Some(terrain) = terrain {
        if let Ok(seed) = terrain.seed.parse::<u64>() {
            let tile_size = terrain.tile_size.max(8) as f32;
            let terrain_changed = terrain_state.seed != seed
                || terrain_state.generator_version != terrain.generator_version
                || (terrain_state.tile_size - tile_size).abs() > f32::EPSILON;

            if terrain_changed {
                terrain_state.seed = seed;
                terrain_state.generator_version = terrain.generator_version;
                terrain_state.tile_size = tile_size;
                terrain_state.last_center_cell = None;
                terrain_state.needs_refresh = true;
            }
        }
    }

    let structure_obstacles: Vec<StructureObstacle> = structures
        .iter()
        .map(|structure| StructureObstacle {
            x: structure.x,
            y: structure.y,
            half_extent: STRUCTURE_COLLIDER_HALF_EXTENT,
        })
        .collect();
    let local_player_id = current_player_id.0.clone();
    let mut local_player_world_position: Option<Vec2> = None;

    let mut remote_entities: HashMap<String, (Entity, Entity)> = remote_sprite_query
        .iter_mut()
        .map(|(entity, actor, label, ..)| (actor.id.clone(), (entity, label.0)))
        .collect();

    for player in players.into_iter().filter(|state| state.connected) {
        let is_local = current_player_id
            .0
            .as_deref()
            .is_some_and(|player_id| player_id == player.id);
        let desired_display_name = resolve_actor_display_name(
            character_name_by_player
                .get(player.id.as_str())
                .map(String::as_str),
            player.id.as_str(),
        );

        if is_local {
            local_player_world_position = Some(Vec2::new(player.x, player.y));
            if let Ok((
                mut local_transform,
                mut local_actor,
                mut local_texture,
                mut local_atlas,
                mut local_sprite_id,
                label_entity,
            )) = local_query.get_single_mut()
            {
                local_actor.id = player.id.clone();
                reconcile_local_transform(
                    &mut local_transform,
                    Vec2::new(player.x, player.y),
                    local_ack_seq,
                    &mut input_history,
                    &structure_obstacles,
                );
                local_player_world_position = Some(local_transform.translation.truncate());
                let desired_sprite_id = character_sprite_by_player
                    .get(player.id.as_str())
                    .map(String::as_str)
                    .unwrap_or(DEFAULT_CHARACTER_SPRITE_ID);
                apply_character_sprite_to_actor(
                    &mut local_texture,
                    &mut local_atlas,
                    &mut local_sprite_id,
                    desired_sprite_id,
                    &character_atlas,
                );
                set_actor_name_label_text(
                    label_entity.0,
                    desired_display_name.as_str(),
                    &mut name_label_query,
                );
            }

            if let Some((entity, _)) = remote_entities.remove(&player.id) {
                commands.entity(entity).despawn_recursive();
            }

            continue;
        }

        if let Some((entity, label_entity)) = remote_entities.remove(&player.id) {
            commands
                .entity(entity)
                .insert(RemoteTarget(Vec2::new(player.x, player.y)))
                .insert(ActorVelocity(Vec2::new(player.vx, player.vy)));
            if let Ok((_, _, _, mut remote_texture, mut remote_atlas, mut remote_sprite_id)) =
                remote_sprite_query.get_mut(entity)
            {
                let desired_sprite_id = character_sprite_by_player
                    .get(player.id.as_str())
                    .map(String::as_str)
                    .unwrap_or(DEFAULT_CHARACTER_SPRITE_ID);
                apply_character_sprite_to_actor(
                    &mut remote_texture,
                    &mut remote_atlas,
                    &mut remote_sprite_id,
                    desired_sprite_id,
                    &character_atlas,
                );
            }
            set_actor_name_label_text(
                label_entity,
                desired_display_name.as_str(),
                &mut name_label_query,
            );
        } else {
            let desired_sprite_id = character_sprite_by_player
                .get(player.id.as_str())
                .map(String::as_str)
                .unwrap_or(DEFAULT_CHARACTER_SPRITE_ID);
            spawn_remote_actor(
                &mut commands,
                &player,
                desired_sprite_id,
                desired_display_name.as_str(),
                &character_atlas,
            );
        }
    }

    for (entity, _) in remote_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }

    let mut structure_entities: HashMap<String, Entity> = world_object_query
        .iter()
        .filter_map(|(entity, structure, _, _)| {
            structure.map(|structure| (structure.id.clone(), entity))
        })
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

    let (latest_enemies, latest_combat_players) = if let Some(snapshot) = combat {
        (snapshot.enemies, snapshot.players)
    } else {
        (Vec::new(), Vec::new())
    };

    if let Some(local_id) = local_player_id.as_deref() {
        if let Some(local_combat) = latest_combat_players
            .iter()
            .find(|combat_player| combat_player.player_id == local_id)
        {
            let previous_health = if let Ok(mut state) = LAST_LOCAL_COMBAT_HEALTH.lock() {
                match state.as_mut() {
                    Some((tracked_player_id, tracked_health)) if tracked_player_id == local_id => {
                        let previous = *tracked_health;
                        *tracked_health = local_combat.health;
                        Some(previous)
                    }
                    _ => {
                        *state = Some((local_id.to_string(), local_combat.health));
                        None
                    }
                }
            } else {
                None
            };

            if let Some(previous_health) = previous_health {
                if previous_health > local_combat.health {
                    if let Some(popup_position) = local_player_world_position {
                        let popup_text = if local_combat.health == 0 {
                            "DOWN".to_string()
                        } else {
                            format!("-{}", previous_health - local_combat.health)
                        };
                        spawn_combat_popup(
                            &mut commands,
                            popup_text,
                            popup_position.x,
                            popup_position.y + PLAYER_DAMAGE_POPUP_OFFSET_Y,
                            Color::srgb_u8(255, 126, 126),
                        );
                    }
                }
            }
        } else {
            if let Ok(mut state) = LAST_LOCAL_COMBAT_HEALTH.lock() {
                if state
                    .as_ref()
                    .is_some_and(|(tracked_player_id, _)| tracked_player_id == local_id)
                {
                    *state = None;
                }
            }
        }
    }

    let mut enemy_entities: HashMap<String, (Entity, u32)> = world_object_query
        .iter()
        .filter_map(|(entity, _, enemy, _)| enemy.map(|enemy| (enemy.id.clone(), (entity, enemy.health))))
        .collect();
    let mut enemy_health_bar_entities: HashMap<String, Entity> = world_object_query
        .iter()
        .filter_map(|(entity, _, _, bar)| bar.map(|bar| (bar.enemy_id.clone(), entity)))
        .collect();
    for enemy in latest_enemies {
        if enemy.health == 0 {
            if let Some((entity, _)) = enemy_entities.remove(enemy.id.as_str()) {
                commands.entity(entity).despawn_recursive();
            }
            if let Some(bar_entity) = enemy_health_bar_entities.remove(enemy.id.as_str()) {
                commands.entity(bar_entity).despawn_recursive();
            }
            continue;
        }

        let targets_local_player = local_player_id
            .as_deref()
            .zip(enemy.target_player_id.as_deref())
            .is_some_and(|(local_id, target_id)| local_id == target_id);

        if let Some((entity, previous_health)) = enemy_entities.remove(enemy.id.as_str()) {
            if previous_health > enemy.health {
                let applied_damage = previous_health - enemy.health;
                spawn_combat_popup(
                    &mut commands,
                    format!("-{applied_damage}"),
                    enemy.x,
                    enemy.y + enemy_size(enemy.kind.as_str()) * 0.92,
                    Color::srgb_u8(255, 205, 126),
                );
            }
            commands.entity(entity).insert((
                Transform::from_xyz(enemy.x, enemy.y, ENEMY_Z),
                Sprite {
                    color: enemy_color(
                        enemy.kind.as_str(),
                        enemy.health,
                        enemy.max_health,
                        targets_local_player,
                    ),
                    custom_size: Some(Vec2::splat(enemy_size(enemy.kind.as_str()))),
                    ..default()
                },
                EnemyActor {
                    id: enemy.id.clone(),
                    health: enemy.health,
                },
            ));
        } else {
            spawn_enemy_actor(&mut commands, &enemy, targets_local_player);
        }

        if let Some(bar_entity) = enemy_health_bar_entities.remove(enemy.id.as_str()) {
            commands.entity(bar_entity).insert((
                Transform::from_xyz(
                    enemy.x,
                    enemy.y + enemy_size(enemy.kind.as_str()) + ENEMY_HEALTH_BAR_OFFSET_Y,
                    ENEMY_HEALTH_BAR_Z,
                ),
                Sprite {
                    color: enemy_health_bar_color(
                        enemy.health,
                        enemy.max_health,
                        targets_local_player,
                    ),
                    custom_size: Some(Vec2::new(
                        enemy_health_bar_width(enemy.kind.as_str(), enemy.health, enemy.max_health),
                        ENEMY_HEALTH_BAR_HEIGHT,
                    )),
                    ..default()
                },
                EnemyHealthBarActor {
                    enemy_id: enemy.id.clone(),
                },
            ));
        } else {
            spawn_enemy_health_bar_actor(&mut commands, &enemy, targets_local_player);
        }
    }

    for (entity, _) in enemy_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }
    for entity in enemy_health_bar_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }

    let (mining_nodes, mining_active) = if let Some(mining_snapshot) = mining {
        (mining_snapshot.nodes, mining_snapshot.active)
    } else {
        (Vec::new(), Vec::new())
    };
    let mut active_node_ids: HashSet<String> = mining_active
        .iter()
        .map(|progress| progress.node_id.clone())
        .collect();
    let local_active_mining = local_player_id.as_deref().and_then(|player_id| {
        mining_active
            .iter()
            .find(|progress| progress.player_id == player_id)
    });
    mining_interaction.active_node_id =
        local_active_mining.map(|progress| progress.node_id.clone());

    let mut mining_node_entities: HashMap<String, Entity> = mining_node_query
        .iter()
        .map(|(entity, node)| (node.id.clone(), entity))
        .collect();
    let mut mining_node_lookup = HashMap::new();
    for node in mining_nodes {
        if node.remaining == 0 {
            active_node_ids.remove(node.id.as_str());
            if let Some(entity) = mining_node_entities.remove(node.id.as_str()) {
                commands.entity(entity).despawn_recursive();
            }
            continue;
        }

        mining_node_lookup.insert(node.id.clone(), node.clone());
        if let Some(entity) = mining_node_entities.remove(node.id.as_str()) {
            commands.entity(entity).insert((
                Transform::from_xyz(node.x, node.y, MINING_NODE_Z),
                MiningNodeActor {
                    id: node.id.clone(),
                    kind: node.kind,
                    remaining: node.remaining,
                    active: active_node_ids.contains(node.id.as_str()),
                },
            ));
        } else {
            spawn_mining_node_actor(
                &mut commands,
                &node,
                active_node_ids.contains(node.id.as_str()),
            );
        }
    }

    for entity in mining_node_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }

    let mut mining_progress_entities: HashMap<String, Entity> = mining_progress_query
        .iter()
        .map(|(entity, progress)| (progress.player_id.clone(), entity))
        .collect();
    for progress in mining_active {
        let Some(node) = mining_node_lookup.get(progress.node_id.as_str()) else {
            if let Some(entity) = mining_progress_entities.remove(progress.player_id.as_str()) {
                commands.entity(entity).despawn_recursive();
            }
            continue;
        };

        let normalized_progress = progress.progress.clamp(0.0, 1.0);
        if let Some(entity) = mining_progress_entities.remove(progress.player_id.as_str()) {
            commands.entity(entity).insert((
                Transform::from_xyz(node.x, node.y + MINING_NODE_BASE_SIZE, MINING_PROGRESS_Z),
                MiningProgressActor {
                    player_id: progress.player_id,
                    node_id: progress.node_id,
                    progress: normalized_progress,
                },
            ));
        } else {
            spawn_mining_progress_actor(
                &mut commands,
                node.x,
                node.y + MINING_NODE_BASE_SIZE,
                normalized_progress,
                progress.player_id,
                progress.node_id,
            );
        }
    }

    for entity in mining_progress_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }

    let latest_drops = drops.map_or_else(Vec::new, |snapshot| snapshot.drops);
    let mut drop_entities: HashMap<String, Entity> = drop_query
        .iter()
        .map(|(entity, drop)| (drop.id.clone(), entity))
        .collect();
    for drop in latest_drops {
        if let Some(entity) = drop_entities.remove(drop.id.as_str()) {
            commands.entity(entity).insert((
                Transform::from_xyz(drop.x, drop.y, DROP_Z),
                DropActor {
                    id: drop.id.clone(),
                },
            ));
        } else {
            spawn_drop_actor(&mut commands, &drop);
        }
    }

    for entity in drop_entities.values() {
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
            commands.entity(entity).insert((
                Transform::from_xyz(preview.x, preview.y, BUILD_PREVIEW_Z),
                Sprite {
                    color: structure_preview_color(preview.kind.as_str(), false, preview.can_place),
                    custom_size: Some(Vec2::splat(STRUCTURE_SIZE)),
                    ..default()
                },
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
    let mut predicted_projectile_entities: HashMap<String, Entity> = predicted_queries
        .p0()
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
                    if let Ok(mut target) = predicted_queries.p1().get_mut(predicted_entity) {
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

fn animate_mining_effects(
    time: Res<Time>,
    current_player_id: Res<CurrentPlayerId>,
    mut node_query: Query<(&MiningNodeActor, &mut Sprite), Without<MiningProgressActor>>,
    mut progress_query: Query<
        (&MiningProgressActor, &mut Sprite, &mut Transform),
        Without<MiningNodeActor>,
    >,
) {
    let t = time.elapsed_seconds();
    let pulse = ((t * 8.5).sin() * 0.5 + 0.5).clamp(0.0, 1.0);

    for (node, mut sprite) in &mut node_query {
        let base = mining_node_base_color(node.kind.as_str()).to_srgba();
        let richness = (node.remaining as f32 / 1200.0).clamp(0.22, 1.0);
        let active_boost = if node.active {
            0.08 + pulse * 0.12
        } else {
            0.0
        };
        let alpha = if node.active { 0.92 } else { 0.78 };
        let scale = if node.active { 1.0 + pulse * 0.15 } else { 1.0 };

        sprite.color = Color::srgba(
            (base.red * (0.82 + richness * 0.28) + active_boost).clamp(0.0, 1.0),
            (base.green * (0.82 + richness * 0.28) + active_boost).clamp(0.0, 1.0),
            (base.blue * (0.82 + richness * 0.28) + active_boost).clamp(0.0, 1.0),
            alpha,
        );
        sprite.custom_size = Some(Vec2::splat(mining_node_size(node.remaining) * scale));
    }

    for (progress, mut sprite, mut transform) in &mut progress_query {
        let node_phase = (progress.node_id.len() as f32 % 9.0) * 0.33;
        let local_pulse = (((t + node_phase) * 11.0).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
        let width = (MINING_PROGRESS_BAR_WIDTH * progress.progress.clamp(0.0, 1.0)).max(1.0);
        let is_local = current_player_id
            .0
            .as_deref()
            .is_some_and(|player_id| player_id == progress.player_id);

        sprite.custom_size = Some(Vec2::new(width, MINING_PROGRESS_BAR_HEIGHT));
        sprite.color = if is_local {
            Color::srgba(0.45, 0.96, 0.74, 0.62 + local_pulse * 0.34)
        } else {
            Color::srgba(0.99, 0.81, 0.49, 0.46 + local_pulse * 0.28)
        };
        transform.translation.z =
            MINING_PROGRESS_Z + if is_local { 0.06 * local_pulse } else { 0.0 };
    }
}

fn animate_combat_popups(
    time: Res<Time>,
    mut commands: Commands,
    mut popup_query: Query<(Entity, &mut CombatPopupActor, &mut Transform, &mut Text)>,
) {
    let dt = time.delta_seconds();
    for (entity, mut popup, mut transform, mut text) in &mut popup_query {
        popup.ttl -= dt;
        if popup.ttl <= 0.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }

        transform.translation.y += popup.rise_speed * dt;
        let alpha = (popup.ttl / popup.max_ttl).clamp(0.0, 1.0);
        let base = popup.base_color.to_srgba();
        if let Some(section) = text.sections.get_mut(0) {
            section.style.color = Color::srgba(base.red, base.green, base.blue, alpha);
        }
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

fn world_to_terrain_cell(world_axis: f32, tile_size: f32) -> i32 {
    (world_axis / tile_size).floor() as i32
}

fn tint_channel(channel: u8, delta: i16) -> u8 {
    (channel as i16 + delta).clamp(0, 255) as u8
}

fn terrain_parity_delta(grid_x: i32, grid_y: i32) -> i16 {
    if (grid_x + grid_y) & 1 == 0 {
        5
    } else {
        -5
    }
}

fn terrain_base_color(base: TerrainBaseKind, grid_x: i32, grid_y: i32) -> Color {
    let (r, g, b) = match base {
        TerrainBaseKind::DeepWater => (13, 38, 86),
        TerrainBaseKind::ShallowWater => (33, 78, 124),
        TerrainBaseKind::Grass => (40, 92, 58),
        TerrainBaseKind::Dirt => (97, 79, 49),
        TerrainBaseKind::Rock => (95, 101, 112),
    };
    let delta = terrain_parity_delta(grid_x, grid_y);
    Color::srgb_u8(
        tint_channel(r, delta),
        tint_channel(g, delta),
        tint_channel(b, delta),
    )
}

fn terrain_resource_overlay_color(
    resource: TerrainResourceKind,
    richness: u16,
    grid_x: i32,
    grid_y: i32,
) -> Color {
    let (r, g, b) = match resource {
        TerrainResourceKind::IronOre => (119, 153, 184),
        TerrainResourceKind::CopperOre => (195, 123, 66),
        TerrainResourceKind::Coal => (66, 68, 74),
    };
    let delta = terrain_parity_delta(grid_x + 1, grid_y + 1);
    let alpha = (0.18 + (richness as f32 / 1200.0) * 0.35).clamp(0.18, 0.53);
    Color::srgba(
        tint_channel(r, delta) as f32 / 255.0,
        tint_channel(g, delta) as f32 / 255.0,
        tint_channel(b, delta) as f32 / 255.0,
        alpha,
    )
}

fn sync_terrain_tiles(
    mut commands: Commands,
    mut terrain_state: ResMut<TerrainRenderState>,
    local_transform_query: Query<&Transform, (With<LocalActor>, Without<TerrainTileActor>)>,
    terrain_tile_query: Query<(Entity, &TerrainTileActor)>,
    terrain_overlay_query: Query<(Entity, &TerrainResourceOverlayActor)>,
) {
    let Ok(local_transform) = local_transform_query.get_single() else {
        return;
    };

    let tile_size = terrain_state.tile_size.max(8.0);
    let center = IVec2::new(
        world_to_terrain_cell(local_transform.translation.x, tile_size),
        world_to_terrain_cell(local_transform.translation.y, tile_size),
    );
    let center_changed = terrain_state.last_center_cell != Some(center);

    if !terrain_state.needs_refresh && !center_changed {
        return;
    }

    let mut base_entities: HashMap<(i32, i32), Entity> = terrain_tile_query
        .iter()
        .map(|(entity, tile)| ((tile.grid_x, tile.grid_y), entity))
        .collect();
    let mut overlay_entities: HashMap<(i32, i32), Entity> = terrain_overlay_query
        .iter()
        .map(|(entity, overlay)| ((overlay.grid_x, overlay.grid_y), entity))
        .collect();

    if terrain_state.needs_refresh {
        for entity in base_entities.values() {
            commands.entity(*entity).despawn_recursive();
        }
        for entity in overlay_entities.values() {
            commands.entity(*entity).despawn_recursive();
        }
        base_entities.clear();
        overlay_entities.clear();
    }

    let radius = terrain_state.render_radius_tiles.max(2);
    for grid_x in (center.x - radius)..=(center.x + radius) {
        for grid_y in (center.y - radius)..=(center.y + radius) {
            let key = (grid_x, grid_y);
            let sample = sample_terrain(terrain_state.seed, grid_x, grid_y);
            if base_entities.remove(&key).is_none() {
                commands.spawn((
                    SpriteBundle {
                        sprite: Sprite {
                            color: terrain_base_color(sample.base, grid_x, grid_y),
                            custom_size: Some(Vec2::splat(tile_size)),
                            ..default()
                        },
                        transform: Transform::from_xyz(
                            grid_x as f32 * tile_size,
                            grid_y as f32 * tile_size,
                            FLOOR_Z,
                        ),
                        ..default()
                    },
                    TerrainTileActor { grid_x, grid_y },
                ));
            }

            if let Some(resource) = sample.resource {
                if overlay_entities.remove(&key).is_none() {
                    commands.spawn((
                        SpriteBundle {
                            sprite: Sprite {
                                color: terrain_resource_overlay_color(
                                    resource,
                                    sample.resource_richness,
                                    grid_x,
                                    grid_y,
                                ),
                                custom_size: Some(Vec2::splat(tile_size * 0.48)),
                                ..default()
                            },
                            transform: Transform::from_xyz(
                                grid_x as f32 * tile_size,
                                grid_y as f32 * tile_size,
                                TERRAIN_RESOURCE_OVERLAY_Z,
                            ),
                            ..default()
                        },
                        TerrainResourceOverlayActor { grid_x, grid_y },
                    ));
                }
            } else if let Some(entity) = overlay_entities.remove(&key) {
                commands.entity(entity).despawn_recursive();
            }
        }
    }

    for entity in base_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }
    for entity in overlay_entities.values() {
        commands.entity(*entity).despawn_recursive();
    }

    terrain_state.last_center_cell = Some(center);
    terrain_state.needs_refresh = false;
}

fn apply_character_sprite_to_actor(
    texture: &mut Handle<Image>,
    atlas: &mut TextureAtlas,
    sprite_component: &mut CharacterSpriteId,
    desired_sprite_id: &str,
    character_atlas: &CharacterAtlasHandles,
) {
    let (resolved, resolved_sprite_id) = character_atlas.resolve(desired_sprite_id);
    if sprite_component.0 == resolved_sprite_id {
        return;
    }

    *texture = resolved.texture.clone();
    atlas.layout = resolved.layout.clone();
    sprite_component.0 = resolved_sprite_id;
}

fn fallback_display_name(player_id: &str) -> String {
    let mut fallback = String::new();
    for ch in player_id.chars() {
        if fallback.len() >= MAX_NAME_LABEL_LENGTH {
            break;
        }
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            fallback.push(ch);
        }
    }

    if fallback.is_empty() {
        "Unknown".to_string()
    } else {
        fallback
    }
}

fn resolve_actor_display_name(authoritative_name: Option<&str>, player_id: &str) -> String {
    let Some(name) = authoritative_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return fallback_display_name(player_id);
    };

    name.chars().take(MAX_NAME_LABEL_LENGTH).collect()
}

fn spawn_actor_name_label(commands: &mut Commands, label: &str) -> Entity {
    commands
        .spawn((
            Text2dBundle {
                text: Text::from_section(
                    label.to_string(),
                    TextStyle {
                        font_size: NAME_LABEL_FONT_SIZE,
                        color: Color::srgb_u8(223, 236, 255),
                        ..default()
                    },
                ),
                text_anchor: bevy::sprite::Anchor::BottomCenter,
                transform: Transform::from_xyz(0.0, NAME_LABEL_OFFSET_Y, NAME_LABEL_Z_OFFSET),
                ..default()
            },
            ActorNameLabel,
        ))
        .id()
}

fn set_actor_name_label_text(
    label_entity: Entity,
    label_text: &str,
    label_query: &mut Query<&mut Text, With<ActorNameLabel>>,
) {
    let Ok(mut text) = label_query.get_mut(label_entity) else {
        return;
    };

    let Some(section) = text.sections.get_mut(0) else {
        return;
    };

    if section.value == label_text {
        return;
    }

    section.value.clear();
    section.value.push_str(label_text);
}

fn spawn_remote_actor(
    commands: &mut Commands,
    player: &PlayerState,
    sprite_id: &str,
    display_name: &str,
    character_atlas: &CharacterAtlasHandles,
) {
    let (atlas_handles, resolved_sprite_id) = character_atlas.resolve(sprite_id);
    let actor_entity = commands
        .spawn((
            SpriteBundle {
                texture: atlas_handles.texture.clone(),
                transform: Transform::from_xyz(player.x, player.y, SNAPSHOT_Z)
                    .with_scale(Vec3::splat(CHARACTER_SCALE)),
                ..default()
            },
            TextureAtlas {
                layout: atlas_handles.layout.clone(),
                index: 0,
            },
            Actor {
                id: player.id.clone(),
            },
            ActorVelocity(Vec2::new(player.vx, player.vy)),
            CharacterAnimator::default(),
            CharacterSpriteId(resolved_sprite_id),
            RemoteActor,
            RemoteTarget(Vec2::new(player.x, player.y)),
        ))
        .id();
    let name_label = spawn_actor_name_label(commands, display_name);
    commands.entity(actor_entity).add_child(name_label);
    commands
        .entity(actor_entity)
        .insert(ActorNameLabelEntity(name_label));
}

fn structure_color(kind: &str) -> Color {
    match kind {
        "beacon" => Color::srgb_u8(99, 210, 255),
        "miner" => Color::srgb_u8(167, 139, 250),
        "assembler" => Color::srgb_u8(74, 222, 128),
        _ => Color::srgb_u8(255, 255, 255),
    }
}

fn structure_preview_color(kind: &str, is_local: bool, can_place: bool) -> Color {
    if !can_place {
        let alpha = if is_local { 0.58 } else { 0.42 };
        return Color::srgba(235.0 / 255.0, 112.0 / 255.0, 103.0 / 255.0, alpha);
    }

    let base = structure_color(kind).to_srgba();
    let alpha = if is_local { 0.55 } else { 0.35 };
    Color::srgba(base.red, base.green, base.blue, alpha)
}

fn enemy_size(kind: &str) -> f32 {
    match kind {
        "biter_small" => ENEMY_BASE_SIZE,
        "biter_medium" => ENEMY_BASE_SIZE * 1.24,
        "spitter_small" => ENEMY_BASE_SIZE * 1.1,
        _ => ENEMY_BASE_SIZE,
    }
}

fn enemy_color(kind: &str, health: u32, max_health: u32, targets_local_player: bool) -> Color {
    let base = match kind {
        "biter_small" => Color::srgb_u8(206, 95, 84),
        "biter_medium" => Color::srgb_u8(190, 74, 67),
        "spitter_small" => Color::srgb_u8(133, 119, 214),
        _ => Color::srgb_u8(204, 89, 81),
    }
    .to_srgba();

    let health_ratio = if max_health == 0 {
        0.0
    } else {
        (health as f32 / max_health as f32).clamp(0.0, 1.0)
    };
    let vitality = 0.56 + health_ratio * 0.44;
    let aggro_boost = if targets_local_player { 0.12 } else { 0.0 };

    Color::srgba(
        (base.red * vitality + aggro_boost).clamp(0.0, 1.0),
        (base.green * vitality).clamp(0.0, 1.0),
        (base.blue * vitality).clamp(0.0, 1.0),
        0.9,
    )
}

fn enemy_health_ratio(health: u32, max_health: u32) -> f32 {
    if max_health == 0 {
        0.0
    } else {
        (health as f32 / max_health as f32).clamp(0.0, 1.0)
    }
}

fn enemy_health_bar_width(kind: &str, health: u32, max_health: u32) -> f32 {
    let ratio = enemy_health_ratio(health, max_health);
    (enemy_size(kind) * 1.25 * ratio).max(2.0)
}

fn enemy_health_bar_color(health: u32, max_health: u32, targets_local_player: bool) -> Color {
    let ratio = enemy_health_ratio(health, max_health);
    let warning_boost = if targets_local_player { 0.1 } else { 0.0 };
    let red = (0.93 - ratio * 0.36 + warning_boost).clamp(0.0, 1.0);
    let green = (0.22 + ratio * 0.68).clamp(0.0, 1.0);
    Color::srgba(red, green, 0.2, 0.92)
}

fn mining_node_base_color(kind: &str) -> Color {
    match kind {
        "iron_ore" => Color::srgb_u8(133, 163, 188),
        "copper_ore" => Color::srgb_u8(194, 116, 71),
        "coal" => Color::srgb_u8(72, 76, 84),
        _ => Color::srgb_u8(183, 192, 207),
    }
}

fn mining_node_size(remaining: u32) -> f32 {
    let richness = (remaining as f32 / 1200.0).clamp(0.18, 1.0);
    MINING_NODE_BASE_SIZE + richness * 7.5
}

fn drop_color(resource: &str) -> Color {
    match resource {
        "iron_ore" => Color::srgb_u8(168, 201, 224),
        "copper_ore" => Color::srgb_u8(218, 143, 88),
        "coal" => Color::srgb_u8(102, 108, 118),
        "stone" => Color::srgb_u8(170, 174, 182),
        "iron_plate" => Color::srgb_u8(226, 236, 246),
        "copper_plate" => Color::srgb_u8(231, 172, 122),
        "gear" => Color::srgb_u8(150, 170, 191),
        _ => Color::srgb_u8(214, 220, 236),
    }
}

fn drop_size(amount: u32) -> f32 {
    let normalized = (amount as f32 / 24.0).clamp(0.2, 1.0);
    DROP_BASE_SIZE + normalized * 7.0
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

fn spawn_enemy_actor(
    commands: &mut Commands,
    enemy: &EnemySnapshotState,
    targets_local_player: bool,
) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: enemy_color(
                    enemy.kind.as_str(),
                    enemy.health,
                    enemy.max_health,
                    targets_local_player,
                ),
                custom_size: Some(Vec2::splat(enemy_size(enemy.kind.as_str()))),
                ..default()
            },
            transform: Transform::from_xyz(enemy.x, enemy.y, ENEMY_Z),
            ..default()
        },
        EnemyActor {
            id: enemy.id.clone(),
            health: enemy.health,
        },
    ));
}

fn spawn_enemy_health_bar_actor(
    commands: &mut Commands,
    enemy: &EnemySnapshotState,
    targets_local_player: bool,
) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: enemy_health_bar_color(enemy.health, enemy.max_health, targets_local_player),
                custom_size: Some(Vec2::new(
                    enemy_health_bar_width(enemy.kind.as_str(), enemy.health, enemy.max_health),
                    ENEMY_HEALTH_BAR_HEIGHT,
                )),
                ..default()
            },
            transform: Transform::from_xyz(
                enemy.x,
                enemy.y + enemy_size(enemy.kind.as_str()) + ENEMY_HEALTH_BAR_OFFSET_Y,
                ENEMY_HEALTH_BAR_Z,
            ),
            ..default()
        },
        EnemyHealthBarActor {
            enemy_id: enemy.id.clone(),
        },
    ));
}

fn spawn_combat_popup(commands: &mut Commands, text: String, x: f32, y: f32, color: Color) {
    commands.spawn((
        Text2dBundle {
            text: Text::from_section(
                text,
                TextStyle {
                    font_size: 17.0,
                    color,
                    ..default()
                },
            ),
            text_anchor: bevy::sprite::Anchor::BottomCenter,
            transform: Transform::from_xyz(x, y, COMBAT_POPUP_Z),
            ..default()
        },
        CombatPopupActor {
            ttl: COMBAT_POPUP_TTL_SECONDS,
            max_ttl: COMBAT_POPUP_TTL_SECONDS,
            rise_speed: COMBAT_POPUP_RISE_SPEED,
            base_color: color,
        },
    ));
}

fn spawn_mining_node_actor(commands: &mut Commands, node: &MiningNodeSnapshotState, active: bool) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: mining_node_base_color(node.kind.as_str()),
                custom_size: Some(Vec2::splat(mining_node_size(node.remaining))),
                ..default()
            },
            transform: Transform::from_xyz(node.x, node.y, MINING_NODE_Z),
            ..default()
        },
        MiningNodeActor {
            id: node.id.clone(),
            kind: node.kind.clone(),
            remaining: node.remaining,
            active,
        },
    ));
}

fn spawn_mining_progress_actor(
    commands: &mut Commands,
    x: f32,
    y: f32,
    progress: f32,
    player_id: String,
    node_id: String,
) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: Color::srgba(0.49, 0.9, 0.73, 0.82),
                custom_size: Some(Vec2::new(
                    (MINING_PROGRESS_BAR_WIDTH * progress.clamp(0.0, 1.0)).max(1.0),
                    MINING_PROGRESS_BAR_HEIGHT,
                )),
                ..default()
            },
            transform: Transform::from_xyz(x, y, MINING_PROGRESS_Z),
            ..default()
        },
        MiningProgressActor {
            player_id,
            node_id,
            progress: progress.clamp(0.0, 1.0),
        },
    ));
}

fn spawn_drop_actor(commands: &mut Commands, drop: &DropSnapshotState) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: drop_color(drop.resource.as_str()),
                custom_size: Some(Vec2::splat(drop_size(drop.amount))),
                ..default()
            },
            transform: Transform::from_xyz(drop.x, drop.y, DROP_Z),
            ..default()
        },
        DropActor {
            id: drop.id.clone(),
        },
    ));
}

fn spawn_build_preview_actor(commands: &mut Commands, preview: &BuildPreviewState) {
    commands.spawn((
        SpriteBundle {
            sprite: Sprite {
                color: structure_preview_color(preview.kind.as_str(), false, preview.can_place),
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

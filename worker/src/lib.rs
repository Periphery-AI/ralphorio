use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value};
use sim_core::domain::{DEFAULT_INVENTORY_MAX_SLOTS, GAMEPLAY_SCHEMA_VERSION};
use sim_core::{
    deterministic_seed_from_room_code, movement_step_with_obstacles, projectile_step,
    InputState as CoreInputState, StructureObstacle, PLAYER_COLLIDER_RADIUS,
    STRUCTURE_COLLIDER_HALF_EXTENT, TERRAIN_GENERATOR_VERSION, TERRAIN_TILE_SIZE,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use worker::durable::{DurableObject, State, WebSocketIncomingMessage};
use worker::*;

const PROTOCOL_VERSION: u32 = 2;
const PLAYER_ID_RE_MIN: usize = 3;
const PLAYER_ID_RE_MAX: usize = 120;
const ROOM_RE_MAX: usize = 24;

const SIM_RATE_HZ: u32 = 30;
const SNAPSHOT_RATE_HZ: u32 = 10;
const SIM_DT_SECONDS: f32 = 1.0 / SIM_RATE_HZ as f32;
const SIM_DT_MS: f64 = 1000.0 / SIM_RATE_HZ as f64;
const SNAPSHOT_INTERVAL_TICKS: u64 = (SIM_RATE_HZ / SNAPSHOT_RATE_HZ) as u64;
const MAX_CATCHUP_STEPS: usize = 8;

const MOVE_SPEED: f32 = 220.0;
const MOVEMENT_MAP_LIMIT: f32 = 5000.0;
const PROJECTILE_MAP_LIMIT: f32 = 5500.0;
const PROJECTILE_TTL_MS: i64 = 1800;
const PROJECTILE_MAX_SPEED: f64 = 900.0;

const BUILD_GRID_SIZE: f64 = 32.0;
const BUILD_CHUNK_CELLS: i64 = 32;
const BUILD_PREVIEW_STALE_MS: i64 = 15000;
const STATE_CHECKPOINT_INTERVAL_MS: i64 = 1000;
const RESUME_TOKEN_TTL_MS: i64 = 86_400_000;

const PREVIEW_COMMAND_MIN_INTERVAL_MS: i64 = 40;
const PLACE_COMMAND_MIN_INTERVAL_MS: i64 = 120;
const PROJECTILE_FIRE_MIN_INTERVAL_MS: i64 = 33;

const MAX_STRUCTURES: usize = 1024;
const MAX_PROJECTILES: usize = 4096;
const MAX_PREVIEWS: usize = 256;
const ROOM_META_ROOM_CODE_KEY: &str = "room_code";
const ROOM_META_TERRAIN_SEED_KEY: &str = "terrain_seed";
const DEFAULT_CHARACTER_SPRITE_ID: &str = "engineer-default";
const DEFAULT_CHARACTER_PROFILE_ID: &str = "default";
const MAX_PROTOCOL_IDENTIFIER_LEN: usize = 64;
const MAX_CHARACTER_NAME_LEN: usize = 32;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ClientEnvelopeKind {
    Command,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProtocolFeature {
    Core,
    Movement,
    Build,
    Projectile,
    Inventory,
    Mining,
    Crafting,
    Combat,
    Character,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SocketAttachment {
    player_id: String,
    last_seq: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientCommandEnvelope {
    v: u32,
    kind: ClientEnvelopeKind,
    seq: u32,
    feature: ProtocolFeature,
    action: String,
    client_time: f64,
    payload: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerEnvelope {
    v: u32,
    kind: &'static str,
    tick: u64,
    server_time: i64,
    feature: &'static str,
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct InputState {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct InputCommand {
    seq: u32,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct InputBatchPayload {
    inputs: Vec<InputCommand>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildPlacePayload {
    x: f64,
    y: f64,
    kind: String,
    client_build_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct BuildRemovePayload {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildPreviewPayload {
    active: bool,
    x: Option<f64>,
    y: Option<f64>,
    kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectileFirePayload {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    client_projectile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InventoryMovePayload {
    from_slot: u16,
    to_slot: u16,
    amount: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InventorySplitPayload {
    slot: u16,
    amount: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MiningStartPayload {
    node_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MiningCancelPayload {
    node_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CraftQueuePayload {
    recipe: String,
    count: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CraftCancelPayload {
    recipe: Option<String>,
    clear: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CombatAttackPayload {
    target_id: String,
    attack_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CharacterMetadataPayload {
    character_id: Option<String>,
    name: String,
    sprite_id: Option<String>,
    set_active: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CharacterSelectPayload {
    character_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct JwtClaims {
    sub: String,
    sid: Option<String>,
    exp: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClerkSessionResponse {
    user_id: String,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BuildRow {
    structure_id: String,
    owner_id: String,
    kind: String,
    x: f64,
    y: f64,
    grid_x: Option<i64>,
    grid_y: Option<i64>,
    created_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct MetaValueRow {
    value: String,
}

#[derive(Debug, Deserialize)]
struct SessionTokenRow {
    token: String,
    player_id: String,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct CharacterProfileRow {
    character_id: String,
    name: String,
    sprite_id: String,
}

#[derive(Debug, Deserialize)]
struct CharacterIdRow {
    #[serde(rename = "character_id")]
    _character_id: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeHydratedPlayerRow {
    player_id: String,
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    up: i64,
    down: i64,
    left: i64,
    right: i64,
    last_input_seq: i64,
    connected: i64,
    last_seen: i64,
}

#[derive(Debug, Clone)]
struct RuntimePlayerState {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    input: InputState,
    last_input_seq: u32,
    connected: bool,
    last_seen: i64,
    last_preview_cmd_at: i64,
    last_place_cmd_at: i64,
    last_projectile_fire_at: i64,
}

#[derive(Debug, Clone)]
struct RuntimeStructureState {
    structure_id: String,
    owner_id: String,
    kind: String,
    x: f32,
    y: f32,
    grid_x: i64,
    grid_y: i64,
    chunk_x: i64,
    chunk_y: i64,
    created_at: i64,
}

#[derive(Debug, Clone)]
struct RuntimePreviewState {
    player_id: String,
    kind: String,
    x: f32,
    y: f32,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct RuntimeProjectileState {
    projectile_id: String,
    owner_id: String,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    expires_at: i64,
    client_projectile_id: Option<String>,
    updated_at: i64,
}

#[derive(Debug, Default)]
struct RoomRuntimeState {
    players: HashMap<String, RuntimePlayerState>,
    structures: HashMap<String, RuntimeStructureState>,
    previews: HashMap<String, RuntimePreviewState>,
    projectiles: HashMap<String, RuntimeProjectileState>,
}

fn now_ms() -> i64 {
    Date::now().as_millis() as i64
}

fn now_seconds() -> i64 {
    now_ms() / 1000
}

fn sanitize_room_code(input: &str) -> Option<String> {
    let candidate = input.trim().to_ascii_uppercase();
    if candidate.is_empty() || candidate.len() > ROOM_RE_MAX {
        return None;
    }

    if candidate
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Some(candidate)
    } else {
        None
    }
}

fn sanitize_player_id(input: &str) -> Option<String> {
    let candidate = input.trim();
    if candidate.len() < PLAYER_ID_RE_MIN || candidate.len() > PLAYER_ID_RE_MAX {
        return None;
    }

    if candidate
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn is_valid_protocol_identifier(input: &str) -> bool {
    let candidate = input.trim();
    if candidate.is_empty() || candidate.len() > MAX_PROTOCOL_IDENTIFIER_LEN {
        return false;
    }

    candidate
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == ':')
}

fn sanitize_character_name(input: &str) -> Option<String> {
    let candidate = input.trim();
    if candidate.is_empty() || candidate.len() > MAX_CHARACTER_NAME_LEN {
        return None;
    }

    if candidate.chars().any(|ch| ch.is_control()) {
        return None;
    }

    Some(candidate.to_string())
}

fn sanitize_character_id(input: &str) -> Option<String> {
    let candidate = input.trim();
    if !is_valid_protocol_identifier(candidate) {
        return None;
    }
    Some(candidate.to_string())
}

fn default_character_name_for_player(player_id: &str) -> String {
    let trimmed = player_id.trim();
    let mut display_name = String::new();
    let mut count = 0usize;
    for ch in trimmed.chars() {
        if ch.is_control() {
            continue;
        }
        if count >= MAX_CHARACTER_NAME_LEN {
            break;
        }
        display_name.push(ch);
        count += 1;
    }

    if display_name.is_empty() {
        "Player".to_string()
    } else {
        display_name
    }
}

fn random_player_id() -> String {
    let time = now_ms() as u64;
    let random = (js_sys::Math::random() * 1_000_000_000.0) as u64;
    format!("anon_{:x}", time ^ random)
}

fn parse_room_code_from_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 5 {
        return None;
    }

    if parts[1] != "api" || parts[2] != "rooms" || parts[4] != "ws" {
        return None;
    }

    sanitize_room_code(parts[3])
}

fn parse_query_param(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

fn parse_jwt_claims_unverified(token: &str) -> Result<JwtClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(Error::RustError("invalid JWT token format".into()));
    }

    let payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| Error::RustError("invalid JWT payload encoding".into()))?;

    serde_json::from_slice::<JwtClaims>(&payload)
        .map_err(|_| Error::RustError("invalid JWT payload".into()))
}

async fn verify_clerk_session(secret_key: &str, session_id: &str, player_id: &str) -> Result<()> {
    let url = format!("https://api.clerk.com/v1/sessions/{session_id}");

    let headers = Headers::new();
    headers.set("authorization", &format!("Bearer {secret_key}"))?;

    let mut init = RequestInit::new();
    init.with_method(Method::Get);
    init.with_headers(headers);

    let request = Request::new_with_init(&url, &init)?;
    let mut response = Fetch::Request(request).send().await?;

    if response.status_code() != 200 {
        return Err(Error::RustError("Clerk session verification failed".into()));
    }

    let clerk_session: ClerkSessionResponse = response.json().await?;
    if clerk_session.user_id != player_id {
        return Err(Error::RustError("session user mismatch".into()));
    }

    if let Some(status) = clerk_session.status {
        if status != "active" {
            return Err(Error::RustError("session not active".into()));
        }
    }

    Ok(())
}

async fn authenticate_player(url: &Url, env: &Env) -> Result<String> {
    let player_id_from_query = parse_query_param(url, "playerId")
        .as_deref()
        .and_then(sanitize_player_id);

    let token = parse_query_param(url, "token");
    let secret_key = env
        .secret("CLERK_SECRET_KEY")
        .ok()
        .map(|secret| secret.to_string());

    if let Some(secret_key) = secret_key {
        let token = token.ok_or_else(|| Error::RustError("missing Clerk token".into()))?;
        let claims = parse_jwt_claims_unverified(&token)?;

        let player_id = player_id_from_query
            .clone()
            .unwrap_or_else(|| claims.sub.clone());
        let player_id = sanitize_player_id(&player_id)
            .ok_or_else(|| Error::RustError("invalid player id".into()))?;

        if claims.sub != player_id {
            return Err(Error::RustError(
                "token subject does not match player id".into(),
            ));
        }

        let exp = claims
            .exp
            .ok_or_else(|| Error::RustError("token missing exp claim".into()))?;
        if exp <= now_seconds() {
            return Err(Error::RustError("token expired".into()));
        }

        let session_id = claims
            .sid
            .ok_or_else(|| Error::RustError("token missing sid claim".into()))?;
        verify_clerk_session(&secret_key, &session_id, &player_id).await?;

        return Ok(player_id);
    }

    Ok(player_id_from_query.unwrap_or_else(random_player_id))
}

fn json_response(payload: Value, status: u16) -> Result<Response> {
    Response::from_json(&payload).map(|response| response.with_status(status))
}

fn map_input_to_core(input: &InputState) -> CoreInputState {
    CoreInputState {
        up: input.up,
        down: input.down,
        left: input.left,
        right: input.right,
    }
}

fn is_valid_structure_kind(kind: &str) -> bool {
    matches!(kind, "beacon" | "miner" | "assembler")
}

fn structure_half_extent(_kind: &str) -> f32 {
    STRUCTURE_COLLIDER_HALF_EXTENT
}

fn snap_axis_to_grid(value: f64) -> i64 {
    let clamped = value.clamp(-(MOVEMENT_MAP_LIMIT as f64), MOVEMENT_MAP_LIMIT as f64);
    (clamped / BUILD_GRID_SIZE).round() as i64
}

fn grid_cell_center(grid_axis: i64) -> f64 {
    grid_axis as f64 * BUILD_GRID_SIZE
}

fn chunk_coord_for_grid(grid_axis: i64) -> i64 {
    if grid_axis >= 0 {
        grid_axis / BUILD_CHUNK_CELLS
    } else {
        ((grid_axis + 1) / BUILD_CHUNK_CELLS) - 1
    }
}

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let url = req.url()?;
    let method = req.method();
    let fetch_mode = req
        .headers()
        .get("Sec-Fetch-Mode")?
        .unwrap_or_default()
        .to_ascii_lowercase();
    let accept_header = req
        .headers()
        .get("Accept")?
        .unwrap_or_default()
        .to_ascii_lowercase();

    if url.path() == "/api/health" {
        return Response::from_json(&json!({
            "ok": true,
            "timestamp": now_ms(),
        }));
    }

    if let Some(room_code) = parse_room_code_from_path(url.path()) {
        let upgrade = req
            .headers()
            .get("Upgrade")?
            .unwrap_or_default()
            .to_ascii_lowercase();

        if upgrade != "websocket" {
            return json_response(json!({ "error": "Expected websocket upgrade." }), 426);
        }

        let namespace = env.durable_object("ROOMS")?;
        let object_id = namespace.id_from_name(&room_code)?;
        let stub = object_id.get_stub()?;
        return stub.fetch_with_request(req).await;
    }

    if let Ok(assets) = env.assets("ASSETS") {
        let asset_response = assets.fetch_request(req).await?;
        if asset_response.status_code() != 404 {
            return Ok(asset_response);
        }

        let is_get_or_head = matches!(method, Method::Get | Method::Head);
        let is_html_navigation = fetch_mode == "navigate" || accept_header.contains("text/html");
        let path = url.path();
        let last_segment = path.rsplit('/').next().unwrap_or_default();
        let looks_like_static_file = last_segment.contains('.');
        if is_get_or_head
            && is_html_navigation
            && !path.starts_with("/api/")
            && !looks_like_static_file
        {
            let mut index_url = url.clone();
            index_url.set_path("/index.html");
            index_url.set_query(None);
            index_url.set_fragment(None);

            let mut init = RequestInit::new();
            init.with_method(method);
            let index_request = Request::new_with_init(index_url.as_str(), &init)?;
            return assets.fetch_request(index_request).await;
        }

        return Ok(asset_response);
    }

    Response::error("Not Found", 404)
}

#[durable_object]
pub struct RoomDurableObject {
    state: State,
    env: Env,
    room_code: RefCell<String>,
    tick: Cell<u64>,
    last_loop_ms: Cell<f64>,
    accumulator_ms: Cell<f64>,
    last_checkpoint_ms: Cell<i64>,
    snapshot_dirty: Cell<bool>,
    dirty_presence: Cell<bool>,
    dirty_build: Cell<bool>,
    dirty_projectiles: Cell<bool>,
    terrain_seed: Cell<u64>,
    runtime: RefCell<RoomRuntimeState>,
}

impl RoomDurableObject {
    fn sql(&self) -> SqlStorage {
        self.state.storage().sql()
    }

    fn default_runtime_player(now: i64) -> RuntimePlayerState {
        RuntimePlayerState {
            x: 0.0,
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
            input: InputState {
                up: false,
                down: false,
                left: false,
                right: false,
            },
            last_input_seq: 0,
            connected: false,
            last_seen: now,
            last_preview_cmd_at: 0,
            last_place_cmd_at: 0,
            last_projectile_fire_at: 0,
        }
    }

    fn hydrate_runtime_from_db(&self) -> Result<()> {
        let sql = self.sql();
        let now = now_ms();

        let player_rows: Vec<RuntimeHydratedPlayerRow> = sql
            .exec(
                "
                SELECT s.player_id, s.x, s.y, s.vx, s.vy,
                       COALESCE(i.up, 0) AS up,
                       COALESCE(i.down, 0) AS down,
                       COALESCE(i.left, 0) AS left,
                       COALESCE(i.right, 0) AS right,
                       COALESCE(i.last_input_seq, 0) AS last_input_seq,
                       COALESCE(p.connected, 0) AS connected,
                       COALESCE(p.last_seen, 0) AS last_seen
                FROM movement_state s
                LEFT JOIN movement_input_state i ON i.player_id = s.player_id
                LEFT JOIN presence_players p ON p.player_id = s.player_id
                ORDER BY s.player_id ASC
                ",
                None,
            )?
            .to_array()?;

        let structure_rows: Vec<BuildRow> = sql
            .exec(
                "
                SELECT structure_id, owner_id, kind, x, y, grid_x, grid_y, created_at
                FROM build_structures
                ORDER BY created_at ASC
                LIMIT ?
                ",
                Some(vec![(MAX_STRUCTURES as i64).into()]),
            )?
            .to_array()?;

        let mut runtime = self.runtime.borrow_mut();
        runtime.players.clear();
        runtime.structures.clear();
        runtime.previews.clear();
        runtime.projectiles.clear();

        for row in player_rows {
            runtime.players.insert(
                row.player_id.clone(),
                RuntimePlayerState {
                    x: row.x as f32,
                    y: row.y as f32,
                    vx: row.vx as f32,
                    vy: row.vy as f32,
                    input: InputState {
                        up: row.up != 0,
                        down: row.down != 0,
                        left: row.left != 0,
                        right: row.right != 0,
                    },
                    last_input_seq: row.last_input_seq.max(0) as u32,
                    connected: row.connected != 0,
                    last_seen: row.last_seen.max(0),
                    last_preview_cmd_at: 0,
                    last_place_cmd_at: 0,
                    last_projectile_fire_at: 0,
                },
            );
        }

        for row in structure_rows {
            let grid_x = row.grid_x.unwrap_or_else(|| snap_axis_to_grid(row.x));
            let grid_y = row.grid_y.unwrap_or_else(|| snap_axis_to_grid(row.y));
            runtime.structures.insert(
                row.structure_id.clone(),
                RuntimeStructureState {
                    structure_id: row.structure_id,
                    owner_id: row.owner_id,
                    kind: row.kind,
                    x: grid_cell_center(grid_x) as f32,
                    y: grid_cell_center(grid_y) as f32,
                    grid_x,
                    grid_y,
                    chunk_x: chunk_coord_for_grid(grid_x),
                    chunk_y: chunk_coord_for_grid(grid_y),
                    created_at: row.created_at.unwrap_or(now),
                },
            );
        }

        drop(runtime);

        // Preview/projectile state is ephemeral and should not survive process hibernation.
        sql.exec("DELETE FROM build_previews", None)?;
        sql.exec("DELETE FROM projectile_state", None)?;
        Ok(())
    }

    fn checkpoint_runtime_players_to_db(&self) -> Result<()> {
        let sql = self.sql();
        let runtime = self.runtime.borrow();

        for (player_id, player) in runtime.players.iter() {
            sql.exec(
                "
                INSERT INTO movement_state (player_id, x, y, vx, vy, updated_at)
                VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(player_id) DO UPDATE SET
                  x = excluded.x,
                  y = excluded.y,
                  vx = excluded.vx,
                  vy = excluded.vy,
                  updated_at = excluded.updated_at
                ",
                Some(vec![
                    player_id.as_str().into(),
                    (player.x as f64).into(),
                    (player.y as f64).into(),
                    (player.vx as f64).into(),
                    (player.vy as f64).into(),
                    player.last_seen.into(),
                ]),
            )?;

            sql.exec(
                "
                INSERT INTO movement_input_state (player_id, up, down, left, right, last_input_seq, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(player_id) DO UPDATE SET
                  up = excluded.up,
                  down = excluded.down,
                  left = excluded.left,
                  right = excluded.right,
                  last_input_seq = excluded.last_input_seq,
                  updated_at = excluded.updated_at
                ",
                Some(vec![
                    player_id.as_str().into(),
                    (player.input.up as i64).into(),
                    (player.input.down as i64).into(),
                    (player.input.left as i64).into(),
                    (player.input.right as i64).into(),
                    (player.last_input_seq as i64).into(),
                    player.last_seen.into(),
                ]),
            )?;

            sql.exec(
                "
                INSERT INTO presence_players (player_id, connected, last_seen)
                VALUES (?, ?, ?)
                ON CONFLICT(player_id) DO UPDATE SET
                  connected = excluded.connected,
                  last_seen = excluded.last_seen
                ",
                Some(vec![
                    player_id.as_str().into(),
                    (player.connected as i64).into(),
                    player.last_seen.into(),
                ]),
            )?;
        }

        Ok(())
    }

    fn checkpoint_runtime_if_due(&self) -> Result<()> {
        let now = now_ms();
        if now - self.last_checkpoint_ms.get() < STATE_CHECKPOINT_INTERVAL_MS {
            return Ok(());
        }

        self.checkpoint_runtime_players_to_db()?;
        self.last_checkpoint_ms.set(now);
        Ok(())
    }

    fn persist_structure_insert(&self, structure: &RuntimeStructureState) -> Result<()> {
        self.sql().exec(
            "
            INSERT INTO build_structures (structure_id, owner_id, kind, x, y, grid_x, grid_y, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(structure_id) DO UPDATE SET
              owner_id = excluded.owner_id,
              kind = excluded.kind,
              x = excluded.x,
              y = excluded.y,
              grid_x = excluded.grid_x,
              grid_y = excluded.grid_y
            ",
            Some(vec![
                structure.structure_id.as_str().into(),
                structure.owner_id.as_str().into(),
                structure.kind.as_str().into(),
                (structure.x as f64).into(),
                (structure.y as f64).into(),
                structure.grid_x.into(),
                structure.grid_y.into(),
                structure.created_at.into(),
            ]),
        )?;
        Ok(())
    }

    fn persist_structure_delete(&self, structure_id: &str) -> Result<()> {
        self.sql().exec(
            "DELETE FROM build_structures WHERE structure_id = ?",
            Some(vec![structure_id.into()]),
        )?;
        Ok(())
    }

    fn backfill_character_profiles_from_presence(&self) -> Result<()> {
        let now = now_ms();
        let sql = self.sql();

        sql.exec(
            "
            INSERT INTO character_profiles (user_id, character_id, name, sprite_id, created_at, updated_at)
            SELECT p.player_id,
                   ?,
                   CASE
                     WHEN LENGTH(TRIM(p.player_id)) = 0 THEN ?
                     ELSE SUBSTR(TRIM(p.player_id), 1, ?)
                   END,
                   ?,
                   ?,
                   ?
            FROM presence_players p
            LEFT JOIN character_profiles cp
              ON cp.user_id = p.player_id
             AND cp.character_id = ?
            WHERE cp.user_id IS NULL
            ",
            Some(vec![
                DEFAULT_CHARACTER_PROFILE_ID.into(),
                "Player".into(),
                (MAX_CHARACTER_NAME_LEN as i64).into(),
                DEFAULT_CHARACTER_SPRITE_ID.into(),
                now.into(),
                now.into(),
                DEFAULT_CHARACTER_PROFILE_ID.into(),
            ]),
        )?;

        sql.exec(
            "
            INSERT INTO active_character_profiles (user_id, character_id, updated_at)
            SELECT cp.user_id, ?, ?
            FROM character_profiles cp
            LEFT JOIN active_character_profiles ap ON ap.user_id = cp.user_id
            WHERE cp.character_id = ?
              AND ap.user_id IS NULL
            ",
            Some(vec![
                DEFAULT_CHARACTER_PROFILE_ID.into(),
                now.into(),
                DEFAULT_CHARACTER_PROFILE_ID.into(),
            ]),
        )?;

        Ok(())
    }

    fn ensure_default_character_profile(&self, user_id: &str) -> Result<()> {
        let now = now_ms();
        let default_name = default_character_name_for_player(user_id);
        let sql = self.sql();

        sql.exec(
            "
            INSERT INTO character_profiles (user_id, character_id, name, sprite_id, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(user_id, character_id) DO NOTHING
            ",
            Some(vec![
                user_id.into(),
                DEFAULT_CHARACTER_PROFILE_ID.into(),
                default_name.into(),
                DEFAULT_CHARACTER_SPRITE_ID.into(),
                now.into(),
                now.into(),
            ]),
        )?;

        sql.exec(
            "
            INSERT INTO active_character_profiles (user_id, character_id, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(user_id) DO NOTHING
            ",
            Some(vec![
                user_id.into(),
                DEFAULT_CHARACTER_PROFILE_ID.into(),
                now.into(),
            ]),
        )?;

        Ok(())
    }

    fn upsert_character_profile(
        &self,
        user_id: &str,
        character_id: &str,
        name: &str,
        sprite_id: &str,
        set_active: bool,
    ) -> Result<()> {
        let now = now_ms();
        self.sql().exec(
            "
            INSERT INTO character_profiles (user_id, character_id, name, sprite_id, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(user_id, character_id) DO UPDATE SET
              name = excluded.name,
              sprite_id = excluded.sprite_id,
              updated_at = excluded.updated_at
            ",
            Some(vec![
                user_id.into(),
                character_id.into(),
                name.into(),
                sprite_id.into(),
                now.into(),
                now.into(),
            ]),
        )?;

        if set_active {
            self.set_active_character_profile(user_id, character_id)?;
        }

        Ok(())
    }

    fn set_active_character_profile(&self, user_id: &str, character_id: &str) -> Result<()> {
        let rows: Vec<CharacterIdRow> = self
            .sql()
            .exec(
                "
                SELECT character_id
                FROM character_profiles
                WHERE user_id = ? AND character_id = ?
                LIMIT 1
                ",
                Some(vec![user_id.into(), character_id.into()]),
            )?
            .to_array()?;

        if rows.is_empty() {
            return Err(Error::RustError("character profile does not exist".into()));
        }

        let now = now_ms();
        self.sql().exec(
            "
            INSERT INTO active_character_profiles (user_id, character_id, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(user_id) DO UPDATE SET
              character_id = excluded.character_id,
              updated_at = excluded.updated_at
            ",
            Some(vec![user_id.into(), character_id.into(), now.into()]),
        )?;

        Ok(())
    }

    fn load_active_character_profile(&self, user_id: &str) -> Result<CharacterProfileRow> {
        let rows: Vec<CharacterProfileRow> = self
            .sql()
            .exec(
                "
                SELECT cp.character_id, cp.name, cp.sprite_id
                FROM active_character_profiles ap
                JOIN character_profiles cp
                  ON cp.user_id = ap.user_id
                 AND cp.character_id = ap.character_id
                WHERE ap.user_id = ?
                LIMIT 1
                ",
                Some(vec![user_id.into()]),
            )?
            .to_array()?;

        if let Some(profile) = rows.into_iter().next() {
            return Ok(profile);
        }

        // Repair missing/partial legacy rows by restoring a stable default profile.
        self.ensure_default_character_profile(user_id)?;
        self.sql().exec(
            "
            INSERT INTO active_character_profiles (user_id, character_id, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(user_id) DO UPDATE SET
              character_id = excluded.character_id,
              updated_at = excluded.updated_at
            ",
            Some(vec![
                user_id.into(),
                DEFAULT_CHARACTER_PROFILE_ID.into(),
                now_ms().into(),
            ]),
        )?;

        Ok(CharacterProfileRow {
            character_id: DEFAULT_CHARACTER_PROFILE_ID.to_string(),
            name: default_character_name_for_player(user_id),
            sprite_id: DEFAULT_CHARACTER_SPRITE_ID.to_string(),
        })
    }

    fn initialize_schema(&self) -> Result<()> {
        let sql = self.sql();

        sql.exec(
            "CREATE TABLE IF NOT EXISTS room_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS session_tokens (
              token TEXT PRIMARY KEY,
              player_id TEXT NOT NULL,
              expires_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            )
            ",
            None,
        )?;

        sql.exec(
            "CREATE INDEX IF NOT EXISTS idx_session_tokens_player ON session_tokens(player_id)",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS presence_players (
              player_id TEXT PRIMARY KEY,
              connected INTEGER NOT NULL DEFAULT 0,
              last_seen INTEGER NOT NULL
            )
            ",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS character_profiles (
              user_id TEXT NOT NULL,
              character_id TEXT NOT NULL,
              name TEXT NOT NULL,
              sprite_id TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              PRIMARY KEY(user_id, character_id)
            )
            ",
            None,
        )?;

        sql.exec(
            "CREATE INDEX IF NOT EXISTS idx_character_profiles_user ON character_profiles(user_id)",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS active_character_profiles (
              user_id TEXT PRIMARY KEY,
              character_id TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            )
            ",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS movement_state (
              player_id TEXT PRIMARY KEY,
              x REAL NOT NULL DEFAULT 0,
              y REAL NOT NULL DEFAULT 0,
              vx REAL NOT NULL DEFAULT 0,
              vy REAL NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL
            )
            ",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS movement_input_state (
              player_id TEXT PRIMARY KEY,
              up INTEGER NOT NULL DEFAULT 0,
              down INTEGER NOT NULL DEFAULT 0,
              left INTEGER NOT NULL DEFAULT 0,
              right INTEGER NOT NULL DEFAULT 0,
              last_input_seq INTEGER NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL
            )
            ",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS build_structures (
              structure_id TEXT PRIMARY KEY,
              owner_id TEXT NOT NULL,
              kind TEXT NOT NULL,
              x REAL NOT NULL,
              y REAL NOT NULL,
              grid_x INTEGER,
              grid_y INTEGER,
              created_at INTEGER NOT NULL
            )
            ",
            None,
        )?;

        if let Err(error) = sql.exec(
            "ALTER TABLE build_structures ADD COLUMN grid_x INTEGER",
            None,
        ) {
            let message = format!("{error}");
            if !message.contains("duplicate column") {
                return Err(error);
            }
        }

        if let Err(error) = sql.exec(
            "ALTER TABLE build_structures ADD COLUMN grid_y INTEGER",
            None,
        ) {
            let message = format!("{error}");
            if !message.contains("duplicate column") {
                return Err(error);
            }
        }

        sql.exec(
            "UPDATE build_structures SET grid_x = CAST(ROUND(x / ?) AS INTEGER), grid_y = CAST(ROUND(y / ?) AS INTEGER) WHERE grid_x IS NULL OR grid_y IS NULL",
            Some(vec![BUILD_GRID_SIZE.into(), BUILD_GRID_SIZE.into()]),
        )?;

        sql.exec(
            "
            DELETE FROM build_structures
            WHERE structure_id IN (
              SELECT older.structure_id
              FROM build_structures older
              JOIN build_structures newer
                ON older.grid_x = newer.grid_x
               AND older.grid_y = newer.grid_y
               AND older.created_at < newer.created_at
            )
            ",
            None,
        )?;

        sql.exec(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_build_structures_grid ON build_structures(grid_x, grid_y) WHERE grid_x IS NOT NULL AND grid_y IS NOT NULL",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS build_previews (
              player_id TEXT PRIMARY KEY,
              kind TEXT NOT NULL,
              x REAL NOT NULL,
              y REAL NOT NULL,
              grid_x INTEGER NOT NULL,
              grid_y INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            )
            ",
            None,
        )?;

        sql.exec(
            "CREATE INDEX IF NOT EXISTS idx_build_previews_updated_at ON build_previews(updated_at)",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS projectile_state (
              projectile_id TEXT PRIMARY KEY,
              owner_id TEXT NOT NULL,
              x REAL NOT NULL,
              y REAL NOT NULL,
              vx REAL NOT NULL,
              vy REAL NOT NULL,
              expires_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            )
            ",
            None,
        )?;

        if let Err(error) = sql.exec(
            "ALTER TABLE projectile_state ADD COLUMN client_projectile_id TEXT",
            None,
        ) {
            let message = format!("{error}");
            if !message.contains("duplicate column") {
                return Err(error);
            }
        }

        sql.exec(
            "DELETE FROM build_previews WHERE updated_at < ?",
            Some(vec![(now_ms() - BUILD_PREVIEW_STALE_MS).into()]),
        )?;

        sql.exec(
            "DELETE FROM session_tokens WHERE expires_at < ?",
            Some(vec![now_ms().into()]),
        )?;

        self.backfill_character_profiles_from_presence()?;

        Ok(())
    }

    fn load_room_meta_value(&self, key: &str) -> Result<Option<String>> {
        let rows: Vec<MetaValueRow> = self
            .sql()
            .exec(
                "SELECT value FROM room_meta WHERE key = ? LIMIT 1",
                Some(vec![key.into()]),
            )?
            .to_array()?;

        Ok(rows.first().map(|row| row.value.clone()))
    }

    fn persist_room_meta_value(&self, key: &str, value: &str) -> Result<()> {
        self.sql().exec(
            "INSERT INTO room_meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            Some(vec![key.into(), value.into()]),
        )?;
        Ok(())
    }

    fn load_room_code_from_db(&self) -> Result<Option<String>> {
        self.load_room_meta_value(ROOM_META_ROOM_CODE_KEY)
    }

    fn persist_room_code(&self, room_code: &str) -> Result<()> {
        self.persist_room_meta_value(ROOM_META_ROOM_CODE_KEY, room_code)
    }

    fn load_terrain_seed_from_db(&self) -> Result<Option<u64>> {
        let Some(raw_seed) = self.load_room_meta_value(ROOM_META_TERRAIN_SEED_KEY)? else {
            return Ok(None);
        };

        match raw_seed.parse::<u64>() {
            Ok(seed) => Ok(Some(seed)),
            Err(_) => Ok(None),
        }
    }

    fn persist_terrain_seed(&self, terrain_seed: u64) -> Result<()> {
        self.persist_room_meta_value(ROOM_META_TERRAIN_SEED_KEY, &terrain_seed.to_string())
    }

    fn ensure_terrain_seed(&self, room_code: &str) -> Result<u64> {
        if let Some(existing_seed) = self.load_terrain_seed_from_db()? {
            self.terrain_seed.set(existing_seed);
            return Ok(existing_seed);
        }

        let derived_seed = deterministic_seed_from_room_code(room_code);
        self.persist_terrain_seed(derived_seed)?;
        self.terrain_seed.set(derived_seed);
        Ok(derived_seed)
    }

    fn issue_resume_token(&self, player_id: &str, resume_token: Option<&str>) -> Result<String> {
        let now = now_ms();
        let expires_at = now + RESUME_TOKEN_TTL_MS;
        let sql = self.sql();

        if let Some(token) = resume_token {
            let rows: Vec<SessionTokenRow> = sql
                .exec(
                    "SELECT token, player_id, expires_at FROM session_tokens WHERE token = ? LIMIT 1",
                    Some(vec![token.into()]),
                )?
                .to_array()?;

            if let Some(row) = rows.first() {
                if row.player_id == player_id && row.expires_at > now {
                    sql.exec(
                        "UPDATE session_tokens SET expires_at = ?, updated_at = ? WHERE token = ?",
                        Some(vec![expires_at.into(), now.into(), token.into()]),
                    )?;
                    return Ok(row.token.clone());
                }
            }
        }

        let token = format!(
            "resume_{:x}_{:x}",
            now as u64,
            (js_sys::Math::random() * 1e12) as u64
        );
        sql.exec(
            "
            INSERT INTO session_tokens (token, player_id, expires_at, updated_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(token) DO UPDATE SET
              player_id = excluded.player_id,
              expires_at = excluded.expires_at,
              updated_at = excluded.updated_at
            ",
            Some(vec![
                token.as_str().into(),
                player_id.into(),
                expires_at.into(),
                now.into(),
            ]),
        )?;

        sql.exec(
            "DELETE FROM session_tokens WHERE token IN (
               SELECT token FROM session_tokens
               WHERE player_id = ?
               ORDER BY updated_at DESC
               LIMIT -1 OFFSET 8
             )",
            Some(vec![player_id.into()]),
        )?;

        sql.exec(
            "DELETE FROM session_tokens WHERE expires_at < ?",
            Some(vec![now.into()]),
        )?;

        Ok(token)
    }

    fn read_socket_attachment(&self, ws: &WebSocket) -> Option<SocketAttachment> {
        ws.deserialize_attachment::<SocketAttachment>()
            .ok()
            .flatten()
    }

    fn connected_player_ids(&self) -> Vec<String> {
        let mut ids = HashSet::new();
        for socket in self.state.get_websockets() {
            if let Some(attachment) = self.read_socket_attachment(&socket) {
                ids.insert(attachment.player_id);
            }
        }

        let mut sorted: Vec<String> = ids.into_iter().collect();
        sorted.sort();
        sorted
    }

    fn player_has_other_socket(&self, target_player_id: &str, excluding: &WebSocket) -> bool {
        for socket in self.state.get_websockets() {
            if socket == *excluding {
                continue;
            }

            if let Some(attachment) = self.read_socket_attachment(&socket) {
                if attachment.player_id == target_player_id {
                    return true;
                }
            }
        }

        false
    }

    fn send_envelope(
        &self,
        socket: &WebSocket,
        kind: &'static str,
        feature: &'static str,
        action: &'static str,
        seq: Option<u32>,
        payload: Option<Value>,
    ) {
        let envelope = ServerEnvelope {
            v: PROTOCOL_VERSION,
            kind,
            tick: self.tick.get(),
            server_time: now_ms(),
            feature,
            action,
            seq,
            payload,
        };

        let _ = socket.send(&envelope);
    }

    fn send_ack(&self, socket: &WebSocket, feature: &'static str, action: &'static str, seq: u32) {
        self.send_envelope(
            socket,
            "ack",
            feature,
            action,
            Some(seq),
            Some(json!({
                "serverTime": now_ms(),
            })),
        );
    }

    fn send_error(
        &self,
        socket: &WebSocket,
        feature: &'static str,
        action: &'static str,
        message: &str,
    ) {
        self.send_envelope(
            socket,
            "error",
            feature,
            action,
            None,
            Some(json!({
                "message": message,
            })),
        );
    }

    fn broadcast_envelope(
        &self,
        kind: &'static str,
        feature: &'static str,
        action: &'static str,
        payload: Option<Value>,
    ) {
        for socket in self.state.get_websockets() {
            self.send_envelope(&socket, kind, feature, action, None, payload.clone());
        }
    }

    fn on_connect_player(&self, player_id: &str) -> Result<()> {
        let now = now_ms();
        let sql = self.sql();

        sql.exec(
            "
            INSERT INTO presence_players (player_id, connected, last_seen)
            VALUES (?, 1, ?)
            ON CONFLICT(player_id) DO UPDATE SET connected = 1, last_seen = excluded.last_seen
            ",
            Some(vec![player_id.into(), now.into()]),
        )?;

        self.ensure_default_character_profile(player_id)?;

        {
            let mut runtime = self.runtime.borrow_mut();
            let player = runtime
                .players
                .entry(player_id.to_string())
                .or_insert_with(|| Self::default_runtime_player(now));
            // Input sequence numbers are connection-scoped. Reset on join so
            // reconnecting clients that start from seq=1 are accepted immediately.
            player.last_input_seq = 0;
            player.input = InputState {
                up: false,
                down: false,
                left: false,
                right: false,
            };
            player.vx = 0.0;
            player.vy = 0.0;
            player.connected = true;
            player.last_seen = now;
        }

        self.checkpoint_runtime_players_to_db()?;
        self.last_checkpoint_ms.set(now);

        self.snapshot_dirty.set(true);
        self.dirty_presence.set(true);
        Ok(())
    }

    fn on_disconnect_player(&self, player_id: &str) -> Result<()> {
        let now = now_ms();
        let sql = self.sql();

        sql.exec(
            "UPDATE presence_players SET connected = 0, last_seen = ? WHERE player_id = ?",
            Some(vec![now.into(), player_id.into()]),
        )?;

        {
            let mut runtime = self.runtime.borrow_mut();
            if let Some(player) = runtime.players.get_mut(player_id) {
                player.connected = false;
                player.last_seen = now;
            }
            runtime.previews.remove(player_id);
        }

        self.checkpoint_runtime_players_to_db()?;
        self.last_checkpoint_ms.set(now);

        self.snapshot_dirty.set(true);
        self.dirty_presence.set(true);
        self.dirty_build.set(true);
        Ok(())
    }

    fn handle_movement_input_batch(&self, player_id: &str, payload: Option<Value>) -> Result<bool> {
        let payload = payload.ok_or_else(|| Error::RustError("missing movement payload".into()))?;
        let input_batch: InputBatchPayload = serde_json::from_value(payload)
            .map_err(|_| Error::RustError("invalid movement payload".into()))?;

        if input_batch.inputs.len() > 128 {
            return Err(Error::RustError("input batch too large".into()));
        }

        if input_batch.inputs.is_empty() {
            return Ok(false);
        }

        let now = now_ms();
        let mut runtime = self.runtime.borrow_mut();
        let player = runtime
            .players
            .entry(player_id.to_string())
            .or_insert_with(|| Self::default_runtime_player(now));
        let mut last_seq = player.last_input_seq;
        let mut latest_state = player.input.clone();
        let mut accepted = false;

        for command in input_batch.inputs {
            if command.seq <= last_seq {
                continue;
            }

            last_seq = command.seq;
            accepted = true;
            latest_state = InputState {
                up: command.up,
                down: command.down,
                left: command.left,
                right: command.right,
            };
        }

        if accepted {
            player.last_input_seq = last_seq;
            player.input = latest_state;
            player.last_seen = now;
        }

        Ok(false)
    }

    fn prune_stale_build_previews(&self) -> Result<()> {
        let cutoff = now_ms() - BUILD_PREVIEW_STALE_MS;
        let mut runtime = self.runtime.borrow_mut();
        runtime
            .previews
            .retain(|_, preview| preview.updated_at >= cutoff);
        Ok(())
    }

    fn can_place_structure_at_cell(
        &self,
        grid_x: i64,
        grid_y: i64,
        center_x: f64,
        center_y: f64,
    ) -> Result<bool> {
        let runtime = self.runtime.borrow();
        if runtime
            .structures
            .values()
            .any(|structure| structure.grid_x == grid_x && structure.grid_y == grid_y)
        {
            return Ok(false);
        }

        let blocked = STRUCTURE_COLLIDER_HALF_EXTENT + PLAYER_COLLIDER_RADIUS;
        for player in runtime.players.values() {
            if !player.connected {
                continue;
            }

            if (player.x as f64 - center_x).abs() < blocked as f64
                && (player.y as f64 - center_y).abs() < blocked as f64
            {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn handle_build_preview(&self, player_id: &str, payload: Option<Value>) -> Result<bool> {
        let payload =
            payload.ok_or_else(|| Error::RustError("missing build preview payload".into()))?;
        let preview: BuildPreviewPayload = serde_json::from_value(payload)
            .map_err(|_| Error::RustError("invalid build preview payload".into()))?;
        let now = now_ms();

        {
            let mut runtime = self.runtime.borrow_mut();
            let player = runtime
                .players
                .entry(player_id.to_string())
                .or_insert_with(|| Self::default_runtime_player(now));

            if now - player.last_preview_cmd_at < PREVIEW_COMMAND_MIN_INTERVAL_MS {
                return Ok(false);
            }
            player.last_preview_cmd_at = now;
            player.last_seen = now;
        }

        if !preview.active {
            self.runtime.borrow_mut().previews.remove(player_id);
            self.prune_stale_build_previews()?;
            self.snapshot_dirty.set(true);
            self.dirty_build.set(true);
            return Ok(false);
        }

        let x = preview
            .x
            .ok_or_else(|| Error::RustError("build preview missing x".into()))?;
        let y = preview
            .y
            .ok_or_else(|| Error::RustError("build preview missing y".into()))?;
        let kind = preview
            .kind
            .as_deref()
            .ok_or_else(|| Error::RustError("build preview missing kind".into()))?;

        if !is_valid_structure_kind(kind) {
            return Err(Error::RustError("invalid structure kind".into()));
        }

        let center_x = grid_cell_center(snap_axis_to_grid(x));
        let center_y = grid_cell_center(snap_axis_to_grid(y));

        self.runtime.borrow_mut().previews.insert(
            player_id.to_string(),
            RuntimePreviewState {
                player_id: player_id.to_string(),
                kind: kind.to_string(),
                x: center_x as f32,
                y: center_y as f32,
                updated_at: now,
            },
        );

        self.prune_stale_build_previews()?;
        self.snapshot_dirty.set(true);
        self.dirty_build.set(true);
        Ok(false)
    }

    fn handle_build_command(
        &self,
        player_id: &str,
        action: &str,
        payload: Option<Value>,
    ) -> Result<bool> {
        match action {
            "place" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing build payload".into()))?;
                let place: BuildPlacePayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid build payload".into()))?;
                let now = now_ms();

                {
                    let mut runtime = self.runtime.borrow_mut();
                    let player = runtime
                        .players
                        .entry(player_id.to_string())
                        .or_insert_with(|| Self::default_runtime_player(now));

                    if now - player.last_place_cmd_at < PLACE_COMMAND_MIN_INTERVAL_MS {
                        return Ok(false);
                    }
                    player.last_place_cmd_at = now;
                    player.last_seen = now;
                }

                if !is_valid_structure_kind(place.kind.as_str()) {
                    return Err(Error::RustError("invalid structure kind".into()));
                }

                let grid_x = snap_axis_to_grid(place.x);
                let grid_y = snap_axis_to_grid(place.y);
                let snapped_x = grid_cell_center(grid_x);
                let snapped_y = grid_cell_center(grid_y);

                if !self.can_place_structure_at_cell(grid_x, grid_y, snapped_x, snapped_y)? {
                    return Err(Error::RustError("build cell is blocked".into()));
                }

                let structure_id = place
                    .client_build_id
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| format!("build_{}_{}", now_ms(), js_sys::Math::random()));

                let structure = RuntimeStructureState {
                    structure_id: structure_id.clone(),
                    owner_id: player_id.to_string(),
                    kind: place.kind.clone(),
                    x: snapped_x as f32,
                    y: snapped_y as f32,
                    grid_x,
                    grid_y,
                    chunk_x: chunk_coord_for_grid(grid_x),
                    chunk_y: chunk_coord_for_grid(grid_y),
                    created_at: now,
                };

                self.runtime
                    .borrow_mut()
                    .structures
                    .insert(structure_id.clone(), structure.clone());
                self.persist_structure_insert(&structure)?;

                let overflow_structure_id = {
                    let runtime = self.runtime.borrow();
                    if runtime.structures.len() > MAX_STRUCTURES {
                        runtime
                            .structures
                            .values()
                            .min_by_key(|value| value.created_at)
                            .map(|value| value.structure_id.clone())
                    } else {
                        None
                    }
                };

                if let Some(overflow_structure_id) = overflow_structure_id {
                    self.runtime
                        .borrow_mut()
                        .structures
                        .remove(&overflow_structure_id);
                    self.persist_structure_delete(&overflow_structure_id)?;
                }

                self.snapshot_dirty.set(true);
                self.dirty_build.set(true);
                Ok(true)
            }
            "preview" => self.handle_build_preview(player_id, payload),
            "remove" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing build payload".into()))?;
                let remove: BuildRemovePayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid build payload".into()))?;

                self.runtime.borrow_mut().structures.remove(&remove.id);
                self.persist_structure_delete(&remove.id)?;

                self.snapshot_dirty.set(true);
                self.dirty_build.set(true);
                Ok(true)
            }
            _ => Err(Error::RustError("invalid build action".into())),
        }
    }

    fn handle_projectile_fire(&self, player_id: &str, payload: Option<Value>) -> Result<bool> {
        let payload =
            payload.ok_or_else(|| Error::RustError("missing projectile payload".into()))?;
        let mut fire: ProjectileFirePayload = serde_json::from_value(payload)
            .map_err(|_| Error::RustError("invalid projectile payload".into()))?;
        let now = now_ms();

        {
            let mut runtime = self.runtime.borrow_mut();
            let player = runtime
                .players
                .entry(player_id.to_string())
                .or_insert_with(|| Self::default_runtime_player(now));

            if now - player.last_projectile_fire_at < PROJECTILE_FIRE_MIN_INTERVAL_MS {
                return Ok(false);
            }
            player.last_projectile_fire_at = now;
            player.last_seen = now;
        }

        let speed = (fire.vx * fire.vx + fire.vy * fire.vy).sqrt();
        if speed > PROJECTILE_MAX_SPEED && speed > 0.0 {
            let scale = PROJECTILE_MAX_SPEED / speed;
            fire.vx *= scale;
            fire.vy *= scale;
        }

        let projectile_id = format!("proj_{}_{}", now, js_sys::Math::random());
        let expires_at = now + PROJECTILE_TTL_MS;
        let updated_at = now;

        self.runtime.borrow_mut().projectiles.insert(
            projectile_id.clone(),
            RuntimeProjectileState {
                projectile_id,
                owner_id: player_id.to_string(),
                x: fire.x as f32,
                y: fire.y as f32,
                vx: fire.vx as f32,
                vy: fire.vy as f32,
                expires_at,
                client_projectile_id: fire.client_projectile_id,
                updated_at,
            },
        );

        let overflow_projectile_id = {
            let runtime = self.runtime.borrow();
            if runtime.projectiles.len() > MAX_PROJECTILES {
                runtime
                    .projectiles
                    .values()
                    .min_by_key(|projectile| projectile.updated_at)
                    .map(|projectile| projectile.projectile_id.clone())
            } else {
                None
            }
        };

        if let Some(overflow_projectile_id) = overflow_projectile_id {
            self.runtime
                .borrow_mut()
                .projectiles
                .remove(&overflow_projectile_id);
        }

        self.snapshot_dirty.set(true);
        self.dirty_projectiles.set(true);
        Ok(true)
    }

    fn handle_inventory_command(
        &self,
        player_id: &str,
        action: &str,
        payload: Option<Value>,
    ) -> Result<bool> {
        let now = now_ms();
        {
            let mut runtime = self.runtime.borrow_mut();
            let player = runtime
                .players
                .entry(player_id.to_string())
                .or_insert_with(|| Self::default_runtime_player(now));
            player.last_seen = now;
        }

        match action {
            "move" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing inventory payload".into()))?;
                let move_payload: InventoryMovePayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid inventory payload".into()))?;

                if move_payload.from_slot == move_payload.to_slot
                    || move_payload.from_slot >= DEFAULT_INVENTORY_MAX_SLOTS as u16
                    || move_payload.to_slot >= DEFAULT_INVENTORY_MAX_SLOTS as u16
                    || move_payload.amount.is_some_and(|amount| amount == 0)
                {
                    return Err(Error::RustError("invalid inventory move payload".into()));
                }

                Err(Error::RustError(
                    "inventory commands are not implemented".into(),
                ))
            }
            "split" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing inventory payload".into()))?;
                let split_payload: InventorySplitPayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid inventory payload".into()))?;

                if split_payload.slot >= DEFAULT_INVENTORY_MAX_SLOTS as u16
                    || split_payload.amount == 0
                {
                    return Err(Error::RustError("invalid inventory split payload".into()));
                }

                Err(Error::RustError(
                    "inventory commands are not implemented".into(),
                ))
            }
            _ => Err(Error::RustError("invalid inventory action".into())),
        }
    }

    fn handle_mining_command(
        &self,
        player_id: &str,
        action: &str,
        payload: Option<Value>,
    ) -> Result<bool> {
        let now = now_ms();
        {
            let mut runtime = self.runtime.borrow_mut();
            let player = runtime
                .players
                .entry(player_id.to_string())
                .or_insert_with(|| Self::default_runtime_player(now));
            player.last_seen = now;
        }

        match action {
            "start" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing mining payload".into()))?;
                let start_payload: MiningStartPayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid mining payload".into()))?;
                if !is_valid_protocol_identifier(start_payload.node_id.as_str()) {
                    return Err(Error::RustError("invalid mining node id".into()));
                }

                Err(Error::RustError(
                    "mining commands are not implemented".into(),
                ))
            }
            "cancel" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing mining payload".into()))?;
                let cancel_payload: MiningCancelPayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid mining payload".into()))?;

                if let Some(node_id) = cancel_payload.node_id.as_deref() {
                    if !is_valid_protocol_identifier(node_id) {
                        return Err(Error::RustError("invalid mining node id".into()));
                    }
                }

                Err(Error::RustError(
                    "mining commands are not implemented".into(),
                ))
            }
            _ => Err(Error::RustError("invalid mining action".into())),
        }
    }

    fn handle_crafting_command(
        &self,
        player_id: &str,
        action: &str,
        payload: Option<Value>,
    ) -> Result<bool> {
        let now = now_ms();
        {
            let mut runtime = self.runtime.borrow_mut();
            let player = runtime
                .players
                .entry(player_id.to_string())
                .or_insert_with(|| Self::default_runtime_player(now));
            player.last_seen = now;
        }

        match action {
            "queue" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing crafting payload".into()))?;
                let queue_payload: CraftQueuePayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid crafting payload".into()))?;

                if queue_payload.count == 0
                    || !is_valid_protocol_identifier(queue_payload.recipe.as_str())
                {
                    return Err(Error::RustError("invalid crafting queue payload".into()));
                }

                Err(Error::RustError(
                    "crafting commands are not implemented".into(),
                ))
            }
            "cancel" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing crafting payload".into()))?;
                let cancel_payload: CraftCancelPayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid crafting payload".into()))?;

                let clear_requested = cancel_payload.clear.unwrap_or(false);
                if !clear_requested && cancel_payload.recipe.is_none() {
                    return Err(Error::RustError("invalid crafting cancel payload".into()));
                }

                if let Some(recipe) = cancel_payload.recipe.as_deref() {
                    if !is_valid_protocol_identifier(recipe) {
                        return Err(Error::RustError("invalid crafting recipe id".into()));
                    }
                }

                Err(Error::RustError(
                    "crafting commands are not implemented".into(),
                ))
            }
            _ => Err(Error::RustError("invalid crafting action".into())),
        }
    }

    fn handle_combat_command(
        &self,
        player_id: &str,
        action: &str,
        payload: Option<Value>,
    ) -> Result<bool> {
        let now = now_ms();
        {
            let mut runtime = self.runtime.borrow_mut();
            let player = runtime
                .players
                .entry(player_id.to_string())
                .or_insert_with(|| Self::default_runtime_player(now));
            player.last_seen = now;
        }

        match action {
            "attack" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing combat payload".into()))?;
                let attack_payload: CombatAttackPayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid combat payload".into()))?;

                if !is_valid_protocol_identifier(attack_payload.target_id.as_str()) {
                    return Err(Error::RustError("invalid combat target id".into()));
                }

                if let Some(attack_id) = attack_payload.attack_id.as_deref() {
                    if !is_valid_protocol_identifier(attack_id) {
                        return Err(Error::RustError("invalid combat attack id".into()));
                    }
                }

                Err(Error::RustError(
                    "combat commands are not implemented".into(),
                ))
            }
            _ => Err(Error::RustError("invalid combat action".into())),
        }
    }

    fn handle_character_command(
        &self,
        player_id: &str,
        action: &str,
        payload: Option<Value>,
    ) -> Result<bool> {
        let now = now_ms();
        {
            let mut runtime = self.runtime.borrow_mut();
            let player = runtime
                .players
                .entry(player_id.to_string())
                .or_insert_with(|| Self::default_runtime_player(now));
            player.last_seen = now;
        }

        match action {
            "set_profile" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing character payload".into()))?;
                let profile_payload: CharacterMetadataPayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid character payload".into()))?;
                let name = sanitize_character_name(profile_payload.name.as_str())
                    .ok_or_else(|| Error::RustError("invalid character name".into()))?;
                let character_id = match profile_payload.character_id.as_deref() {
                    Some(raw_character_id) => sanitize_character_id(raw_character_id)
                        .ok_or_else(|| Error::RustError("invalid character id".into()))?,
                    None => DEFAULT_CHARACTER_PROFILE_ID.to_string(),
                };
                let sprite_id = profile_payload
                    .sprite_id
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or(DEFAULT_CHARACTER_SPRITE_ID);

                if !is_valid_protocol_identifier(sprite_id) {
                    return Err(Error::RustError("invalid character sprite id".into()));
                }

                self.upsert_character_profile(
                    player_id,
                    character_id.as_str(),
                    name.as_str(),
                    sprite_id,
                    profile_payload.set_active.unwrap_or(true),
                )?;
                self.snapshot_dirty.set(true);
                self.dirty_presence.set(true);
                Ok(true)
            }
            "set_active" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing character payload".into()))?;
                let select_payload: CharacterSelectPayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid character payload".into()))?;
                let character_id = sanitize_character_id(select_payload.character_id.as_str())
                    .ok_or_else(|| Error::RustError("invalid character id".into()))?;

                self.ensure_default_character_profile(player_id)?;
                self.set_active_character_profile(player_id, character_id.as_str())?;
                self.snapshot_dirty.set(true);
                self.dirty_presence.set(true);
                Ok(true)
            }
            _ => Err(Error::RustError("invalid character action".into())),
        }
    }

    fn run_simulation_until_now(&self) -> Result<()> {
        let now = now_ms() as f64;
        let elapsed = (now - self.last_loop_ms.get()).clamp(0.0, 250.0);
        self.last_loop_ms.set(now);

        let mut accumulator = self.accumulator_ms.get() + elapsed;
        let mut steps = 0usize;

        while accumulator >= SIM_DT_MS && steps < MAX_CATCHUP_STEPS {
            self.tick.set(self.tick.get().saturating_add(1));

            let connected_players = self.connected_player_ids();
            let movement_changed = self.tick_movement(&connected_players)?;
            let projectile_changed = self.tick_projectiles()?;

            if movement_changed || projectile_changed {
                self.snapshot_dirty.set(true);
                if projectile_changed {
                    self.dirty_projectiles.set(true);
                }
            }

            if self.tick.get() % SNAPSHOT_INTERVAL_TICKS == 0 || self.snapshot_dirty.get() {
                self.broadcast_snapshot(false);
                self.snapshot_dirty.set(false);
                self.dirty_presence.set(false);
                self.dirty_build.set(false);
                self.dirty_projectiles.set(false);
            }

            self.checkpoint_runtime_if_due()?;

            accumulator -= SIM_DT_MS;
            steps += 1;
        }

        if steps == MAX_CATCHUP_STEPS && accumulator >= SIM_DT_MS {
            accumulator = 0.0;
        }

        self.accumulator_ms.set(accumulator);
        Ok(())
    }

    fn tick_movement(&self, connected_players: &[String]) -> Result<bool> {
        if connected_players.is_empty() {
            return Ok(false);
        }

        let now = now_ms();
        let structure_obstacles: Vec<StructureObstacle> = {
            let runtime = self.runtime.borrow();
            runtime
                .structures
                .values()
                .map(|structure| StructureObstacle {
                    x: structure.x,
                    y: structure.y,
                    half_extent: structure_half_extent(structure.kind.as_str()),
                })
                .collect()
        };

        let mut changed = false;
        let mut runtime = self.runtime.borrow_mut();

        for player_id in connected_players {
            let player = runtime
                .players
                .entry(player_id.clone())
                .or_insert_with(|| Self::default_runtime_player(now));

            let step = movement_step_with_obstacles(
                player.x,
                player.y,
                map_input_to_core(&player.input),
                SIM_DT_SECONDS,
                MOVE_SPEED,
                MOVEMENT_MAP_LIMIT,
                &structure_obstacles,
                PLAYER_COLLIDER_RADIUS,
            );

            if (step.x - player.x).abs() > f32::EPSILON
                || (step.y - player.y).abs() > f32::EPSILON
                || (step.vx - player.vx).abs() > f32::EPSILON
                || (step.vy - player.vy).abs() > f32::EPSILON
            {
                changed = true;
            }

            player.x = step.x;
            player.y = step.y;
            player.vx = step.vx;
            player.vy = step.vy;
            player.connected = true;
            player.last_seen = now;
        }

        Ok(changed)
    }

    fn tick_projectiles(&self) -> Result<bool> {
        let now = now_ms();
        let mut runtime = self.runtime.borrow_mut();
        if runtime.projectiles.is_empty() {
            return Ok(false);
        }

        let mut changed = false;

        let projectile_ids: Vec<String> = runtime.projectiles.keys().cloned().collect();
        for projectile_id in projectile_ids {
            let Some(projectile) = runtime.projectiles.get_mut(&projectile_id) else {
                continue;
            };

            if projectile.expires_at <= now {
                runtime.projectiles.remove(&projectile_id);
                changed = true;
                continue;
            }

            let (next_x, next_y) = projectile_step(
                projectile.x,
                projectile.y,
                projectile.vx,
                projectile.vy,
                SIM_DT_SECONDS,
                PROJECTILE_MAP_LIMIT,
            );
            projectile.x = next_x;
            projectile.y = next_y;
            projectile.updated_at = now;
            changed = true;
        }

        Ok(changed)
    }

    fn snapshot_payload(&self, full: bool) -> Result<Value> {
        let connected_players = self.connected_player_ids();
        let connected_set: HashSet<&str> = connected_players.iter().map(String::as_str).collect();
        let online = connected_players.clone();
        let now = now_ms();

        self.prune_stale_build_previews()?;
        let runtime = self.runtime.borrow();

        let mut movement_players = Vec::new();
        for player_id in connected_players.iter() {
            if let Some(player) = runtime.players.get(player_id) {
                movement_players.push(json!({
                    "id": player_id,
                    "x": player.x,
                    "y": player.y,
                    "vx": player.vx,
                    "vy": player.vy,
                    "connected": true,
                }));
            }
        }

        let mut input_acks = JsonMap::new();
        for player_id in connected_players.iter() {
            if let Some(player) = runtime.players.get(player_id) {
                input_acks.insert(player_id.clone(), Value::from(player.last_input_seq));
            }
        }

        let mut structure_rows: Vec<&RuntimeStructureState> = runtime.structures.values().collect();
        structure_rows.sort_by_key(|row| std::cmp::Reverse(row.created_at));
        let structures: Vec<Value> = structure_rows
            .iter()
            .take(MAX_STRUCTURES)
            .map(|row| {
                json!({
                    "id": row.structure_id,
                    "ownerId": row.owner_id,
                    "kind": row.kind,
                    "x": row.x,
                    "y": row.y,
                    "chunkX": row.chunk_x,
                    "chunkY": row.chunk_y,
                })
            })
            .collect();

        let mut preview_rows: Vec<&RuntimePreviewState> = runtime
            .previews
            .iter()
            .map(|(_, row)| row)
            .filter(|row| connected_set.contains(row.player_id.as_str()))
            .filter(|row| row.updated_at > now - BUILD_PREVIEW_STALE_MS)
            .collect();
        preview_rows.sort_by_key(|row| std::cmp::Reverse(row.updated_at));

        let previews: Vec<Value> = preview_rows
            .iter()
            .take(MAX_PREVIEWS)
            .map(|row| {
                json!({
                    "playerId": row.player_id,
                    "kind": row.kind,
                    "x": row.x,
                    "y": row.y,
                })
            })
            .collect();

        let mut projectile_rows: Vec<&RuntimeProjectileState> = runtime
            .projectiles
            .iter()
            .map(|(_, row)| row)
            .filter(|row| row.expires_at > now)
            .collect();
        projectile_rows.sort_by_key(|row| std::cmp::Reverse(row.updated_at));

        let projectiles: Vec<Value> = projectile_rows
            .iter()
            .take(MAX_PROJECTILES)
            .map(|row| {
                json!({
                    "id": row.projectile_id,
                    "ownerId": row.owner_id,
                    "x": row.x,
                    "y": row.y,
                    "vx": row.vx,
                    "vy": row.vy,
                    "clientProjectileId": row.client_projectile_id,
                })
            })
            .collect();

        let mut character_profiles = Vec::with_capacity(connected_players.len());
        for player_id in connected_players.iter() {
            let profile = self.load_active_character_profile(player_id)?;
            character_profiles.push(json!({
                "playerId": player_id,
                "characterId": profile.character_id,
                "name": profile.name,
                "spriteId": profile.sprite_id,
            }));
        }

        let include_presence = full || self.dirty_presence.get();
        let include_build = full || self.dirty_build.get();
        let include_projectiles = full || self.dirty_projectiles.get();
        let include_inventory = full;
        let include_mining = full;
        let include_crafting = full;
        let include_combat = full;
        let include_character = full || self.dirty_presence.get();
        let mut features = JsonMap::new();

        if include_presence {
            features.insert(
                "presence".to_string(),
                json!({
                    "online": online,
                    "onlineCount": connected_players.len(),
                }),
            );
        }

        // Movement is always emitted so clients can keep interpolation/prediction alive.
        features.insert(
            "movement".to_string(),
            json!({
                "players": movement_players,
                "inputAcks": input_acks,
                "speed": MOVE_SPEED,
            }),
        );

        features.insert(
            "terrain".to_string(),
            json!({
                "seed": self.terrain_seed.get().to_string(),
                "generatorVersion": TERRAIN_GENERATOR_VERSION,
                "tileSize": TERRAIN_TILE_SIZE,
            }),
        );

        if include_build {
            features.insert(
                "build".to_string(),
                json!({
                    "structures": structures,
                    "structureCount": runtime.structures.len(),
                    "previews": previews,
                    "previewCount": preview_rows.len().min(MAX_PREVIEWS),
                }),
            );
        }

        if include_projectiles {
            features.insert(
                "projectile".to_string(),
                json!({
                    "projectiles": projectiles,
                    "projectileCount": projectile_rows.len().min(MAX_PROJECTILES),
                }),
            );
        }

        if include_inventory {
            features.insert(
                "inventory".to_string(),
                json!({
                    "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
                    "revision": self.tick.get(),
                    "players": [],
                    "playerCount": 0,
                }),
            );
        }

        if include_mining {
            features.insert(
                "mining".to_string(),
                json!({
                    "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
                    "nodes": [],
                    "nodeCount": 0,
                    "active": [],
                    "activeCount": 0,
                }),
            );
        }

        if include_crafting {
            features.insert(
                "crafting".to_string(),
                json!({
                    "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
                    "queues": [],
                    "queueCount": 0,
                }),
            );
        }

        if include_combat {
            features.insert(
                "combat".to_string(),
                json!({
                    "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
                    "enemies": [],
                    "enemyCount": 0,
                    "players": [],
                    "playerCount": 0,
                }),
            );
        }

        if include_character {
            features.insert(
                "character".to_string(),
                json!({
                    "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
                    "players": character_profiles,
                    "playerCount": connected_players.len(),
                }),
            );
        }

        Ok(json!({
            "roomCode": self.room_code.borrow().clone(),
            "serverTick": self.tick.get(),
            "simRateHz": SIM_RATE_HZ,
            "snapshotRateHz": SNAPSHOT_RATE_HZ,
            "serverTime": now_ms(),
            "mode": if full { "full" } else { "delta" },
            "features": features,
        }))
    }

    fn send_snapshot_to(&self, socket: &WebSocket, full: bool) {
        if let Ok(payload) = self.snapshot_payload(full) {
            self.send_envelope(socket, "snapshot", "core", "state", None, Some(payload));
        }
    }

    fn broadcast_snapshot(&self, full: bool) {
        if let Ok(payload) = self.snapshot_payload(full) {
            self.broadcast_envelope("snapshot", "core", "state", Some(payload));
        }
    }

    fn parse_client_message(
        &self,
        message: WebSocketIncomingMessage,
    ) -> Result<ClientCommandEnvelope> {
        let raw = match message {
            WebSocketIncomingMessage::String(text) => text,
            WebSocketIncomingMessage::Binary(_) => {
                return Err(Error::RustError(
                    "binary websocket payloads are not supported".into(),
                ));
            }
        };

        if raw.len() > 32 * 1024 {
            return Err(Error::RustError("protocol envelope too large".into()));
        }

        let envelope: ClientCommandEnvelope = serde_json::from_str(&raw)
            .map_err(|_| Error::RustError("malformed protocol envelope".into()))?;

        if envelope.v != PROTOCOL_VERSION
            || envelope.kind != ClientEnvelopeKind::Command
            || envelope.seq < 1
            || envelope.action.is_empty()
            || envelope.action.len() > 32
            || !is_valid_protocol_identifier(envelope.action.as_str())
            || !envelope.client_time.is_finite()
        {
            return Err(Error::RustError("invalid protocol envelope".into()));
        }

        Ok(envelope)
    }

    fn apply_command(
        &self,
        socket: &WebSocket,
        player_id: &str,
        envelope: &ClientCommandEnvelope,
    ) -> Result<bool> {
        match envelope.feature {
            ProtocolFeature::Core => match envelope.action.as_str() {
                "ping" => {
                    self.send_envelope(
                        socket,
                        "pong",
                        "core",
                        "pong",
                        Some(envelope.seq),
                        Some(json!({
                            "clientTime": envelope.client_time,
                        })),
                    );
                    Ok(false)
                }
                _ => Err(Error::RustError("invalid core action".into())),
            },
            ProtocolFeature::Movement => match envelope.action.as_str() {
                "input_batch" => {
                    self.handle_movement_input_batch(player_id, envelope.payload.clone())
                }
                _ => Err(Error::RustError("invalid movement action".into())),
            },
            ProtocolFeature::Build => self.handle_build_command(
                player_id,
                envelope.action.as_str(),
                envelope.payload.clone(),
            ),
            ProtocolFeature::Projectile => match envelope.action.as_str() {
                "fire" => self.handle_projectile_fire(player_id, envelope.payload.clone()),
                _ => Err(Error::RustError("invalid projectile action".into())),
            },
            ProtocolFeature::Inventory => self.handle_inventory_command(
                player_id,
                envelope.action.as_str(),
                envelope.payload.clone(),
            ),
            ProtocolFeature::Mining => self.handle_mining_command(
                player_id,
                envelope.action.as_str(),
                envelope.payload.clone(),
            ),
            ProtocolFeature::Crafting => self.handle_crafting_command(
                player_id,
                envelope.action.as_str(),
                envelope.payload.clone(),
            ),
            ProtocolFeature::Combat => self.handle_combat_command(
                player_id,
                envelope.action.as_str(),
                envelope.payload.clone(),
            ),
            ProtocolFeature::Character => self.handle_character_command(
                player_id,
                envelope.action.as_str(),
                envelope.payload.clone(),
            ),
        }
    }

    fn restore_presence_from_active_sockets(&self) -> Result<()> {
        let players = self.connected_player_ids();
        if players.is_empty() {
            return Ok(());
        }

        let sql = self.sql();
        let now = now_ms();
        {
            let mut runtime = self.runtime.borrow_mut();
            for player_id in players.iter() {
                let player = runtime
                    .players
                    .entry(player_id.clone())
                    .or_insert_with(|| Self::default_runtime_player(now));
                player.connected = true;
                player.last_seen = now;
            }
        }
        for player_id in players {
            sql.exec(
                "
                INSERT INTO presence_players (player_id, connected, last_seen)
                VALUES (?, 1, ?)
                ON CONFLICT(player_id) DO UPDATE SET connected = 1, last_seen = excluded.last_seen
                ",
                Some(vec![player_id.into(), now.into()]),
            )?;
        }

        Ok(())
    }
}

impl DurableObject for RoomDurableObject {
    fn new(state: State, env: Env) -> Self {
        let room = Self {
            state,
            env,
            room_code: RefCell::new("UNKNOWN".to_string()),
            tick: Cell::new(0),
            last_loop_ms: Cell::new(now_ms() as f64),
            accumulator_ms: Cell::new(0.0),
            last_checkpoint_ms: Cell::new(now_ms()),
            snapshot_dirty: Cell::new(false),
            dirty_presence: Cell::new(false),
            dirty_build: Cell::new(false),
            dirty_projectiles: Cell::new(false),
            terrain_seed: Cell::new(deterministic_seed_from_room_code("UNKNOWN")),
            runtime: RefCell::new(RoomRuntimeState::default()),
        };

        if let Err(error) = room.initialize_schema() {
            console_error!("failed to initialize schema: {error}");
        }

        if let Ok(Some(room_code)) = room.load_room_code_from_db() {
            room.room_code.replace(room_code);
        }

        let startup_room_code = room.room_code.borrow().clone();
        if startup_room_code != "UNKNOWN" {
            if let Err(error) = room.ensure_terrain_seed(startup_room_code.as_str()) {
                console_error!("failed to ensure terrain seed: {error}");
            }
        } else if let Ok(Some(seed)) = room.load_terrain_seed_from_db() {
            room.terrain_seed.set(seed);
        }

        if let Err(error) = room.hydrate_runtime_from_db() {
            console_error!("failed to hydrate runtime state: {error}");
        }

        if let Err(error) = room.restore_presence_from_active_sockets() {
            console_error!("failed to restore presence: {error}");
        }

        room
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        let url = req.url()?;
        let room_code = parse_room_code_from_path(url.path())
            .ok_or_else(|| Error::RustError("invalid room endpoint".into()))?;

        self.room_code.replace(room_code.clone());
        self.persist_room_code(&room_code)?;
        self.ensure_terrain_seed(&room_code)?;

        let upgrade = req
            .headers()
            .get("Upgrade")?
            .unwrap_or_default()
            .to_ascii_lowercase();

        if upgrade != "websocket" {
            return json_response(json!({ "error": "WebSocket upgrade required." }), 426);
        }

        let resume_token_hint =
            parse_query_param(&url, "resumeToken").or_else(|| parse_query_param(&url, "resume"));
        let player_id = authenticate_player(&url, &self.env).await?;
        let resume_token = self.issue_resume_token(&player_id, resume_token_hint.as_deref())?;

        let pair = WebSocketPair::new()?;
        let server = pair.server;
        let client = pair.client;

        self.state
            .accept_websocket_with_tags(&server, &[player_id.as_str()]);

        server.serialize_attachment(SocketAttachment {
            player_id: player_id.clone(),
            last_seq: 0,
        })?;

        self.on_connect_player(&player_id)?;

        self.send_envelope(
            &server,
            "welcome",
            "core",
            "connected",
            None,
            Some(json!({
                "roomCode": room_code,
                "playerId": player_id,
                "simRateHz": SIM_RATE_HZ,
                "snapshotRateHz": SNAPSHOT_RATE_HZ,
                "resumeToken": resume_token,
            })),
        );

        self.send_snapshot_to(&server, true);
        self.broadcast_snapshot(false);

        Response::from_websocket(client)
    }

    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        self.run_simulation_until_now()?;

        let mut attachment = match self.read_socket_attachment(&ws) {
            Some(attachment) => attachment,
            None => return Ok(()),
        };

        let envelope = match self.parse_client_message(message) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.send_error(&ws, "core", "invalid_message", &format!("{error}"));
                return Ok(());
            }
        };

        if envelope.seq <= attachment.last_seq {
            self.send_ack(&ws, "core", "duplicate", envelope.seq);
            return Ok(());
        }

        attachment.last_seq = envelope.seq;
        ws.serialize_attachment(attachment.clone())?;

        match self.apply_command(&ws, &attachment.player_id, &envelope) {
            Ok(state_changed) => {
                self.send_ack(&ws, "core", "command", envelope.seq);
                if state_changed {
                    self.snapshot_dirty.set(true);
                    self.broadcast_snapshot(false);
                    self.snapshot_dirty.set(false);
                    self.dirty_presence.set(false);
                    self.dirty_build.set(false);
                    self.dirty_projectiles.set(false);
                }
            }
            Err(error) => {
                self.send_error(&ws, "core", "command_rejected", &format!("{error}"));
                self.send_ack(&ws, "core", "command", envelope.seq);
            }
        }

        Ok(())
    }

    async fn websocket_close(
        &self,
        ws: WebSocket,
        _code: usize,
        _reason: String,
        _was_clean: bool,
    ) -> Result<()> {
        if let Some(attachment) = self.read_socket_attachment(&ws) {
            if !self.player_has_other_socket(&attachment.player_id, &ws) {
                self.on_disconnect_player(&attachment.player_id)?;
                self.broadcast_snapshot(false);
            }
        }

        Ok(())
    }

    async fn websocket_error(&self, ws: WebSocket, _error: Error) -> Result<()> {
        if let Some(attachment) = self.read_socket_attachment(&ws) {
            if !self.player_has_other_socket(&attachment.player_id, &ws) {
                self.on_disconnect_player(&attachment.player_id)?;
                self.broadcast_snapshot(false);
            }
        }

        Ok(())
    }
}

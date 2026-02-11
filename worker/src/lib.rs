use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value};
use sim_core::{movement_step, projectile_step, InputState as CoreInputState};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use worker::durable::{DurableObject, State, WebSocketIncomingMessage};
use worker::*;

const PROTOCOL_VERSION: u32 = 2;
const PLAYER_ID_RE_MIN: usize = 3;
const PLAYER_ID_RE_MAX: usize = 120;
const ROOM_RE_MAX: usize = 24;

const SIM_RATE_HZ: u32 = 60;
const SNAPSHOT_RATE_HZ: u32 = 20;
const SIM_DT_SECONDS: f32 = 1.0 / SIM_RATE_HZ as f32;
const SIM_DT_MS: f64 = 1000.0 / SIM_RATE_HZ as f64;
const SNAPSHOT_INTERVAL_TICKS: u64 = (SIM_RATE_HZ / SNAPSHOT_RATE_HZ) as u64;
const MAX_CATCHUP_STEPS: usize = 8;

const MOVE_SPEED: f32 = 220.0;
const MOVEMENT_MAP_LIMIT: f32 = 5000.0;
const PROJECTILE_MAP_LIMIT: f32 = 5500.0;
const PROJECTILE_TTL_MS: i64 = 1800;
const PROJECTILE_MAX_SPEED: f64 = 900.0;

const MAX_STRUCTURES: usize = 1024;
const MAX_PROJECTILES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SocketAttachment {
    player_id: String,
    last_seq: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientCommandEnvelope {
    v: u32,
    kind: String,
    seq: u32,
    feature: String,
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
struct ProjectileFirePayload {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    client_projectile_id: Option<String>,
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
struct PresenceRow {
    player_id: String,
}

#[derive(Debug, Deserialize)]
struct MovementStateRow {
    player_id: String,
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
}

#[derive(Debug, Deserialize)]
struct MovementInputRow {
    up: i64,
    down: i64,
    left: i64,
    right: i64,
    last_input_seq: i64,
}

#[derive(Debug, Deserialize)]
struct MovementInputRowWithPlayerId {
    player_id: String,
    last_input_seq: i64,
}

#[derive(Debug, Deserialize)]
struct BuildRow {
    structure_id: String,
    owner_id: String,
    kind: String,
    x: f64,
    y: f64,
}

#[derive(Debug, Deserialize)]
struct ProjectileRow {
    projectile_id: String,
    owner_id: String,
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    expires_at: i64,
    client_projectile_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RoomCodeRow {
    value: String,
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

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let url = req.url()?;

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
        return assets.fetch_request(req).await;
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
    snapshot_dirty: Cell<bool>,
}

impl RoomDurableObject {
    fn sql(&self) -> SqlStorage {
        self.state.storage().sql()
    }

    fn initialize_schema(&self) -> Result<()> {
        let sql = self.sql();

        sql.exec(
            "CREATE TABLE IF NOT EXISTS room_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
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
              created_at INTEGER NOT NULL
            )
            ",
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

        Ok(())
    }

    fn load_room_code_from_db(&self) -> Result<Option<String>> {
        let sql = self.sql();
        let rows: Vec<RoomCodeRow> = sql
            .exec(
                "SELECT value FROM room_meta WHERE key = 'room_code' LIMIT 1",
                None,
            )?
            .to_array()?;

        Ok(rows.first().map(|row| row.value.clone()))
    }

    fn persist_room_code(&self, room_code: &str) -> Result<()> {
        let sql = self.sql();
        sql.exec(
            "INSERT INTO room_meta (key, value) VALUES ('room_code', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            Some(vec![room_code.into()]),
        )?;
        Ok(())
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

        ids.into_iter().collect()
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

        sql.exec(
            "
            INSERT INTO movement_state (player_id, x, y, vx, vy, updated_at)
            VALUES (?, 0, 0, 0, 0, ?)
            ON CONFLICT(player_id) DO UPDATE SET updated_at = excluded.updated_at
            ",
            Some(vec![player_id.into(), now.into()]),
        )?;

        sql.exec(
            "
            INSERT INTO movement_input_state (player_id, up, down, left, right, last_input_seq, updated_at)
            VALUES (?, 0, 0, 0, 0, 0, ?)
            ON CONFLICT(player_id) DO UPDATE SET updated_at = excluded.updated_at
            ",
            Some(vec![player_id.into(), now.into()]),
        )?;

        self.snapshot_dirty.set(true);
        Ok(())
    }

    fn on_disconnect_player(&self, player_id: &str) -> Result<()> {
        let now = now_ms();
        self.sql().exec(
            "UPDATE presence_players SET connected = 0, last_seen = ? WHERE player_id = ?",
            Some(vec![now.into(), player_id.into()]),
        )?;
        self.snapshot_dirty.set(true);
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

        let sql = self.sql();

        let existing: Vec<MovementInputRow> = sql
            .exec(
                "SELECT up, down, left, right, last_input_seq FROM movement_input_state WHERE player_id = ? LIMIT 1",
                Some(vec![player_id.into()]),
            )?
            .to_array()?;

        let mut last_seq = existing
            .first()
            .map(|row| row.last_input_seq.max(0) as u32)
            .unwrap_or(0);

        let mut latest_state = existing
            .first()
            .map(|row| InputState {
                up: row.up != 0,
                down: row.down != 0,
                left: row.left != 0,
                right: row.right != 0,
            })
            .unwrap_or(InputState {
                up: false,
                down: false,
                left: false,
                right: false,
            });

        for command in input_batch.inputs {
            if command.seq <= last_seq {
                continue;
            }

            last_seq = command.seq;
            latest_state = InputState {
                up: command.up,
                down: command.down,
                left: command.left,
                right: command.right,
            };
        }

        let now = now_ms();
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
                player_id.into(),
                (latest_state.up as i64).into(),
                (latest_state.down as i64).into(),
                (latest_state.left as i64).into(),
                (latest_state.right as i64).into(),
                (last_seq as i64).into(),
                now.into(),
            ]),
        )?;

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

                if !matches!(place.kind.as_str(), "beacon" | "miner" | "assembler") {
                    return Err(Error::RustError("invalid structure kind".into()));
                }

                let structure_id = place
                    .client_build_id
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| format!("build_{}_{}", now_ms(), js_sys::Math::random()));

                self.sql().exec(
                    "
                    INSERT INTO build_structures (structure_id, owner_id, kind, x, y, created_at)
                    VALUES (?, ?, ?, ?, ?, ?)
                    ON CONFLICT(structure_id) DO NOTHING
                    ",
                    Some(vec![
                        structure_id.into(),
                        player_id.into(),
                        place.kind.into(),
                        place.x.into(),
                        place.y.into(),
                        now_ms().into(),
                    ]),
                )?;

                self.snapshot_dirty.set(true);
                Ok(true)
            }
            "remove" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing build payload".into()))?;
                let remove: BuildRemovePayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid build payload".into()))?;

                self.sql().exec(
                    "DELETE FROM build_structures WHERE structure_id = ?",
                    Some(vec![remove.id.into()]),
                )?;

                self.snapshot_dirty.set(true);
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

        let speed = (fire.vx * fire.vx + fire.vy * fire.vy).sqrt();
        if speed > PROJECTILE_MAX_SPEED && speed > 0.0 {
            let scale = PROJECTILE_MAX_SPEED / speed;
            fire.vx *= scale;
            fire.vy *= scale;
        }

        let projectile_id = format!("proj_{}_{}", now_ms(), js_sys::Math::random());
        let expires_at = now_ms() + PROJECTILE_TTL_MS;

        self.sql().exec(
            "
            INSERT INTO projectile_state (projectile_id, owner_id, x, y, vx, vy, expires_at, updated_at, client_projectile_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
            Some(vec![
                projectile_id.into(),
                player_id.into(),
                fire.x.into(),
                fire.y.into(),
                fire.vx.into(),
                fire.vy.into(),
                expires_at.into(),
                now_ms().into(),
                fire.client_projectile_id.into(),
            ]),
        )?;

        self.sql().exec(
            "
            DELETE FROM projectile_state
            WHERE projectile_id IN (
              SELECT projectile_id
              FROM projectile_state
              ORDER BY updated_at ASC
              LIMIT (SELECT MAX(0, COUNT(*) - ?) FROM projectile_state)
            )
            ",
            Some(vec![(MAX_PROJECTILES as i64).into()]),
        )?;

        self.snapshot_dirty.set(true);
        Ok(true)
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
            }

            if self.tick.get() % SNAPSHOT_INTERVAL_TICKS == 0 || self.snapshot_dirty.get() {
                self.broadcast_snapshot();
                self.snapshot_dirty.set(false);
            }

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

        let sql = self.sql();
        let now = now_ms();

        for player_id in connected_players {
            let input_rows: Vec<MovementInputRow> = sql
                .exec(
                    "SELECT up, down, left, right, last_input_seq FROM movement_input_state WHERE player_id = ? LIMIT 1",
                    Some(vec![player_id.as_str().into()]),
                )?
                .to_array()?;

            let input = input_rows
                .first()
                .map(|row| InputState {
                    up: row.up != 0,
                    down: row.down != 0,
                    left: row.left != 0,
                    right: row.right != 0,
                })
                .unwrap_or(InputState {
                    up: false,
                    down: false,
                    left: false,
                    right: false,
                });

            let state_rows: Vec<MovementStateRow> = sql
                .exec(
                    "SELECT player_id, x, y, vx, vy FROM movement_state WHERE player_id = ? LIMIT 1",
                    Some(vec![player_id.as_str().into()]),
                )?
                .to_array()?;

            let (current_x, current_y) = state_rows
                .first()
                .map(|row| (row.x, row.y))
                .unwrap_or((0.0, 0.0));

            let step = movement_step(
                current_x as f32,
                current_y as f32,
                map_input_to_core(&input),
                SIM_DT_SECONDS,
                MOVE_SPEED,
                MOVEMENT_MAP_LIMIT,
            );

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
                    (step.x as f64).into(),
                    (step.y as f64).into(),
                    (step.vx as f64).into(),
                    (step.vy as f64).into(),
                    now.into(),
                ]),
            )?;
        }

        Ok(true)
    }

    fn tick_projectiles(&self) -> Result<bool> {
        let sql = self.sql();
        let now = now_ms();

        let projectiles: Vec<ProjectileRow> = sql
            .exec(
                "
                SELECT projectile_id, owner_id, x, y, vx, vy, expires_at, client_projectile_id
                FROM projectile_state
                ",
                None,
            )?
            .to_array()?;

        if projectiles.is_empty() {
            return Ok(false);
        }

        let mut changed = false;

        for projectile in projectiles {
            if projectile.expires_at <= now {
                sql.exec(
                    "DELETE FROM projectile_state WHERE projectile_id = ?",
                    Some(vec![projectile.projectile_id.into()]),
                )?;
                changed = true;
                continue;
            }

            let (next_x, next_y) = projectile_step(
                projectile.x as f32,
                projectile.y as f32,
                projectile.vx as f32,
                projectile.vy as f32,
                SIM_DT_SECONDS,
                PROJECTILE_MAP_LIMIT,
            );

            sql.exec(
                "UPDATE projectile_state SET x = ?, y = ?, updated_at = ? WHERE projectile_id = ?",
                Some(vec![
                    (next_x as f64).into(),
                    (next_y as f64).into(),
                    now.into(),
                    projectile.projectile_id.into(),
                ]),
            )?;
            changed = true;
        }

        Ok(changed)
    }

    fn snapshot_payload(&self) -> Result<Value> {
        let sql = self.sql();
        let connected_players = self.connected_player_ids();
        let connected_set: HashSet<&str> = connected_players.iter().map(String::as_str).collect();

        let presence_rows: Vec<PresenceRow> = sql
            .exec(
                "SELECT player_id FROM presence_players WHERE connected = 1 ORDER BY last_seen DESC",
                None,
            )?
            .to_array()?;

        let online: Vec<String> = presence_rows.into_iter().map(|row| row.player_id).collect();

        let movement_rows: Vec<MovementStateRow> = sql
            .exec(
                "SELECT player_id, x, y, vx, vy FROM movement_state ORDER BY player_id ASC",
                None,
            )?
            .to_array()?;

        let movement_players: Vec<Value> = movement_rows
            .into_iter()
            .filter(|row| connected_set.contains(row.player_id.as_str()))
            .map(|row| {
                json!({
                    "id": row.player_id,
                    "x": row.x,
                    "y": row.y,
                    "vx": row.vx,
                    "vy": row.vy,
                    "connected": true,
                })
            })
            .collect();

        let input_rows: Vec<MovementInputRowWithPlayerId> = sql
            .exec(
                "SELECT player_id, last_input_seq FROM movement_input_state ORDER BY player_id ASC",
                None,
            )?
            .to_array()?;

        let mut input_acks = JsonMap::new();
        for row in input_rows {
            input_acks.insert(row.player_id, Value::from(row.last_input_seq.max(0)));
        }

        let structure_rows: Vec<BuildRow> = sql
            .exec(
                "
                SELECT structure_id, owner_id, kind, x, y
                FROM build_structures
                ORDER BY created_at DESC
                LIMIT ?
                ",
                Some(vec![(MAX_STRUCTURES as i64).into()]),
            )?
            .to_array()?;

        let structures: Vec<Value> = structure_rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.structure_id,
                    "ownerId": row.owner_id,
                    "kind": row.kind,
                    "x": row.x,
                    "y": row.y,
                })
            })
            .collect();

        let projectile_rows: Vec<ProjectileRow> = sql
            .exec(
                "
                SELECT projectile_id, owner_id, x, y, vx, vy, expires_at, client_projectile_id
                FROM projectile_state
                WHERE expires_at > ?
                ORDER BY updated_at DESC
                LIMIT ?
                ",
                Some(vec![now_ms().into(), (MAX_PROJECTILES as i64).into()]),
            )?
            .to_array()?;

        let projectiles: Vec<Value> = projectile_rows
            .iter()
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

        Ok(json!({
            "roomCode": self.room_code.borrow().clone(),
            "serverTick": self.tick.get(),
            "simRateHz": SIM_RATE_HZ,
            "snapshotRateHz": SNAPSHOT_RATE_HZ,
            "serverTime": now_ms(),
            "features": {
                "presence": {
                    "online": online,
                    "onlineCount": connected_players.len(),
                },
                "movement": {
                    "players": movement_players,
                    "inputAcks": input_acks,
                    "speed": MOVE_SPEED,
                },
                "build": {
                    "structures": structures,
                    "structureCount": structure_rows.len(),
                },
                "projectile": {
                    "projectiles": projectiles,
                    "projectileCount": projectile_rows.len(),
                },
            }
        }))
    }

    fn send_snapshot_to(&self, socket: &WebSocket) {
        if let Ok(payload) = self.snapshot_payload() {
            self.send_envelope(socket, "snapshot", "core", "state", None, Some(payload));
        }
    }

    fn broadcast_snapshot(&self) {
        if let Ok(payload) = self.snapshot_payload() {
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

        let envelope: ClientCommandEnvelope = serde_json::from_str(&raw)
            .map_err(|_| Error::RustError("malformed protocol envelope".into()))?;

        if envelope.v != PROTOCOL_VERSION
            || envelope.kind != "command"
            || envelope.seq < 1
            || envelope.feature.is_empty()
            || envelope.action.is_empty()
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
        match (envelope.feature.as_str(), envelope.action.as_str()) {
            ("core", "ping") => {
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
            ("movement", "input_batch") => {
                self.handle_movement_input_batch(player_id, envelope.payload.clone())
            }
            ("build", action) => {
                self.handle_build_command(player_id, action, envelope.payload.clone())
            }
            ("projectile", "fire") => {
                self.handle_projectile_fire(player_id, envelope.payload.clone())
            }
            _ => Err(Error::RustError("unknown feature/action".into())),
        }
    }

    fn restore_presence_from_active_sockets(&self) -> Result<()> {
        let players = self.connected_player_ids();
        if players.is_empty() {
            return Ok(());
        }

        let sql = self.sql();
        let now = now_ms();
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
            snapshot_dirty: Cell::new(false),
        };

        if let Err(error) = room.initialize_schema() {
            console_error!("failed to initialize schema: {error}");
        }

        if let Ok(Some(room_code)) = room.load_room_code_from_db() {
            room.room_code.replace(room_code);
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

        let upgrade = req
            .headers()
            .get("Upgrade")?
            .unwrap_or_default()
            .to_ascii_lowercase();

        if upgrade != "websocket" {
            return json_response(json!({ "error": "WebSocket upgrade required." }), 426);
        }

        let player_id = authenticate_player(&url, &self.env).await?;

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
            })),
        );

        self.send_snapshot_to(&server);
        self.broadcast_snapshot();

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
                    self.broadcast_snapshot();
                    self.snapshot_dirty.set(false);
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
                self.broadcast_snapshot();
            }
        }

        Ok(())
    }

    async fn websocket_error(&self, ws: WebSocket, _error: Error) -> Result<()> {
        if let Some(attachment) = self.read_socket_attachment(&ws) {
            if !self.player_has_other_socket(&attachment.player_id, &ws) {
                self.on_disconnect_player(&attachment.player_id)?;
                self.broadcast_snapshot();
            }
        }

        Ok(())
    }
}

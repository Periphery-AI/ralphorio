use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value};
use sim_core::domain::{
    recipe_definition, RecipeKind, ResourceKind, DEFAULT_INVENTORY_MAX_SLOTS,
    GAMEPLAY_SCHEMA_VERSION,
};
use sim_core::{
    deterministic_seed_from_room_code, movement_step_with_obstacles, projectile_step,
    sample_terrain, InputState as CoreInputState, StructureObstacle, TerrainBaseKind,
    TerrainResourceKind, PLAYER_COLLIDER_RADIUS, STRUCTURE_COLLIDER_HALF_EXTENT,
    TERRAIN_GENERATOR_VERSION, TERRAIN_TILE_SIZE,
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
const MINING_DURATION_MS: i64 = 900;
const MINING_YIELD_PER_ACTION: u32 = 6;
const MINING_INTERACTION_RANGE: f32 = 74.0;
const MINING_NODE_SCAN_RADIUS_TILES: i32 = 14;
const MAX_MINING_NODES: usize = 4096;
const MAX_MINING_NODES_PER_SNAPSHOT: usize = 384;
const MAX_MINING_ACTIVE_PER_SNAPSHOT: usize = 256;
const ENEMY_SCAN_RADIUS_TILES: i32 = 18;
const ENEMY_DESPAWN_RADIUS_TILES: i32 = ENEMY_SCAN_RADIUS_TILES + 8;
const ENEMY_MIN_PLAYER_DISTANCE: f32 = 112.0;
const ENEMY_AGGRO_RANGE: f32 = 320.0;
const ENEMY_ATTACK_RANGE: f32 = 42.0;
const ENEMY_PROJECTILE_HIT_RADIUS: f32 = 20.0;
const ENEMY_MOVE_SPEED: f32 = 86.0;
const ENEMY_ATTACK_COOLDOWN_TICKS: u64 = 24;
const ENEMY_SPAWN_MODULUS: u64 = 997;
const ENEMY_SPAWN_THRESHOLD: u64 = 11;
const ENEMY_SPAWN_SALT: u64 = 0x6d13_ef42_2dd9_1f7a;
const ENEMY_KIND_BITER: &str = "biter";
const ENEMY_MAX_HEALTH: u16 = 64;
const ENEMY_ATTACK_POWER: u16 = 7;
const ENEMY_ARMOR: u16 = 1;
const PLAYER_COMBAT_MAX_HEALTH: u16 = 100;
const PLAYER_COMBAT_ATTACK_POWER: u16 = 18;
const PLAYER_COMBAT_ARMOR: u16 = 1;
const COMBAT_ATTACK_RANGE: f32 = 640.0;
const COMBAT_ATTACK_PROJECTILE_SPEED: f32 = 760.0;
const PROJECTILE_BASE_DAMAGE: u16 = 12;
const MAX_ENEMIES: usize = 1024;
const MAX_ENEMIES_PER_SNAPSHOT: usize = 320;
const DROP_TTL_MS: i64 = 30_000;
const DROP_OWNER_GRACE_MS: i64 = 4_000;
const DROP_PICKUP_RANGE: f32 = 84.0;
const MAX_DROPS: usize = 4096;
const MAX_DROPS_PER_SNAPSHOT: usize = 384;
const MAX_CRAFT_QUEUE_ENTRIES_PER_PLAYER: usize = 32;
const MAX_CRAFT_QUEUE_TOTAL_PER_PLAYER: u32 = 512;
const MAX_CRAFT_QUEUES_PER_SNAPSHOT: usize = 128;
const MAX_CRAFT_PENDING_ENTRIES_PER_SNAPSHOT_QUEUE: usize = 24;

const MAX_STRUCTURES: usize = 1024;
const MAX_PROJECTILES: usize = 4096;
const MAX_PREVIEWS: usize = 256;
const ROOM_META_ROOM_CODE_KEY: &str = "room_code";
const ROOM_META_TERRAIN_SEED_KEY: &str = "terrain_seed";
const DEFAULT_CHARACTER_SPRITE_ID: &str = "engineer-default";
const SUPPORTED_CHARACTER_SPRITE_IDS: [&str; 3] =
    ["engineer-default", "surveyor-cyan", "machinist-rose"];
const SUPPORTED_RESOURCE_IDS: [&str; 7] = [
    "iron_ore",
    "copper_ore",
    "coal",
    "stone",
    "iron_plate",
    "copper_plate",
    "gear",
];
const SUPPORTED_MINING_NODE_KINDS: [&str; 3] = ["iron_ore", "copper_ore", "coal"];
const DEFAULT_CHARACTER_PROFILE_ID: &str = "default";
const MAX_PROTOCOL_IDENTIFIER_LEN: usize = 64;
const MAX_CHARACTER_NAME_LEN: usize = 32;
const MAX_CHARACTER_PROFILE_SLOTS: usize = 8;

const BUILD_COST_BEACON: [(&str, u32); 2] = [("iron_plate", 2), ("copper_plate", 1)];
const BUILD_COST_MINER: [(&str, u32); 2] = [("iron_plate", 3), ("gear", 2)];
const BUILD_COST_ASSEMBLER: [(&str, u32); 2] = [("iron_plate", 9), ("gear", 5)];

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
    Drops,
    Crafting,
    Combat,
    Character,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoomApiEndpoint {
    WebSocket,
    CharacterProfiles,
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
struct InventoryDiscardPayload {
    slot: u16,
    amount: Option<u32>,
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
struct DropPickupPayload {
    drop_id: String,
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
    client_projectile_id: Option<String>,
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
#[serde(rename_all = "camelCase")]
struct CharacterProfileUpsertPayload {
    character_id: String,
    name: String,
    sprite_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CharacterProfilesUpdatePayload {
    profiles: Vec<CharacterProfileUpsertPayload>,
    active_character_id: String,
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
struct InventoryStackRow {
    player_id: String,
    slot: i64,
    resource: String,
    amount: i64,
}

#[derive(Debug, Deserialize)]
struct MiningNodeRow {
    node_id: String,
    kind: String,
    x: f64,
    y: f64,
    grid_x: i64,
    grid_y: i64,
    remaining: i64,
    max_yield: i64,
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
struct DropRow {
    drop_id: String,
    resource: String,
    amount: i64,
    x: f64,
    y: f64,
    owner_player_id: Option<String>,
    owner_expires_at: i64,
    expires_at: i64,
    created_at: i64,
    updated_at: i64,
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

#[derive(Debug, Clone)]
struct RuntimeInventoryStackState {
    resource: String,
    amount: u32,
}

#[derive(Debug, Clone)]
struct RuntimeInventoryState {
    max_slots: u8,
    slots: Vec<Option<RuntimeInventoryStackState>>,
}

#[derive(Debug, Clone)]
struct RuntimeMiningNodeState {
    node_id: String,
    kind: String,
    x: f32,
    y: f32,
    grid_x: i32,
    grid_y: i32,
    remaining: u32,
    max_yield: u32,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct RuntimeMiningProgressState {
    player_id: String,
    node_id: String,
    started_at: i64,
    completes_at: i64,
}

#[derive(Debug, Clone)]
struct RuntimeDropState {
    drop_id: String,
    resource: String,
    amount: u32,
    x: f32,
    y: f32,
    owner_player_id: Option<String>,
    owner_expires_at: i64,
    expires_at: i64,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct RuntimeEnemyState {
    enemy_id: String,
    kind: String,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    health: u16,
    max_health: u16,
    attack_power: u16,
    armor: u16,
    target_player_id: Option<String>,
    last_attack_tick: u64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct RuntimePlayerCombatState {
    health: u16,
    max_health: u16,
    attack_power: u16,
    armor: u16,
}

impl Default for RuntimePlayerCombatState {
    fn default() -> Self {
        Self {
            health: PLAYER_COMBAT_MAX_HEALTH,
            max_health: PLAYER_COMBAT_MAX_HEALTH,
            attack_power: PLAYER_COMBAT_ATTACK_POWER,
            armor: PLAYER_COMBAT_ARMOR,
        }
    }
}

#[derive(Debug, Clone)]
struct RuntimeCraftQueueEntry {
    recipe: String,
    count: u16,
}

#[derive(Debug, Clone)]
struct RuntimeActiveCraftState {
    recipe: String,
    remaining_ticks: u16,
}

#[derive(Debug, Clone, Default)]
struct RuntimeCraftQueueState {
    pending: Vec<RuntimeCraftQueueEntry>,
    active: Option<RuntimeActiveCraftState>,
}

#[derive(Debug, Clone, Copy, Default)]
struct CraftingTickResult {
    changed: bool,
    inventory_changed: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct CombatTickResult {
    changed: bool,
    projectiles_changed: bool,
    drops_changed: bool,
}

impl RuntimeInventoryState {
    fn new(max_slots: u8) -> Self {
        Self {
            max_slots,
            slots: vec![None; max_slots as usize],
        }
    }

    fn normalize(&mut self) {
        let expected_len = self.max_slots as usize;
        if self.slots.len() < expected_len {
            self.slots.resize(expected_len, None);
        } else if self.slots.len() > expected_len {
            self.slots.truncate(expected_len);
        }
    }

    fn slot_index(&self, slot: u16) -> Result<usize> {
        let index = slot as usize;
        if index >= self.max_slots as usize {
            return Err(Error::RustError("inventory slot out of bounds".into()));
        }
        Ok(index)
    }

    fn first_free_slot(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_none)
    }

    #[allow(dead_code)]
    fn add_resource(&mut self, resource: &str, amount: u32) -> Result<()> {
        if !is_supported_resource_id(resource) {
            return Err(Error::RustError("invalid inventory resource id".into()));
        }
        if amount == 0 {
            return Err(Error::RustError("inventory amount must be positive".into()));
        }

        if let Some(existing) = self
            .slots
            .iter_mut()
            .flatten()
            .find(|stack| stack.resource == resource)
        {
            existing.amount = existing
                .amount
                .checked_add(amount)
                .ok_or_else(|| Error::RustError("inventory stack overflow".into()))?;
            return Ok(());
        }

        let Some(free_slot) = self.first_free_slot() else {
            return Err(Error::RustError("inventory has no free slot".into()));
        };

        self.slots[free_slot] = Some(RuntimeInventoryStackState {
            resource: resource.to_string(),
            amount,
        });
        Ok(())
    }

    #[allow(dead_code)]
    fn remove_resource(&mut self, resource: &str, amount: u32) -> Result<()> {
        if !is_supported_resource_id(resource) {
            return Err(Error::RustError("invalid inventory resource id".into()));
        }
        if amount == 0 {
            return Err(Error::RustError("inventory amount must be positive".into()));
        }

        let available: u32 = self
            .slots
            .iter()
            .flatten()
            .filter(|stack| stack.resource == resource)
            .map(|stack| stack.amount)
            .sum();

        if available < amount {
            return Err(Error::RustError("insufficient inventory resource".into()));
        }

        let mut remaining = amount;
        for slot in self.slots.iter_mut().flatten() {
            if slot.resource != resource || remaining == 0 {
                continue;
            }
            let taken = slot.amount.min(remaining);
            slot.amount -= taken;
            remaining -= taken;
        }

        for slot in self.slots.iter_mut() {
            if slot.as_ref().is_some_and(|stack| stack.amount == 0) {
                *slot = None;
            }
        }

        Ok(())
    }

    fn move_stack(&mut self, from_slot: u16, to_slot: u16, amount: Option<u32>) -> Result<()> {
        if from_slot == to_slot {
            return Err(Error::RustError(
                "inventory source and destination match".into(),
            ));
        }
        let from_index = self.slot_index(from_slot)?;
        let to_index = self.slot_index(to_slot)?;

        let source = self.slots[from_index]
            .clone()
            .ok_or_else(|| Error::RustError("inventory source slot is empty".into()))?;
        let moving_amount = amount.unwrap_or(source.amount);

        if moving_amount == 0 || moving_amount > source.amount {
            return Err(Error::RustError("invalid inventory move amount".into()));
        }

        let destination = self.slots[to_index].clone();
        match destination {
            None => {
                if moving_amount == source.amount {
                    self.slots[to_index] = Some(source);
                    self.slots[from_index] = None;
                } else {
                    self.slots[to_index] = Some(RuntimeInventoryStackState {
                        resource: source.resource.clone(),
                        amount: moving_amount,
                    });
                    self.slots[from_index] = Some(RuntimeInventoryStackState {
                        resource: source.resource,
                        amount: source.amount - moving_amount,
                    });
                }
                Ok(())
            }
            Some(mut destination_stack) => {
                if destination_stack.resource == source.resource {
                    destination_stack.amount = destination_stack
                        .amount
                        .checked_add(moving_amount)
                        .ok_or_else(|| Error::RustError("inventory stack overflow".into()))?;

                    let remaining = source.amount - moving_amount;
                    self.slots[to_index] = Some(destination_stack);
                    self.slots[from_index] = if remaining == 0 {
                        None
                    } else {
                        Some(RuntimeInventoryStackState {
                            resource: source.resource,
                            amount: remaining,
                        })
                    };
                    return Ok(());
                }

                if moving_amount != source.amount {
                    return Err(Error::RustError(
                        "cannot partially move into occupied slot with different resource".into(),
                    ));
                }

                self.slots[to_index] = Some(source);
                self.slots[from_index] = Some(destination_stack);
                Ok(())
            }
        }
    }

    fn split_stack(&mut self, slot: u16, amount: u32) -> Result<()> {
        let source_index = self.slot_index(slot)?;
        if amount == 0 {
            return Err(Error::RustError("invalid inventory split amount".into()));
        }

        let source = self.slots[source_index]
            .clone()
            .ok_or_else(|| Error::RustError("inventory source slot is empty".into()))?;
        if amount >= source.amount {
            return Err(Error::RustError(
                "split amount must be less than stack size".into(),
            ));
        }

        let Some(destination_index) = self.first_free_slot() else {
            return Err(Error::RustError("inventory has no free slot".into()));
        };

        self.slots[source_index] = Some(RuntimeInventoryStackState {
            resource: source.resource.clone(),
            amount: source.amount - amount,
        });
        self.slots[destination_index] = Some(RuntimeInventoryStackState {
            resource: source.resource,
            amount,
        });
        Ok(())
    }

    fn discard_from_slot(
        &mut self,
        slot: u16,
        amount: Option<u32>,
    ) -> Result<RuntimeInventoryStackState> {
        let slot_index = self.slot_index(slot)?;
        let source = self.slots[slot_index]
            .clone()
            .ok_or_else(|| Error::RustError("inventory source slot is empty".into()))?;
        let drop_amount = amount.unwrap_or(source.amount);

        if drop_amount == 0 || drop_amount > source.amount {
            return Err(Error::RustError("invalid inventory discard amount".into()));
        }

        if drop_amount == source.amount {
            self.slots[slot_index] = None;
        } else {
            self.slots[slot_index] = Some(RuntimeInventoryStackState {
                resource: source.resource.clone(),
                amount: source.amount - drop_amount,
            });
        }

        Ok(RuntimeInventoryStackState {
            resource: source.resource,
            amount: drop_amount,
        })
    }

    #[allow(dead_code)]
    fn total_resource_amount(&self, resource: &str) -> u32 {
        self.slots
            .iter()
            .flatten()
            .filter(|stack| stack.resource == resource)
            .map(|stack| stack.amount)
            .sum()
    }
}

impl RuntimeCraftQueueState {
    fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.active.is_none()
    }

    fn pending_total_count(&self) -> u32 {
        self.pending.iter().map(|entry| entry.count as u32).sum()
    }

    fn peek_pending_recipe(&self) -> Option<&str> {
        self.pending.first().map(|entry| entry.recipe.as_str())
    }

    fn consume_one_pending(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }

        let mut remove_head = false;
        let recipe = {
            let head = self.pending.first_mut()?;
            if head.count == 0 {
                remove_head = true;
                head.recipe.clone()
            } else {
                head.count -= 1;
                if head.count == 0 {
                    remove_head = true;
                }
                head.recipe.clone()
            }
        };

        if remove_head {
            self.pending.remove(0);
        }
        Some(recipe)
    }
}

fn recipe_kind_from_id(recipe_id: &str) -> Option<RecipeKind> {
    match recipe_id {
        "smelt_iron_plate" => Some(RecipeKind::SmeltIronPlate),
        "smelt_copper_plate" => Some(RecipeKind::SmeltCopperPlate),
        "craft_gear" => Some(RecipeKind::CraftGear),
        _ => None,
    }
}

fn recipe_id_from_kind(recipe: RecipeKind) -> &'static str {
    match recipe {
        RecipeKind::SmeltIronPlate => "smelt_iron_plate",
        RecipeKind::SmeltCopperPlate => "smelt_copper_plate",
        RecipeKind::CraftGear => "craft_gear",
    }
}

fn resource_id_from_kind(resource: ResourceKind) -> &'static str {
    match resource {
        ResourceKind::IronOre => "iron_ore",
        ResourceKind::CopperOre => "copper_ore",
        ResourceKind::Coal => "coal",
        ResourceKind::Stone => "stone",
        ResourceKind::IronPlate => "iron_plate",
        ResourceKind::CopperPlate => "copper_plate",
        ResourceKind::Gear => "gear",
    }
}

fn advance_crafting_queue(
    inventory: &mut RuntimeInventoryState,
    queue: &mut RuntimeCraftQueueState,
) -> CraftingTickResult {
    let mut result = CraftingTickResult::default();
    inventory.normalize();

    if queue.active.is_none() {
        let next_recipe = queue.peek_pending_recipe().map(str::to_string);
        if let Some(next_recipe) = next_recipe {
            if let Some(recipe_kind) = recipe_kind_from_id(next_recipe.as_str()) {
                let definition = recipe_definition(recipe_kind);
                let mut consumed_inventory = inventory.clone();
                let mut can_start = true;
                for input in definition.inputs.iter() {
                    if consumed_inventory
                        .remove_resource(resource_id_from_kind(input.resource), input.amount)
                        .is_err()
                    {
                        can_start = false;
                        break;
                    }
                }

                if can_start {
                    *inventory = consumed_inventory;
                    if let Some(started_recipe) = queue.consume_one_pending() {
                        queue.active = Some(RuntimeActiveCraftState {
                            recipe: started_recipe,
                            remaining_ticks: definition.craft_ticks.max(1),
                        });
                        result.changed = true;
                        result.inventory_changed = true;
                    }
                }
            } else {
                queue.consume_one_pending();
                result.changed = true;
            }
        }
    }

    let completed_recipe = {
        let Some(active) = queue.active.as_mut() else {
            return result;
        };
        if active.remaining_ticks > 0 {
            active.remaining_ticks -= 1;
            result.changed = true;
        }
        if active.remaining_ticks == 0 {
            Some(active.recipe.clone())
        } else {
            None
        }
    };

    let Some(completed_recipe) = completed_recipe else {
        return result;
    };

    queue.active = None;
    result.changed = true;

    let Some(recipe_kind) = recipe_kind_from_id(completed_recipe.as_str()) else {
        return result;
    };
    let definition = recipe_definition(recipe_kind);
    let mut produced_inventory = inventory.clone();
    for output in definition.outputs.iter() {
        if produced_inventory
            .add_resource(resource_id_from_kind(output.resource), output.amount)
            .is_err()
        {
            return result;
        }
    }

    *inventory = produced_inventory;
    result.inventory_changed = true;
    result
}

#[derive(Debug, Default)]
struct RoomRuntimeState {
    players: HashMap<String, RuntimePlayerState>,
    structures: HashMap<String, RuntimeStructureState>,
    previews: HashMap<String, RuntimePreviewState>,
    projectiles: HashMap<String, RuntimeProjectileState>,
    inventories: HashMap<String, RuntimeInventoryState>,
    mining_nodes: HashMap<String, RuntimeMiningNodeState>,
    mining_active: HashMap<String, RuntimeMiningProgressState>,
    drops: HashMap<String, RuntimeDropState>,
    enemies: HashMap<String, RuntimeEnemyState>,
    combat_players: HashMap<String, RuntimePlayerCombatState>,
    craft_queues: HashMap<String, RuntimeCraftQueueState>,
}

fn now_ms() -> i64 {
    Date::now().as_millis() as i64
}

fn now_seconds() -> i64 {
    now_ms() / 1000
}

fn decode_payload<T>(
    payload: Option<Value>,
    missing_error: &'static str,
    invalid_error: &'static str,
) -> Result<T>
where
    T: DeserializeOwned,
{
    let payload = payload.ok_or_else(|| Error::RustError(missing_error.into()))?;
    serde_json::from_value(payload).map_err(|_| Error::RustError(invalid_error.into()))
}

fn parse_client_command_envelope_message(
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

fn validate_movement_input_batch_payload(payload: Option<Value>) -> Result<InputBatchPayload> {
    let input_batch: InputBatchPayload = decode_payload(
        payload,
        "missing movement payload",
        "invalid movement payload",
    )?;

    if input_batch.inputs.len() > 128 {
        return Err(Error::RustError("input batch too large".into()));
    }

    Ok(input_batch)
}

fn validate_inventory_move_payload(payload: Option<Value>) -> Result<InventoryMovePayload> {
    let move_payload: InventoryMovePayload = decode_payload(
        payload,
        "missing inventory payload",
        "invalid inventory payload",
    )?;

    if move_payload.from_slot == move_payload.to_slot
        || move_payload.from_slot >= DEFAULT_INVENTORY_MAX_SLOTS as u16
        || move_payload.to_slot >= DEFAULT_INVENTORY_MAX_SLOTS as u16
        || move_payload.amount.is_some_and(|amount| amount == 0)
    {
        return Err(Error::RustError("invalid inventory move payload".into()));
    }

    Ok(move_payload)
}

fn validate_inventory_split_payload(payload: Option<Value>) -> Result<InventorySplitPayload> {
    let split_payload: InventorySplitPayload = decode_payload(
        payload,
        "missing inventory payload",
        "invalid inventory payload",
    )?;

    if split_payload.slot >= DEFAULT_INVENTORY_MAX_SLOTS as u16 || split_payload.amount == 0 {
        return Err(Error::RustError("invalid inventory split payload".into()));
    }

    Ok(split_payload)
}

fn validate_inventory_discard_payload(payload: Option<Value>) -> Result<InventoryDiscardPayload> {
    let discard_payload: InventoryDiscardPayload = decode_payload(
        payload,
        "missing inventory payload",
        "invalid inventory payload",
    )?;

    if discard_payload.slot >= DEFAULT_INVENTORY_MAX_SLOTS as u16
        || discard_payload.amount.is_some_and(|amount| amount == 0)
    {
        return Err(Error::RustError("invalid inventory discard payload".into()));
    }

    Ok(discard_payload)
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

fn is_supported_character_sprite_id(sprite_id: &str) -> bool {
    SUPPORTED_CHARACTER_SPRITE_IDS
        .iter()
        .any(|supported| supported == &sprite_id)
}

fn is_supported_resource_id(resource_id: &str) -> bool {
    SUPPORTED_RESOURCE_IDS
        .iter()
        .any(|supported| supported == &resource_id)
}

fn is_supported_mining_node_kind(kind: &str) -> bool {
    SUPPORTED_MINING_NODE_KINDS
        .iter()
        .any(|supported| supported == &kind)
}

fn terrain_resource_to_inventory_id(resource: TerrainResourceKind) -> &'static str {
    match resource {
        TerrainResourceKind::IronOre => "iron_ore",
        TerrainResourceKind::CopperOre => "copper_ore",
        TerrainResourceKind::Coal => "coal",
    }
}

fn mining_node_id_for_grid(grid_x: i32, grid_y: i32) -> String {
    format!("node:{grid_x}:{grid_y}")
}

fn enemy_id_for_grid(grid_x: i32, grid_y: i32) -> String {
    format!("enemy:{grid_x}:{grid_y}")
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn hash_grid(seed: u64, grid_x: i32, grid_y: i32) -> u64 {
    let x_bits = grid_x as i64 as u64;
    let y_bits = grid_y as i64 as u64;
    splitmix64(
        seed ^ x_bits.wrapping_mul(0x517c_c1b7_2722_0a95)
            ^ y_bits.wrapping_mul(0x9e37_79b9_7f4a_7c15),
    )
}

fn sample_enemy_spawn(seed: u64, grid_x: i32, grid_y: i32) -> Option<(String, u16, u16, u16)> {
    let terrain = sample_terrain(seed, grid_x, grid_y);
    if matches!(
        terrain.base,
        TerrainBaseKind::DeepWater | TerrainBaseKind::ShallowWater
    ) {
        return None;
    }

    let spawn_roll = hash_grid(seed ^ ENEMY_SPAWN_SALT, grid_x, grid_y) % ENEMY_SPAWN_MODULUS;
    if spawn_roll > ENEMY_SPAWN_THRESHOLD {
        return None;
    }

    Some((
        ENEMY_KIND_BITER.to_string(),
        ENEMY_MAX_HEALTH,
        ENEMY_ATTACK_POWER,
        ENEMY_ARMOR,
    ))
}

fn terrain_grid_axis(world_axis: f32) -> i32 {
    (world_axis / TERRAIN_TILE_SIZE as f32).floor() as i32
}

fn player_within_mining_range(player_x: f32, player_y: f32, node: &RuntimeMiningNodeState) -> bool {
    let dx = player_x - node.x;
    let dy = player_y - node.y;
    dx * dx + dy * dy <= MINING_INTERACTION_RANGE * MINING_INTERACTION_RANGE
}

fn mining_progress_ratio(progress: &RuntimeMiningProgressState, now: i64) -> f32 {
    let duration_ms = (progress.completes_at - progress.started_at).max(1) as f32;
    ((now - progress.started_at) as f32 / duration_ms).clamp(0.0, 1.0)
}

fn next_drop_id(now: i64) -> String {
    format!(
        "drop:{:x}:{:x}",
        now as u64,
        (js_sys::Math::random() * 1e12) as u64
    )
}

fn player_within_drop_pickup_range(player_x: f32, player_y: f32, drop: &RuntimeDropState) -> bool {
    let dx = player_x - drop.x;
    let dy = player_y - drop.y;
    dx * dx + dy * dy <= DROP_PICKUP_RANGE * DROP_PICKUP_RANGE
}

fn enemy_within_despawn_range(player: &RuntimePlayerState, enemy: &RuntimeEnemyState) -> bool {
    (terrain_grid_axis(player.x) - terrain_grid_axis(enemy.x)).abs() <= ENEMY_DESPAWN_RADIUS_TILES
        && (terrain_grid_axis(player.y) - terrain_grid_axis(enemy.y)).abs()
            <= ENEMY_DESPAWN_RADIUS_TILES
}

fn enemy_within_aggro_range(player: &RuntimePlayerState, enemy: &RuntimeEnemyState) -> bool {
    let dx = player.x - enemy.x;
    let dy = player.y - enemy.y;
    dx * dx + dy * dy <= ENEMY_AGGRO_RANGE * ENEMY_AGGRO_RANGE
}

fn clamp_projectile_velocity(vx: f32, vy: f32) -> (f32, f32) {
    let speed = (vx * vx + vy * vy).sqrt();
    if speed <= PROJECTILE_MAX_SPEED as f32 || speed <= f32::EPSILON {
        return (vx, vy);
    }

    let scale = PROJECTILE_MAX_SPEED as f32 / speed;
    (vx * scale, vy * scale)
}

fn player_within_combat_attack_range(
    player_x: f32,
    player_y: f32,
    target_x: f32,
    target_y: f32,
) -> bool {
    let dx = target_x - player_x;
    let dy = target_y - player_y;
    dx * dx + dy * dy <= COMBAT_ATTACK_RANGE * COMBAT_ATTACK_RANGE
}

fn projectile_velocity_towards_target(
    source_x: f32,
    source_y: f32,
    target_x: f32,
    target_y: f32,
    speed: f32,
) -> Option<(f32, f32)> {
    let dx = target_x - source_x;
    let dy = target_y - source_y;
    let distance_sq = dx * dx + dy * dy;
    if distance_sq <= f32::EPSILON {
        return None;
    }

    let distance = distance_sq.sqrt();
    let normalized_x = dx / distance;
    let normalized_y = dy / distance;
    Some((normalized_x * speed, normalized_y * speed))
}

fn resolve_damage(current_health: u16, armor: u16, incoming: u16) -> (u16, u16, bool) {
    let mitigated = incoming.saturating_sub(armor).max(1);
    let applied = mitigated.min(current_health);
    let remaining = current_health.saturating_sub(applied);
    (applied, remaining, remaining == 0)
}

fn enemy_drop_for_kind(kind: &str) -> (&'static str, u32) {
    match kind {
        ENEMY_KIND_BITER => ("gear", 1),
        _ => ("iron_ore", 1),
    }
}

fn player_within_drop_visibility_range(
    player: &RuntimePlayerState,
    drop: &RuntimeDropState,
) -> bool {
    (terrain_grid_axis(player.x) - terrain_grid_axis(drop.x)).abs()
        <= MINING_NODE_SCAN_RADIUS_TILES + 4
        && (terrain_grid_axis(player.y) - terrain_grid_axis(drop.y)).abs()
            <= MINING_NODE_SCAN_RADIUS_TILES + 4
}

fn drop_visible_to_recipient(
    drop: &RuntimeDropState,
    recipient_player_id: Option<&str>,
    players: &HashMap<String, RuntimePlayerState>,
    connected_players: &[String],
) -> bool {
    if let Some(player_id) = recipient_player_id {
        return players.get(player_id).is_some_and(|player| {
            player.connected && player_within_drop_visibility_range(player, drop)
        });
    }

    connected_players.iter().any(|player_id| {
        players.get(player_id.as_str()).is_some_and(|player| {
            player.connected && player_within_drop_visibility_range(player, drop)
        })
    })
}

fn drop_pickup_allowed_for_player(drop: &RuntimeDropState, player_id: &str, now: i64) -> bool {
    match drop.owner_player_id.as_deref() {
        None => true,
        Some(owner_id) if owner_id == player_id => true,
        Some(_) => now >= drop.owner_expires_at,
    }
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

fn parse_room_endpoint(path: &str) -> Option<(String, RoomApiEndpoint)> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 5 {
        return None;
    }

    if parts[1] != "api" || parts[2] != "rooms" {
        return None;
    }

    let room_code = sanitize_room_code(parts[3])?;
    let endpoint = match parts[4] {
        "ws" => RoomApiEndpoint::WebSocket,
        "character-profiles" => RoomApiEndpoint::CharacterProfiles,
        _ => return None,
    };

    Some((room_code, endpoint))
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

fn structure_build_cost(kind: &str) -> Option<&'static [(&'static str, u32)]> {
    match kind {
        "beacon" => Some(&BUILD_COST_BEACON),
        "miner" => Some(&BUILD_COST_MINER),
        "assembler" => Some(&BUILD_COST_ASSEMBLER),
        _ => None,
    }
}

fn consume_structure_build_cost(inventory: &mut RuntimeInventoryState, kind: &str) -> Result<()> {
    let cost = structure_build_cost(kind)
        .ok_or_else(|| Error::RustError("invalid structure kind".into()))?;
    if cost.is_empty() {
        return Ok(());
    }

    let mut consumed_inventory = inventory.clone();
    for (resource, amount) in cost.iter() {
        if consumed_inventory
            .remove_resource(resource, *amount)
            .is_err()
        {
            return Err(Error::RustError(
                "insufficient inventory resources for build placement".into(),
            ));
        }
    }

    *inventory = consumed_inventory;
    Ok(())
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

    if let Some((room_code, _endpoint)) = parse_room_endpoint(url.path()) {
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
    dirty_inventory: Cell<bool>,
    dirty_mining: Cell<bool>,
    dirty_drops: Cell<bool>,
    dirty_crafting: Cell<bool>,
    dirty_combat: Cell<bool>,
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

        let inventory_rows: Vec<InventoryStackRow> = sql
            .exec(
                "
                SELECT player_id, slot, resource, amount
                FROM player_inventory_stacks
                ORDER BY player_id ASC, slot ASC
                ",
                None,
            )?
            .to_array()?;

        let mining_rows: Vec<MiningNodeRow> = sql
            .exec(
                "
                SELECT node_id, kind, x, y, grid_x, grid_y, remaining, max_yield, updated_at
                FROM mining_nodes
                ORDER BY updated_at DESC
                LIMIT ?
                ",
                Some(vec![(MAX_MINING_NODES as i64).into()]),
            )?
            .to_array()?;

        let drop_rows: Vec<DropRow> = sql
            .exec(
                "
                SELECT drop_id, resource, amount, x, y, owner_player_id, owner_expires_at, expires_at, created_at, updated_at
                FROM world_drops
                ORDER BY updated_at DESC
                LIMIT ?
                ",
                Some(vec![(MAX_DROPS as i64).into()]),
            )?
            .to_array()?;

        let mut runtime = self.runtime.borrow_mut();
        runtime.players.clear();
        runtime.structures.clear();
        runtime.previews.clear();
        runtime.projectiles.clear();
        runtime.inventories.clear();
        runtime.mining_nodes.clear();
        runtime.mining_active.clear();
        runtime.drops.clear();
        runtime.enemies.clear();
        runtime.combat_players.clear();
        runtime.craft_queues.clear();

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
            runtime
                .combat_players
                .entry(row.player_id)
                .or_insert_with(RuntimePlayerCombatState::default);
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

        for row in inventory_rows {
            if row.slot < 0
                || row.slot >= DEFAULT_INVENTORY_MAX_SLOTS as i64
                || row.amount <= 0
                || !is_supported_resource_id(row.resource.as_str())
            {
                continue;
            }

            let Ok(amount) = u32::try_from(row.amount) else {
                continue;
            };
            if amount == 0 {
                continue;
            }

            let inventory = runtime
                .inventories
                .entry(row.player_id)
                .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
            inventory.normalize();
            inventory.slots[row.slot as usize] = Some(RuntimeInventoryStackState {
                resource: row.resource,
                amount,
            });
        }

        for row in mining_rows {
            if !is_valid_protocol_identifier(row.node_id.as_str())
                || !is_supported_mining_node_kind(row.kind.as_str())
                || row.remaining < 0
                || row.max_yield <= 0
                || row.remaining > row.max_yield
            {
                continue;
            }

            let Ok(grid_x) = i32::try_from(row.grid_x) else {
                continue;
            };
            let Ok(grid_y) = i32::try_from(row.grid_y) else {
                continue;
            };
            let Ok(remaining) = u32::try_from(row.remaining) else {
                continue;
            };
            let Ok(max_yield) = u32::try_from(row.max_yield) else {
                continue;
            };

            runtime.mining_nodes.insert(
                row.node_id.clone(),
                RuntimeMiningNodeState {
                    node_id: row.node_id,
                    kind: row.kind,
                    x: row.x as f32,
                    y: row.y as f32,
                    grid_x,
                    grid_y,
                    remaining,
                    max_yield,
                    updated_at: row.updated_at,
                },
            );
        }

        for row in drop_rows {
            if !is_valid_protocol_identifier(row.drop_id.as_str())
                || !is_supported_resource_id(row.resource.as_str())
                || row.amount <= 0
                || row.expires_at <= now
            {
                continue;
            }

            let Ok(amount) = u32::try_from(row.amount) else {
                continue;
            };
            if amount == 0 {
                continue;
            }

            let owner_player_id = row.owner_player_id.as_deref().and_then(sanitize_player_id);

            runtime.drops.insert(
                row.drop_id.clone(),
                RuntimeDropState {
                    drop_id: row.drop_id,
                    resource: row.resource,
                    amount,
                    x: row.x as f32,
                    y: row.y as f32,
                    owner_player_id,
                    owner_expires_at: row.owner_expires_at,
                    expires_at: row.expires_at,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
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

    fn ensure_runtime_inventory_for_player(&self, player_id: &str) {
        let mut runtime = self.runtime.borrow_mut();
        runtime
            .inventories
            .entry(player_id.to_string())
            .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
    }

    fn ensure_runtime_combat_for_player(&self, player_id: &str) {
        let mut runtime = self.runtime.borrow_mut();
        runtime
            .combat_players
            .entry(player_id.to_string())
            .or_insert_with(RuntimePlayerCombatState::default);
    }

    fn persist_inventory_for_player(&self, player_id: &str) -> Result<()> {
        let inventory = {
            let runtime = self.runtime.borrow();
            runtime
                .inventories
                .get(player_id)
                .cloned()
                .unwrap_or_else(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS))
        };
        self.persist_inventory_state(player_id, &inventory)
    }

    fn persist_inventory_state(
        &self,
        player_id: &str,
        inventory: &RuntimeInventoryState,
    ) -> Result<()> {
        let sql = self.sql();
        let now = now_ms();
        let mut normalized = inventory.clone();
        normalized.normalize();

        sql.exec(
            "DELETE FROM player_inventory_stacks WHERE player_id = ?",
            Some(vec![player_id.into()]),
        )?;

        for (slot_index, maybe_stack) in normalized.slots.iter().enumerate() {
            let Some(stack) = maybe_stack else {
                continue;
            };

            sql.exec(
                "
                INSERT INTO player_inventory_stacks (player_id, slot, resource, amount, updated_at)
                VALUES (?, ?, ?, ?, ?)
                ",
                Some(vec![
                    player_id.into(),
                    (slot_index as i64).into(),
                    stack.resource.as_str().into(),
                    (stack.amount as i64).into(),
                    now.into(),
                ]),
            )?;
        }

        Ok(())
    }

    fn persist_mining_node(&self, node: &RuntimeMiningNodeState) -> Result<()> {
        self.sql().exec(
            "
            INSERT INTO mining_nodes (node_id, kind, x, y, grid_x, grid_y, remaining, max_yield, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(node_id) DO UPDATE SET
              kind = excluded.kind,
              x = excluded.x,
              y = excluded.y,
              grid_x = excluded.grid_x,
              grid_y = excluded.grid_y,
              remaining = excluded.remaining,
              max_yield = excluded.max_yield,
              updated_at = excluded.updated_at
            ",
            Some(vec![
                node.node_id.as_str().into(),
                node.kind.as_str().into(),
                (node.x as f64).into(),
                (node.y as f64).into(),
                (node.grid_x as i64).into(),
                (node.grid_y as i64).into(),
                (node.remaining as i64).into(),
                (node.max_yield as i64).into(),
                node.updated_at.into(),
            ]),
        )?;
        Ok(())
    }

    fn persist_drop(&self, drop: &RuntimeDropState) -> Result<()> {
        self.sql().exec(
            "
            INSERT INTO world_drops (
              drop_id, resource, amount, x, y,
              owner_player_id, owner_expires_at, expires_at, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(drop_id) DO UPDATE SET
              resource = excluded.resource,
              amount = excluded.amount,
              x = excluded.x,
              y = excluded.y,
              owner_player_id = excluded.owner_player_id,
              owner_expires_at = excluded.owner_expires_at,
              expires_at = excluded.expires_at,
              created_at = excluded.created_at,
              updated_at = excluded.updated_at
            ",
            Some(vec![
                drop.drop_id.as_str().into(),
                drop.resource.as_str().into(),
                (drop.amount as i64).into(),
                (drop.x as f64).into(),
                (drop.y as f64).into(),
                drop.owner_player_id.clone().into(),
                drop.owner_expires_at.into(),
                drop.expires_at.into(),
                drop.created_at.into(),
                drop.updated_at.into(),
            ]),
        )?;
        Ok(())
    }

    fn delete_drop(&self, drop_id: &str) -> Result<()> {
        self.sql().exec(
            "DELETE FROM world_drops WHERE drop_id = ?",
            Some(vec![drop_id.into()]),
        )?;
        Ok(())
    }

    fn spawn_world_drop(
        &self,
        resource: &str,
        amount: u32,
        x: f32,
        y: f32,
        owner_player_id: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        if amount == 0 || !is_supported_resource_id(resource) {
            return Err(Error::RustError("invalid drop payload".into()));
        }

        let mut overflow_drop_id: Option<String> = None;
        let owner_player_id = owner_player_id.and_then(sanitize_player_id);

        let drop = {
            let mut runtime = self.runtime.borrow_mut();
            let mut drop_id = next_drop_id(now);
            let mut attempts = 0u8;
            while runtime.drops.contains_key(drop_id.as_str()) {
                attempts = attempts.saturating_add(1);
                if attempts > 8 {
                    return Err(Error::RustError("failed to allocate drop id".into()));
                }
                drop_id = next_drop_id(now + attempts as i64);
            }

            if runtime.drops.len() >= MAX_DROPS {
                overflow_drop_id = runtime
                    .drops
                    .values()
                    .min_by_key(|drop| (drop.expires_at, drop.created_at))
                    .map(|drop| drop.drop_id.clone());

                if let Some(overflow_id) = overflow_drop_id.as_deref() {
                    runtime.drops.remove(overflow_id);
                }
            }

            let drop = RuntimeDropState {
                drop_id: drop_id.clone(),
                resource: resource.to_string(),
                amount,
                x,
                y,
                owner_player_id,
                owner_expires_at: now + DROP_OWNER_GRACE_MS,
                expires_at: now + DROP_TTL_MS,
                created_at: now,
                updated_at: now,
            };
            runtime.drops.insert(drop_id, drop.clone());
            drop
        };

        if let Some(overflow_id) = overflow_drop_id.as_deref() {
            self.delete_drop(overflow_id)?;
        }
        self.persist_drop(&drop)?;
        Ok(true)
    }

    fn prune_expired_drops(&self, now: i64) -> Result<bool> {
        let expired_drop_ids: Vec<String> = {
            let runtime = self.runtime.borrow();
            runtime
                .drops
                .values()
                .filter(|drop| drop.expires_at <= now)
                .map(|drop| drop.drop_id.clone())
                .collect()
        };

        if expired_drop_ids.is_empty() {
            return Ok(false);
        }

        {
            let mut runtime = self.runtime.borrow_mut();
            for drop_id in expired_drop_ids.iter() {
                runtime.drops.remove(drop_id.as_str());
            }
        }

        for drop_id in expired_drop_ids.iter() {
            self.delete_drop(drop_id.as_str())?;
        }

        Ok(true)
    }

    fn materialize_mining_nodes_for_connected_players(&self) -> Result<bool> {
        let connected_players = self.connected_player_ids();
        if connected_players.is_empty() {
            return Ok(false);
        }

        let player_centers: Vec<(i32, i32)> = {
            let runtime = self.runtime.borrow();
            connected_players
                .iter()
                .filter_map(|player_id| runtime.players.get(player_id))
                .filter(|player| player.connected)
                .map(|player| (terrain_grid_axis(player.x), terrain_grid_axis(player.y)))
                .collect()
        };

        if player_centers.is_empty() {
            return Ok(false);
        }

        let seed = self.terrain_seed.get();
        let now = now_ms();
        let mut discovered: Vec<RuntimeMiningNodeState> = Vec::new();
        let mut discovered_ids = HashSet::new();

        {
            let runtime = self.runtime.borrow();
            for (center_x, center_y) in player_centers.iter().copied() {
                for grid_x in (center_x - MINING_NODE_SCAN_RADIUS_TILES)
                    ..=(center_x + MINING_NODE_SCAN_RADIUS_TILES)
                {
                    for grid_y in (center_y - MINING_NODE_SCAN_RADIUS_TILES)
                        ..=(center_y + MINING_NODE_SCAN_RADIUS_TILES)
                    {
                        let node_id = mining_node_id_for_grid(grid_x, grid_y);
                        if runtime.mining_nodes.contains_key(node_id.as_str())
                            || discovered_ids.contains(node_id.as_str())
                        {
                            continue;
                        }

                        let sample = sample_terrain(seed, grid_x, grid_y);
                        let Some(resource) = sample.resource else {
                            continue;
                        };

                        let kind = terrain_resource_to_inventory_id(resource).to_string();
                        if !is_supported_mining_node_kind(kind.as_str()) {
                            continue;
                        }

                        let max_yield = sample.resource_richness.max(1) as u32;
                        discovered_ids.insert(node_id.clone());
                        discovered.push(RuntimeMiningNodeState {
                            node_id,
                            kind,
                            x: (grid_x as f32) * TERRAIN_TILE_SIZE as f32,
                            y: (grid_y as f32) * TERRAIN_TILE_SIZE as f32,
                            grid_x,
                            grid_y,
                            remaining: max_yield,
                            max_yield,
                            updated_at: now,
                        });
                    }
                }
            }
        }

        if discovered.is_empty() {
            return Ok(false);
        }

        {
            let mut runtime = self.runtime.borrow_mut();
            for node in discovered.iter() {
                runtime
                    .mining_nodes
                    .entry(node.node_id.clone())
                    .or_insert_with(|| node.clone());
            }
        }

        for node in discovered.iter() {
            self.persist_mining_node(node)?;
        }

        Ok(true)
    }

    fn materialize_enemies_for_connected_players(&self) -> Result<bool> {
        let connected_players = self.connected_player_ids();
        if connected_players.is_empty() {
            return Ok(false);
        }

        let player_positions: Vec<(f32, f32)> = {
            let runtime = self.runtime.borrow();
            connected_players
                .iter()
                .filter_map(|player_id| runtime.players.get(player_id))
                .filter(|player| player.connected)
                .map(|player| (player.x, player.y))
                .collect()
        };
        if player_positions.is_empty() {
            return Ok(false);
        }

        let player_centers: Vec<(i32, i32)> = player_positions
            .iter()
            .map(|(x, y)| (terrain_grid_axis(*x), terrain_grid_axis(*y)))
            .collect();

        let seed = self.terrain_seed.get();
        let now = now_ms();
        let mut discovered: Vec<RuntimeEnemyState> = Vec::new();
        let mut discovered_ids = HashSet::new();
        let mut reached_capacity = false;

        {
            let runtime = self.runtime.borrow();
            for (center_x, center_y) in player_centers.iter().copied() {
                for grid_x in
                    (center_x - ENEMY_SCAN_RADIUS_TILES)..=(center_x + ENEMY_SCAN_RADIUS_TILES)
                {
                    for grid_y in
                        (center_y - ENEMY_SCAN_RADIUS_TILES)..=(center_y + ENEMY_SCAN_RADIUS_TILES)
                    {
                        if runtime.enemies.len() + discovered.len() >= MAX_ENEMIES {
                            reached_capacity = true;
                            break;
                        }

                        let enemy_id = enemy_id_for_grid(grid_x, grid_y);
                        if runtime.enemies.contains_key(enemy_id.as_str())
                            || discovered_ids.contains(enemy_id.as_str())
                        {
                            continue;
                        }

                        let Some((kind, max_health, attack_power, armor)) =
                            sample_enemy_spawn(seed, grid_x, grid_y)
                        else {
                            continue;
                        };

                        let x = (grid_x as f32) * TERRAIN_TILE_SIZE as f32;
                        let y = (grid_y as f32) * TERRAIN_TILE_SIZE as f32;
                        let too_close_to_player =
                            player_positions.iter().any(|(player_x, player_y)| {
                                let dx = *player_x - x;
                                let dy = *player_y - y;
                                dx * dx + dy * dy
                                    < ENEMY_MIN_PLAYER_DISTANCE * ENEMY_MIN_PLAYER_DISTANCE
                            });
                        if too_close_to_player {
                            continue;
                        }

                        discovered_ids.insert(enemy_id.clone());
                        discovered.push(RuntimeEnemyState {
                            enemy_id,
                            kind,
                            x,
                            y,
                            vx: 0.0,
                            vy: 0.0,
                            health: max_health,
                            max_health,
                            attack_power,
                            armor,
                            target_player_id: None,
                            last_attack_tick: 0,
                            updated_at: now,
                        });
                    }

                    if reached_capacity {
                        break;
                    }
                }

                if reached_capacity {
                    break;
                }
            }
        }

        if discovered.is_empty() {
            return Ok(false);
        }

        {
            let mut runtime = self.runtime.borrow_mut();
            for enemy in discovered.into_iter() {
                runtime.enemies.insert(enemy.enemy_id.clone(), enemy);
            }
        }

        Ok(true)
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

    fn load_character_profiles(&self, user_id: &str) -> Result<Vec<CharacterProfileRow>> {
        self.sql()
            .exec(
                "
                SELECT character_id, name, sprite_id
                FROM character_profiles
                WHERE user_id = ?
                ORDER BY created_at ASC, character_id ASC
                ",
                Some(vec![user_id.into()]),
            )?
            .to_array()
    }

    fn character_profile_exists(&self, user_id: &str, character_id: &str) -> Result<bool> {
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
        Ok(!rows.is_empty())
    }

    fn character_profiles_payload(&self, user_id: &str) -> Result<Value> {
        self.ensure_default_character_profile(user_id)?;
        let active_profile = self.load_active_character_profile(user_id)?;
        let profiles = self.load_character_profiles(user_id)?;
        let profile_count = profiles.len();
        let profiles = profiles
            .into_iter()
            .map(|profile| {
                json!({
                    "characterId": profile.character_id,
                    "name": profile.name,
                    "spriteId": profile.sprite_id,
                })
            })
            .collect::<Vec<Value>>();

        Ok(json!({
            "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
            "activeCharacterId": active_profile.character_id,
            "profiles": profiles,
            "profileCount": profile_count,
        }))
    }

    fn apply_character_profiles_update(
        &self,
        user_id: &str,
        payload: CharacterProfilesUpdatePayload,
    ) -> Result<()> {
        if payload.profiles.is_empty() {
            return Err(Error::RustError(
                "profiles payload must include at least one slot".into(),
            ));
        }

        if payload.profiles.len() > MAX_CHARACTER_PROFILE_SLOTS {
            return Err(Error::RustError("too many character profile slots".into()));
        }

        let active_character_id = sanitize_character_id(payload.active_character_id.as_str())
            .ok_or_else(|| Error::RustError("invalid active character id".into()))?;
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut normalized_profiles: Vec<(String, String, String)> =
            Vec::with_capacity(payload.profiles.len());

        for profile in payload.profiles {
            let character_id = sanitize_character_id(profile.character_id.as_str())
                .ok_or_else(|| Error::RustError("invalid character id".into()))?;
            if !seen_ids.insert(character_id.clone()) {
                return Err(Error::RustError("duplicate character id".into()));
            }

            let name = sanitize_character_name(profile.name.as_str())
                .ok_or_else(|| Error::RustError("invalid character name".into()))?;
            let sprite_id = profile
                .sprite_id
                .unwrap_or_else(|| DEFAULT_CHARACTER_SPRITE_ID.to_string());

            if sprite_id.is_empty()
                || sprite_id.len() > MAX_PROTOCOL_IDENTIFIER_LEN
                || !is_valid_protocol_identifier(sprite_id.as_str())
                || !is_supported_character_sprite_id(sprite_id.as_str())
            {
                return Err(Error::RustError("invalid character sprite id".into()));
            }
            normalized_profiles.push((character_id, name, sprite_id));
        }

        let active_in_payload = normalized_profiles
            .iter()
            .any(|(character_id, _, _)| character_id.as_str() == active_character_id.as_str());
        if !active_in_payload
            && !self.character_profile_exists(user_id, active_character_id.as_str())?
        {
            return Err(Error::RustError(
                "active character profile does not exist".into(),
            ));
        }

        for (character_id, name, sprite_id) in normalized_profiles {
            self.upsert_character_profile(
                user_id,
                character_id.as_str(),
                name.as_str(),
                sprite_id.as_str(),
                false,
            )?;
        }

        self.set_active_character_profile(user_id, active_character_id.as_str())?;
        self.dirty_presence.set(true);

        Ok(())
    }

    async fn handle_character_profiles_request(
        &self,
        mut req: Request,
        url: &Url,
    ) -> Result<Response> {
        let player_id = authenticate_player(url, &self.env).await?;
        self.ensure_default_character_profile(player_id.as_str())?;

        match req.method() {
            Method::Get => json_response(self.character_profiles_payload(player_id.as_str())?, 200),
            Method::Put => {
                let raw_body = req.text().await?;
                let payload =
                    match serde_json::from_str::<CharacterProfilesUpdatePayload>(raw_body.as_str())
                    {
                        Ok(payload) => payload,
                        Err(_) => {
                            return json_response(
                                json!({ "error": "invalid character profiles payload" }),
                                400,
                            );
                        }
                    };

                match self.apply_character_profiles_update(player_id.as_str(), payload) {
                    Ok(()) => {
                        self.broadcast_snapshot(false);
                        self.dirty_presence.set(false);
                        json_response(self.character_profiles_payload(player_id.as_str())?, 200)
                    }
                    Err(error) => json_response(json!({ "error": format!("{error}") }), 400),
                }
            }
            _ => json_response(json!({ "error": "method not allowed" }), 405),
        }
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
            CREATE TABLE IF NOT EXISTS player_inventory_stacks (
              player_id TEXT NOT NULL,
              slot INTEGER NOT NULL,
              resource TEXT NOT NULL,
              amount INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              PRIMARY KEY(player_id, slot)
            )
            ",
            None,
        )?;

        sql.exec(
            "CREATE INDEX IF NOT EXISTS idx_player_inventory_player ON player_inventory_stacks(player_id)",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS mining_nodes (
              node_id TEXT PRIMARY KEY,
              kind TEXT NOT NULL,
              x REAL NOT NULL,
              y REAL NOT NULL,
              grid_x INTEGER NOT NULL,
              grid_y INTEGER NOT NULL,
              remaining INTEGER NOT NULL,
              max_yield INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            )
            ",
            None,
        )?;

        sql.exec(
            "CREATE INDEX IF NOT EXISTS idx_mining_nodes_grid ON mining_nodes(grid_x, grid_y)",
            None,
        )?;

        sql.exec(
            "
            CREATE TABLE IF NOT EXISTS world_drops (
              drop_id TEXT PRIMARY KEY,
              resource TEXT NOT NULL,
              amount INTEGER NOT NULL,
              x REAL NOT NULL,
              y REAL NOT NULL,
              owner_player_id TEXT,
              owner_expires_at INTEGER NOT NULL,
              expires_at INTEGER NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            )
            ",
            None,
        )?;

        sql.exec(
            "CREATE INDEX IF NOT EXISTS idx_world_drops_expires_at ON world_drops(expires_at)",
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

        sql.exec(
            "DELETE FROM player_inventory_stacks WHERE slot < 0 OR slot >= ? OR amount <= 0",
            Some(vec![(DEFAULT_INVENTORY_MAX_SLOTS as i64).into()]),
        )?;

        sql.exec(
            "
            DELETE FROM mining_nodes
            WHERE remaining < 0
               OR max_yield <= 0
               OR remaining > max_yield
               OR kind NOT IN ('iron_ore', 'copper_ore', 'coal')
            ",
            None,
        )?;

        sql.exec(
            "
            DELETE FROM world_drops
            WHERE amount <= 0
               OR expires_at <= ?
               OR resource NOT IN ('iron_ore', 'copper_ore', 'coal', 'stone', 'iron_plate', 'copper_plate', 'gear')
            ",
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

    fn broadcast_combat_event(&self, action: &'static str, payload: Value) {
        for socket in self.state.get_websockets() {
            self.send_envelope(
                &socket,
                "event",
                "combat",
                action,
                None,
                Some(payload.clone()),
            );
        }
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
        self.ensure_runtime_inventory_for_player(player_id);
        self.ensure_runtime_combat_for_player(player_id);
        let mining_discovered = self.materialize_mining_nodes_for_connected_players()?;
        let _combat_discovered = self.materialize_enemies_for_connected_players()?;

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
        self.dirty_inventory.set(true);
        self.dirty_crafting.set(true);
        if mining_discovered {
            self.dirty_mining.set(true);
        }
        self.dirty_combat.set(true);
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
            runtime.mining_active.remove(player_id);
            runtime.craft_queues.remove(player_id);
        }

        self.checkpoint_runtime_players_to_db()?;
        self.last_checkpoint_ms.set(now);

        self.snapshot_dirty.set(true);
        self.dirty_presence.set(true);
        self.dirty_build.set(true);
        self.dirty_inventory.set(true);
        self.dirty_mining.set(true);
        self.dirty_crafting.set(true);
        self.dirty_combat.set(true);
        Ok(())
    }

    fn handle_movement_input_batch(&self, player_id: &str, payload: Option<Value>) -> Result<bool> {
        let input_batch = validate_movement_input_batch_payload(payload)?;

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

                {
                    let mut runtime = self.runtime.borrow_mut();
                    let inventory = runtime
                        .inventories
                        .entry(player_id.to_string())
                        .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
                    inventory.normalize();
                    consume_structure_build_cost(inventory, place.kind.as_str())?;
                }
                self.persist_inventory_for_player(player_id)?;

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
                self.dirty_inventory.set(true);
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

    fn spawn_projectile(
        &self,
        owner_id: &str,
        x: f32,
        y: f32,
        vx: f32,
        vy: f32,
        client_projectile_id: Option<String>,
        now: i64,
    ) -> String {
        let (vx, vy) = clamp_projectile_velocity(vx, vy);
        let projectile_id = format!("proj_{}_{}", now, js_sys::Math::random());
        let expires_at = now + PROJECTILE_TTL_MS;
        let updated_at = now;

        self.runtime.borrow_mut().projectiles.insert(
            projectile_id.clone(),
            RuntimeProjectileState {
                projectile_id: projectile_id.clone(),
                owner_id: owner_id.to_string(),
                x,
                y,
                vx,
                vy,
                expires_at,
                client_projectile_id,
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
        projectile_id
    }

    fn handle_projectile_fire(&self, player_id: &str, payload: Option<Value>) -> Result<bool> {
        let payload =
            payload.ok_or_else(|| Error::RustError("missing projectile payload".into()))?;
        let fire: ProjectileFirePayload = serde_json::from_value(payload)
            .map_err(|_| Error::RustError("invalid projectile payload".into()))?;
        if !fire.x.is_finite()
            || !fire.y.is_finite()
            || !fire.vx.is_finite()
            || !fire.vy.is_finite()
        {
            return Err(Error::RustError("invalid projectile payload".into()));
        }
        if let Some(client_projectile_id) = fire.client_projectile_id.as_deref() {
            if !is_valid_protocol_identifier(client_projectile_id) {
                return Err(Error::RustError("invalid projectile client id".into()));
            }
        }

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

        self.spawn_projectile(
            player_id,
            fire.x as f32,
            fire.y as f32,
            fire.vx as f32,
            fire.vy as f32,
            fire.client_projectile_id,
            now,
        );
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
                let move_payload = validate_inventory_move_payload(payload)?;

                {
                    let mut runtime = self.runtime.borrow_mut();
                    let inventory = runtime
                        .inventories
                        .entry(player_id.to_string())
                        .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
                    inventory.normalize();
                    inventory.move_stack(
                        move_payload.from_slot,
                        move_payload.to_slot,
                        move_payload.amount,
                    )?;
                }

                self.persist_inventory_for_player(player_id)?;
                self.dirty_inventory.set(true);
                Ok(true)
            }
            "split" => {
                let split_payload = validate_inventory_split_payload(payload)?;

                {
                    let mut runtime = self.runtime.borrow_mut();
                    let inventory = runtime
                        .inventories
                        .entry(player_id.to_string())
                        .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
                    inventory.normalize();
                    inventory.split_stack(split_payload.slot, split_payload.amount)?;
                }

                self.persist_inventory_for_player(player_id)?;
                self.dirty_inventory.set(true);
                Ok(true)
            }
            "discard" => {
                let discard_payload = validate_inventory_discard_payload(payload)?;

                let (discarded_stack, player_x, player_y) = {
                    let mut runtime = self.runtime.borrow_mut();
                    let (player_x, player_y) = {
                        let player = runtime
                            .players
                            .entry(player_id.to_string())
                            .or_insert_with(|| Self::default_runtime_player(now));
                        player.last_seen = now;
                        (player.x, player.y)
                    };

                    let inventory = runtime
                        .inventories
                        .entry(player_id.to_string())
                        .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
                    inventory.normalize();
                    let discarded = inventory
                        .discard_from_slot(discard_payload.slot, discard_payload.amount)?;
                    (discarded, player_x, player_y)
                };

                self.persist_inventory_for_player(player_id)?;
                self.spawn_world_drop(
                    discarded_stack.resource.as_str(),
                    discarded_stack.amount,
                    player_x,
                    player_y,
                    Some(player_id),
                    now,
                )?;
                self.dirty_inventory.set(true);
                self.dirty_drops.set(true);
                Ok(true)
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
                self.materialize_mining_nodes_for_connected_players()?;

                let changed = {
                    let mut runtime = self.runtime.borrow_mut();
                    let (player_x, player_y) = {
                        let player = runtime
                            .players
                            .entry(player_id.to_string())
                            .or_insert_with(|| Self::default_runtime_player(now));
                        player.last_seen = now;
                        (player.x, player.y)
                    };

                    let node = runtime
                        .mining_nodes
                        .get(start_payload.node_id.as_str())
                        .cloned()
                        .ok_or_else(|| Error::RustError("mining node not found".into()))?;
                    if node.remaining == 0 {
                        return Err(Error::RustError("mining node is depleted".into()));
                    }
                    if !player_within_mining_range(player_x, player_y, &node) {
                        return Err(Error::RustError("mining node out of range".into()));
                    }

                    let changed = match runtime.mining_active.get(player_id) {
                        Some(existing) => existing.node_id != node.node_id,
                        None => true,
                    };
                    runtime.mining_active.insert(
                        player_id.to_string(),
                        RuntimeMiningProgressState {
                            player_id: player_id.to_string(),
                            node_id: node.node_id,
                            started_at: now,
                            completes_at: now + MINING_DURATION_MS,
                        },
                    );
                    changed
                };

                if changed {
                    self.dirty_mining.set(true);
                    self.snapshot_dirty.set(true);
                }

                Ok(changed)
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
                let removed = {
                    let mut runtime = self.runtime.borrow_mut();
                    if let Some(existing) = runtime.mining_active.get(player_id) {
                        if let Some(node_id) = cancel_payload.node_id.as_deref() {
                            if existing.node_id != node_id {
                                false
                            } else {
                                runtime.mining_active.remove(player_id).is_some()
                            }
                        } else {
                            runtime.mining_active.remove(player_id).is_some()
                        }
                    } else {
                        false
                    }
                };

                if removed {
                    self.dirty_mining.set(true);
                    self.snapshot_dirty.set(true);
                }

                Ok(removed)
            }
            _ => Err(Error::RustError("invalid mining action".into())),
        }
    }

    fn handle_drops_command(
        &self,
        player_id: &str,
        action: &str,
        payload: Option<Value>,
    ) -> Result<bool> {
        let now = now_ms();
        if self.prune_expired_drops(now)? {
            self.dirty_drops.set(true);
        }

        {
            let mut runtime = self.runtime.borrow_mut();
            let player = runtime
                .players
                .entry(player_id.to_string())
                .or_insert_with(|| Self::default_runtime_player(now));
            player.last_seen = now;
        }

        match action {
            "pickup" => {
                let payload =
                    payload.ok_or_else(|| Error::RustError("missing drop payload".into()))?;
                let pickup_payload: DropPickupPayload = serde_json::from_value(payload)
                    .map_err(|_| Error::RustError("invalid drop payload".into()))?;
                if !is_valid_protocol_identifier(pickup_payload.drop_id.as_str()) {
                    return Err(Error::RustError("invalid drop id".into()));
                }

                let picked_up = {
                    let mut runtime = self.runtime.borrow_mut();
                    let (player_x, player_y) = runtime
                        .players
                        .get(player_id)
                        .map(|player| (player.x, player.y))
                        .ok_or_else(|| Error::RustError("player state missing".into()))?;

                    let drop = runtime
                        .drops
                        .get(pickup_payload.drop_id.as_str())
                        .cloned()
                        .ok_or_else(|| Error::RustError("drop not found".into()))?;

                    if drop.expires_at <= now {
                        return Err(Error::RustError("drop expired".into()));
                    }
                    if !drop_pickup_allowed_for_player(&drop, player_id, now) {
                        return Err(Error::RustError("drop is reserved".into()));
                    }
                    if !player_within_drop_pickup_range(player_x, player_y, &drop) {
                        return Err(Error::RustError("drop out of range".into()));
                    }

                    let inventory = runtime
                        .inventories
                        .entry(player_id.to_string())
                        .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
                    inventory.normalize();
                    inventory.add_resource(drop.resource.as_str(), drop.amount)?;
                    runtime.drops.remove(pickup_payload.drop_id.as_str());
                    drop
                };

                self.persist_inventory_for_player(player_id)?;
                self.delete_drop(picked_up.drop_id.as_str())?;
                self.dirty_inventory.set(true);
                self.dirty_drops.set(true);
                Ok(true)
            }
            _ => Err(Error::RustError("invalid drop action".into())),
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
                let recipe_kind = recipe_kind_from_id(queue_payload.recipe.as_str())
                    .ok_or_else(|| Error::RustError("unknown crafting recipe".into()))?;
                let recipe_id = recipe_id_from_kind(recipe_kind).to_string();

                {
                    let mut runtime = self.runtime.borrow_mut();
                    let queue = runtime
                        .craft_queues
                        .entry(player_id.to_string())
                        .or_insert_with(RuntimeCraftQueueState::default);

                    let queued_total = queue.pending_total_count();
                    let next_total = queued_total
                        .checked_add(queue_payload.count as u32)
                        .ok_or_else(|| Error::RustError("crafting queue is full".into()))?;
                    if next_total > MAX_CRAFT_QUEUE_TOTAL_PER_PLAYER {
                        return Err(Error::RustError("crafting queue is full".into()));
                    }

                    if let Some(last_entry) = queue.pending.last_mut() {
                        if last_entry.recipe == recipe_id {
                            last_entry.count = last_entry
                                .count
                                .checked_add(queue_payload.count)
                                .ok_or_else(|| Error::RustError("crafting queue is full".into()))?;
                        } else {
                            if queue.pending.len() >= MAX_CRAFT_QUEUE_ENTRIES_PER_PLAYER {
                                return Err(Error::RustError("crafting queue is full".into()));
                            }
                            queue.pending.push(RuntimeCraftQueueEntry {
                                recipe: recipe_id,
                                count: queue_payload.count,
                            });
                        }
                    } else {
                        queue.pending.push(RuntimeCraftQueueEntry {
                            recipe: recipe_id,
                            count: queue_payload.count,
                        });
                    }
                }

                self.dirty_crafting.set(true);
                self.snapshot_dirty.set(true);
                Ok(true)
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
                let recipe_to_cancel = if clear_requested {
                    None
                } else {
                    let recipe = cancel_payload.recipe.as_deref().ok_or_else(|| {
                        Error::RustError("invalid crafting cancel payload".into())
                    })?;
                    let kind = recipe_kind_from_id(recipe)
                        .ok_or_else(|| Error::RustError("unknown crafting recipe".into()))?;
                    Some(recipe_id_from_kind(kind).to_string())
                };

                let changed = {
                    let mut runtime = self.runtime.borrow_mut();
                    let Some(mut queue) = runtime.craft_queues.remove(player_id) else {
                        return Ok(false);
                    };

                    let mut changed = false;
                    if clear_requested {
                        if !queue.pending.is_empty() || queue.active.is_some() {
                            queue.pending.clear();
                            queue.active = None;
                            changed = true;
                        }
                    } else if let Some(recipe_id) = recipe_to_cancel.as_deref() {
                        let pending_before = queue.pending.len();
                        queue.pending.retain(|entry| entry.recipe != recipe_id);
                        if queue.pending.len() != pending_before {
                            changed = true;
                        }
                        if queue
                            .active
                            .as_ref()
                            .is_some_and(|active| active.recipe == recipe_id)
                        {
                            queue.active = None;
                            changed = true;
                        }
                    }

                    if !queue.is_empty() {
                        runtime.craft_queues.insert(player_id.to_string(), queue);
                    }
                    changed
                };

                if changed {
                    self.dirty_crafting.set(true);
                    self.snapshot_dirty.set(true);
                }
                Ok(changed)
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
                if let Some(client_projectile_id) = attack_payload.client_projectile_id.as_deref() {
                    if !is_valid_protocol_identifier(client_projectile_id) {
                        return Err(Error::RustError("invalid combat projectile id".into()));
                    }
                }

                let (origin_x, origin_y, velocity_x, velocity_y) = {
                    let mut runtime = self.runtime.borrow_mut();
                    let enemy_snapshot = runtime
                        .enemies
                        .get(attack_payload.target_id.as_str())
                        .filter(|enemy| enemy.health > 0)
                        .cloned();
                    let Some(enemy_snapshot) = enemy_snapshot else {
                        return Ok(false);
                    };

                    let Some(player) = runtime.players.get_mut(player_id) else {
                        return Ok(false);
                    };
                    if now - player.last_projectile_fire_at < PROJECTILE_FIRE_MIN_INTERVAL_MS {
                        return Ok(false);
                    }

                    if !player_within_combat_attack_range(
                        player.x,
                        player.y,
                        enemy_snapshot.x,
                        enemy_snapshot.y,
                    ) {
                        return Ok(false);
                    }

                    let Some((velocity_x, velocity_y)) = projectile_velocity_towards_target(
                        player.x,
                        player.y,
                        enemy_snapshot.x,
                        enemy_snapshot.y,
                        COMBAT_ATTACK_PROJECTILE_SPEED,
                    ) else {
                        return Ok(false);
                    };

                    player.last_projectile_fire_at = now;
                    player.last_seen = now;
                    (player.x, player.y, velocity_x, velocity_y)
                };

                let projectile_id = self.spawn_projectile(
                    player_id,
                    origin_x,
                    origin_y,
                    velocity_x,
                    velocity_y,
                    attack_payload.client_projectile_id,
                    now,
                );
                self.broadcast_combat_event(
                    "player_attacked",
                    json!({
                        "attackerPlayerId": player_id,
                        "targetEnemyId": attack_payload.target_id,
                        "attackId": attack_payload.attack_id,
                        "projectileId": projectile_id,
                    }),
                );
                Ok(true)
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
                if !is_supported_character_sprite_id(sprite_id) {
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
            let combat_result = self.tick_combat()?;
            let mining_changed = self.tick_mining()?;
            let crafting_result = self.tick_crafting()?;
            let drops_changed = self.tick_drops()?;

            if movement_changed
                || projectile_changed
                || combat_result.changed
                || mining_changed
                || crafting_result.changed
                || drops_changed
            {
                self.snapshot_dirty.set(true);
                if projectile_changed {
                    self.dirty_projectiles.set(true);
                }
                if combat_result.changed {
                    self.dirty_combat.set(true);
                }
                if combat_result.projectiles_changed {
                    self.dirty_projectiles.set(true);
                }
                if combat_result.drops_changed {
                    self.dirty_drops.set(true);
                }
                if mining_changed {
                    self.dirty_mining.set(true);
                    self.dirty_inventory.set(true);
                }
                if crafting_result.changed {
                    self.dirty_crafting.set(true);
                }
                if crafting_result.inventory_changed {
                    self.dirty_inventory.set(true);
                }
                if drops_changed {
                    self.dirty_drops.set(true);
                }
            }

            if self.tick.get() % SNAPSHOT_INTERVAL_TICKS == 0 || self.snapshot_dirty.get() {
                self.broadcast_snapshot(false);
                self.snapshot_dirty.set(false);
                self.dirty_presence.set(false);
                self.dirty_build.set(false);
                self.dirty_projectiles.set(false);
                self.dirty_inventory.set(false);
                self.dirty_mining.set(false);
                self.dirty_drops.set(false);
                self.dirty_crafting.set(false);
                self.dirty_combat.set(false);
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

    fn tick_combat(&self) -> Result<CombatTickResult> {
        let now = now_ms();
        let tick = self.tick.get();
        let mut result = CombatTickResult::default();
        if self.materialize_enemies_for_connected_players()? {
            result.changed = true;
        }

        let mut drops_to_spawn: Vec<(String, u32, f32, f32, Option<String>)> = Vec::new();
        let mut combat_events: Vec<(&'static str, Value)> = Vec::new();

        {
            let mut runtime = self.runtime.borrow_mut();
            let connected_player_ids: Vec<String> = runtime
                .players
                .iter()
                .filter(|(_, player)| player.connected)
                .map(|(player_id, _)| player_id.clone())
                .collect();

            for player_id in connected_player_ids.iter() {
                runtime
                    .combat_players
                    .entry(player_id.clone())
                    .or_insert_with(RuntimePlayerCombatState::default);
            }

            if connected_player_ids.is_empty() {
                if !runtime.enemies.is_empty() {
                    runtime.enemies.clear();
                    result.changed = true;
                }
                return Ok(result);
            }

            let stale_enemy_ids: Vec<String> = runtime
                .enemies
                .values()
                .filter(|enemy| {
                    !connected_player_ids.iter().any(|player_id| {
                        runtime
                            .players
                            .get(player_id.as_str())
                            .is_some_and(|player| enemy_within_despawn_range(player, enemy))
                    })
                })
                .map(|enemy| enemy.enemy_id.clone())
                .collect();
            if !stale_enemy_ids.is_empty() {
                for enemy_id in stale_enemy_ids.iter() {
                    runtime.enemies.remove(enemy_id.as_str());
                }
                result.changed = true;
            }

            let projectile_ids: Vec<String> = runtime.projectiles.keys().cloned().collect();
            let mut defeated_enemies: Vec<(RuntimeEnemyState, Option<String>)> = Vec::new();
            for projectile_id in projectile_ids {
                let Some(projectile) = runtime.projectiles.get(projectile_id.as_str()).cloned()
                else {
                    continue;
                };

                let mut hit_enemy_id: Option<String> = None;
                let mut nearest_hit_distance_sq = f32::MAX;
                for enemy in runtime.enemies.values() {
                    if enemy.health == 0 {
                        continue;
                    }
                    let dx = projectile.x - enemy.x;
                    let dy = projectile.y - enemy.y;
                    let distance_sq = dx * dx + dy * dy;
                    if distance_sq > ENEMY_PROJECTILE_HIT_RADIUS * ENEMY_PROJECTILE_HIT_RADIUS {
                        continue;
                    }
                    if distance_sq < nearest_hit_distance_sq {
                        nearest_hit_distance_sq = distance_sq;
                        hit_enemy_id = Some(enemy.enemy_id.clone());
                    }
                }

                let Some(hit_enemy_id) = hit_enemy_id else {
                    continue;
                };

                let owner_attack_power = runtime
                    .combat_players
                    .get(projectile.owner_id.as_str())
                    .map(|stats| stats.attack_power)
                    .unwrap_or(PLAYER_COMBAT_ATTACK_POWER);
                let incoming_damage = owner_attack_power.saturating_add(PROJECTILE_BASE_DAMAGE);

                let mut defeated_enemy: Option<RuntimeEnemyState> = None;
                let mut remaining_health = 0u16;
                let mut applied_damage = 0u16;
                if let Some(enemy) = runtime.enemies.get_mut(hit_enemy_id.as_str()) {
                    let (applied, remaining, defeated) =
                        resolve_damage(enemy.health, enemy.armor, incoming_damage);
                    applied_damage = applied;
                    remaining_health = remaining;
                    enemy.health = remaining;
                    enemy.updated_at = now;
                    if defeated {
                        defeated_enemy = Some(enemy.clone());
                    }
                }

                runtime.projectiles.remove(projectile_id.as_str());
                result.projectiles_changed = true;
                result.changed = true;

                if let Some(defeated_enemy) = defeated_enemy {
                    defeated_enemies.push((defeated_enemy, Some(projectile.owner_id.clone())));
                } else if applied_damage > 0 {
                    combat_events.push((
                        "enemy_damaged",
                        json!({
                            "enemyId": hit_enemy_id,
                            "attackerPlayerId": projectile.owner_id,
                            "remainingHealth": remaining_health,
                        }),
                    ));
                }
            }

            let enemy_ids: Vec<String> = runtime.enemies.keys().cloned().collect();
            let mut pending_player_damage: HashMap<String, u16> = HashMap::new();
            for enemy_id in enemy_ids {
                let Some(enemy_snapshot) = runtime.enemies.get(enemy_id.as_str()).cloned() else {
                    continue;
                };
                if enemy_snapshot.health == 0 {
                    continue;
                }

                let mut target_player: Option<(String, f32, f32, f32)> = None;
                for player_id in connected_player_ids.iter() {
                    let Some(player) = runtime.players.get(player_id.as_str()) else {
                        continue;
                    };
                    let Some(player_combat) = runtime.combat_players.get(player_id.as_str()) else {
                        continue;
                    };
                    if player_combat.health == 0
                        || !enemy_within_aggro_range(player, &enemy_snapshot)
                    {
                        continue;
                    }

                    let dx = player.x - enemy_snapshot.x;
                    let dy = player.y - enemy_snapshot.y;
                    let distance_sq = dx * dx + dy * dy;
                    match target_player {
                        Some((_, _, _, current_best_distance_sq))
                            if distance_sq >= current_best_distance_sq => {}
                        _ => {
                            target_player =
                                Some((player_id.clone(), player.x, player.y, distance_sq));
                        }
                    }
                }

                let Some(enemy) = runtime.enemies.get_mut(enemy_id.as_str()) else {
                    continue;
                };
                let previous_target = enemy.target_player_id.clone();
                let previous_x = enemy.x;
                let previous_y = enemy.y;
                let previous_vx = enemy.vx;
                let previous_vy = enemy.vy;
                let previous_attack_tick = enemy.last_attack_tick;

                if let Some((target_player_id, target_x, target_y, target_distance_sq)) =
                    target_player
                {
                    enemy.target_player_id = Some(target_player_id.clone());

                    if target_distance_sq <= ENEMY_ATTACK_RANGE * ENEMY_ATTACK_RANGE {
                        enemy.vx = 0.0;
                        enemy.vy = 0.0;
                        if tick.saturating_sub(enemy.last_attack_tick)
                            >= ENEMY_ATTACK_COOLDOWN_TICKS
                        {
                            let total_damage =
                                pending_player_damage.entry(target_player_id).or_insert(0);
                            *total_damage = total_damage.saturating_add(enemy.attack_power);
                            enemy.last_attack_tick = tick;
                        }
                    } else {
                        let distance = target_distance_sq.sqrt().max(f32::EPSILON);
                        let direction_x = (target_x - enemy.x) / distance;
                        let direction_y = (target_y - enemy.y) / distance;
                        enemy.vx = direction_x * ENEMY_MOVE_SPEED;
                        enemy.vy = direction_y * ENEMY_MOVE_SPEED;
                        enemy.x = (enemy.x + enemy.vx * SIM_DT_SECONDS)
                            .clamp(-MOVEMENT_MAP_LIMIT, MOVEMENT_MAP_LIMIT);
                        enemy.y = (enemy.y + enemy.vy * SIM_DT_SECONDS)
                            .clamp(-MOVEMENT_MAP_LIMIT, MOVEMENT_MAP_LIMIT);
                    }
                } else {
                    enemy.target_player_id = None;
                    enemy.vx = 0.0;
                    enemy.vy = 0.0;
                }

                enemy.updated_at = now;

                if enemy.target_player_id != previous_target
                    || (enemy.x - previous_x).abs() > f32::EPSILON
                    || (enemy.y - previous_y).abs() > f32::EPSILON
                    || (enemy.vx - previous_vx).abs() > f32::EPSILON
                    || (enemy.vy - previous_vy).abs() > f32::EPSILON
                    || enemy.last_attack_tick != previous_attack_tick
                {
                    result.changed = true;
                }
            }

            for (player_id, incoming_damage) in pending_player_damage.into_iter() {
                let Some(player_combat) = runtime.combat_players.get_mut(player_id.as_str()) else {
                    continue;
                };
                let previous_health = player_combat.health;
                let (applied_damage, remaining_health, defeated) =
                    resolve_damage(previous_health, player_combat.armor, incoming_damage);
                if applied_damage == 0 {
                    continue;
                }

                player_combat.health = remaining_health;
                result.changed = true;
                combat_events.push((
                    "player_damaged",
                    json!({
                        "playerId": player_id,
                        "damage": applied_damage,
                        "remainingHealth": remaining_health,
                    }),
                ));
                if defeated {
                    combat_events.push((
                        "player_defeated",
                        json!({
                            "playerId": player_id,
                        }),
                    ));
                }
            }

            for (defeated_enemy, attacker_player_id) in defeated_enemies.into_iter() {
                if runtime
                    .enemies
                    .remove(defeated_enemy.enemy_id.as_str())
                    .is_none()
                {
                    continue;
                }

                let (drop_resource, drop_amount) =
                    enemy_drop_for_kind(defeated_enemy.kind.as_str());
                drops_to_spawn.push((
                    drop_resource.to_string(),
                    drop_amount,
                    defeated_enemy.x,
                    defeated_enemy.y,
                    attacker_player_id.clone(),
                ));
                result.changed = true;
                result.drops_changed = true;
                combat_events.push((
                    "enemy_defeated",
                    json!({
                        "enemyId": defeated_enemy.enemy_id,
                        "enemyKind": defeated_enemy.kind,
                        "x": defeated_enemy.x,
                        "y": defeated_enemy.y,
                        "byPlayerId": attacker_player_id,
                        "dropResource": drop_resource,
                        "dropAmount": drop_amount,
                    }),
                ));
            }
        }

        for (resource, amount, x, y, owner_player_id) in drops_to_spawn.iter() {
            self.spawn_world_drop(
                resource.as_str(),
                *amount,
                *x,
                *y,
                owner_player_id.as_deref(),
                now,
            )?;
        }

        for (action, payload) in combat_events.into_iter() {
            self.broadcast_combat_event(action, payload);
        }

        Ok(result)
    }

    fn tick_mining(&self) -> Result<bool> {
        let now = now_ms();
        let mut changed = false;
        let mut inventory_players_to_persist = HashSet::new();
        let mut nodes_to_persist: Vec<RuntimeMiningNodeState> = Vec::new();
        let mut depleted_node_ids: HashSet<String> = HashSet::new();
        let mut drops_to_spawn: Vec<(String, u32, f32, f32, String)> = Vec::new();

        {
            let mut runtime = self.runtime.borrow_mut();
            if runtime.mining_active.is_empty() {
                return Ok(false);
            }

            let mining_players: Vec<String> = runtime.mining_active.keys().cloned().collect();
            for active_player_id in mining_players {
                let Some(progress) = runtime
                    .mining_active
                    .get(active_player_id.as_str())
                    .cloned()
                else {
                    continue;
                };

                let Some(player) = runtime.players.get(active_player_id.as_str()) else {
                    runtime.mining_active.remove(active_player_id.as_str());
                    changed = true;
                    continue;
                };
                if !player.connected {
                    runtime.mining_active.remove(active_player_id.as_str());
                    changed = true;
                    continue;
                }

                let Some(current_node) =
                    runtime.mining_nodes.get(progress.node_id.as_str()).cloned()
                else {
                    runtime.mining_active.remove(active_player_id.as_str());
                    changed = true;
                    continue;
                };

                if current_node.remaining == 0
                    || !player_within_mining_range(player.x, player.y, &current_node)
                {
                    runtime.mining_active.remove(active_player_id.as_str());
                    changed = true;
                    continue;
                }

                if now < progress.completes_at {
                    continue;
                }

                let mined_amount = current_node.remaining.min(MINING_YIELD_PER_ACTION);
                if mined_amount == 0 {
                    runtime.mining_active.remove(active_player_id.as_str());
                    changed = true;
                    continue;
                }

                let inventory = runtime
                    .inventories
                    .entry(active_player_id.clone())
                    .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
                inventory.normalize();
                if inventory
                    .add_resource(current_node.kind.as_str(), mined_amount)
                    .is_err()
                {
                    drops_to_spawn.push((
                        current_node.kind.clone(),
                        mined_amount,
                        current_node.x,
                        current_node.y,
                        active_player_id.clone(),
                    ));
                } else {
                    inventory_players_to_persist.insert(active_player_id.clone());
                }

                let Some(node) = runtime.mining_nodes.get_mut(progress.node_id.as_str()) else {
                    runtime.mining_active.remove(active_player_id.as_str());
                    changed = true;
                    continue;
                };
                node.remaining = node.remaining.saturating_sub(mined_amount);
                node.updated_at = now;
                nodes_to_persist.push(node.clone());
                changed = true;
                let node_id = node.node_id.clone();
                let node_remaining = node.remaining;
                let _ = node;

                if node_remaining == 0 {
                    depleted_node_ids.insert(node_id);
                    runtime.mining_active.remove(active_player_id.as_str());
                } else {
                    runtime.mining_active.insert(
                        active_player_id.clone(),
                        RuntimeMiningProgressState {
                            player_id: active_player_id.clone(),
                            node_id,
                            started_at: now,
                            completes_at: now + MINING_DURATION_MS,
                        },
                    );
                }
            }

            if !depleted_node_ids.is_empty() {
                let removed_before = runtime.mining_active.len();
                runtime
                    .mining_active
                    .retain(|_, progress| !depleted_node_ids.contains(progress.node_id.as_str()));
                if runtime.mining_active.len() != removed_before {
                    changed = true;
                }
            }
        }

        for player_id in inventory_players_to_persist.iter() {
            self.persist_inventory_for_player(player_id.as_str())?;
        }
        for node in nodes_to_persist.iter() {
            self.persist_mining_node(node)?;
        }
        for (resource, amount, x, y, owner_player_id) in drops_to_spawn.iter() {
            self.spawn_world_drop(
                resource.as_str(),
                *amount,
                *x,
                *y,
                Some(owner_player_id.as_str()),
                now,
            )?;
        }

        Ok(changed)
    }

    fn tick_drops(&self) -> Result<bool> {
        let now = now_ms();
        self.prune_expired_drops(now)
    }

    fn tick_crafting(&self) -> Result<CraftingTickResult> {
        let mut result = CraftingTickResult::default();
        let mut inventory_players_to_persist: HashSet<String> = HashSet::new();

        {
            let mut runtime = self.runtime.borrow_mut();
            if runtime.craft_queues.is_empty() {
                return Ok(result);
            }

            let players: Vec<String> = runtime.craft_queues.keys().cloned().collect();
            for player_id in players {
                let Some(mut queue) = runtime.craft_queues.remove(player_id.as_str()) else {
                    continue;
                };

                let connected = runtime
                    .players
                    .get(player_id.as_str())
                    .is_some_and(|player| player.connected);
                if !connected {
                    result.changed = true;
                    continue;
                }

                let inventory = runtime
                    .inventories
                    .entry(player_id.clone())
                    .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
                let player_result = advance_crafting_queue(inventory, &mut queue);

                if player_result.inventory_changed {
                    inventory_players_to_persist.insert(player_id.clone());
                }
                result.changed |= player_result.changed;
                result.inventory_changed |= player_result.inventory_changed;

                if !queue.is_empty() {
                    runtime.craft_queues.insert(player_id, queue);
                } else if player_result.changed {
                    result.changed = true;
                }
            }
        }

        for player_id in inventory_players_to_persist.iter() {
            self.persist_inventory_for_player(player_id.as_str())?;
        }

        Ok(result)
    }

    fn snapshot_payload(&self, full: bool, recipient_player_id: Option<&str>) -> Result<Value> {
        let connected_players = self.connected_player_ids();
        let connected_set: HashSet<&str> = connected_players.iter().map(String::as_str).collect();
        let online = connected_players.clone();
        let now = now_ms();
        let mining_discovered = self.materialize_mining_nodes_for_connected_players()?;
        let enemies_discovered = self.materialize_enemies_for_connected_players()?;
        if enemies_discovered {
            self.dirty_combat.set(true);
        }
        let drops_pruned = self.prune_expired_drops(now)?;
        if drops_pruned {
            self.dirty_drops.set(true);
        }

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

        let mut inventory_players = Vec::with_capacity(connected_players.len());
        for player_id in connected_players.iter() {
            let inventory = runtime
                .inventories
                .get(player_id)
                .cloned()
                .unwrap_or_else(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
            let stacks: Vec<Value> = inventory
                .slots
                .iter()
                .enumerate()
                .filter_map(|(slot_index, stack)| {
                    stack.as_ref().map(|stack| {
                        json!({
                            "slot": slot_index,
                            "resource": stack.resource,
                            "amount": stack.amount,
                        })
                    })
                })
                .collect();

            inventory_players.push(json!({
                "playerId": player_id,
                "maxSlots": inventory.max_slots,
                "stacks": stacks,
            }));
        }

        let mut visible_mining_nodes: Vec<&RuntimeMiningNodeState> = runtime
            .mining_nodes
            .values()
            .filter(|node| node.remaining > 0)
            .filter(|node| {
                connected_players.iter().any(|player_id| {
                    runtime.players.get(player_id).is_some_and(|player| {
                        (terrain_grid_axis(player.x) - node.grid_x).abs()
                            <= MINING_NODE_SCAN_RADIUS_TILES + 2
                            && (terrain_grid_axis(player.y) - node.grid_y).abs()
                                <= MINING_NODE_SCAN_RADIUS_TILES + 2
                    })
                })
            })
            .collect();
        visible_mining_nodes.sort_by_key(|node| std::cmp::Reverse(node.updated_at));
        let visible_mining_node_count = visible_mining_nodes.len();
        if visible_mining_nodes.len() > MAX_MINING_NODES_PER_SNAPSHOT {
            visible_mining_nodes.truncate(MAX_MINING_NODES_PER_SNAPSHOT);
        }

        let mining_nodes: Vec<Value> = visible_mining_nodes
            .iter()
            .map(|node| {
                json!({
                    "id": node.node_id,
                    "kind": node.kind,
                    "x": node.x,
                    "y": node.y,
                    "remaining": node.remaining,
                })
            })
            .collect();

        let mut mining_active_rows: Vec<&RuntimeMiningProgressState> = runtime
            .mining_active
            .values()
            .filter(|progress| connected_set.contains(progress.player_id.as_str()))
            .collect();
        mining_active_rows.sort_by_key(|progress| std::cmp::Reverse(progress.started_at));
        let mining_active_count = mining_active_rows.len();
        if mining_active_rows.len() > MAX_MINING_ACTIVE_PER_SNAPSHOT {
            mining_active_rows.truncate(MAX_MINING_ACTIVE_PER_SNAPSHOT);
        }

        let mining_active: Vec<Value> = mining_active_rows
            .iter()
            .filter_map(|progress| {
                let node = runtime.mining_nodes.get(progress.node_id.as_str())?;
                if node.remaining == 0 {
                    return None;
                }
                Some(json!({
                    "playerId": progress.player_id,
                    "nodeId": progress.node_id,
                    "startedAt": progress.started_at,
                    "completesAt": progress.completes_at,
                    "progress": mining_progress_ratio(progress, now),
                }))
            })
            .collect();

        let mut visible_drop_rows: Vec<&RuntimeDropState> = runtime
            .drops
            .values()
            .filter(|drop| drop.expires_at > now)
            .filter(|drop| {
                drop_visible_to_recipient(
                    drop,
                    recipient_player_id,
                    &runtime.players,
                    &connected_players,
                )
            })
            .collect();
        visible_drop_rows.sort_by_key(|drop| std::cmp::Reverse(drop.updated_at));
        let visible_drop_count = visible_drop_rows.len();
        if visible_drop_rows.len() > MAX_DROPS_PER_SNAPSHOT {
            visible_drop_rows.truncate(MAX_DROPS_PER_SNAPSHOT);
        }

        let drops: Vec<Value> = visible_drop_rows
            .iter()
            .map(|drop| {
                json!({
                    "id": drop.drop_id,
                    "resource": drop.resource,
                    "amount": drop.amount,
                    "x": drop.x,
                    "y": drop.y,
                    "spawnedAt": drop.created_at,
                    "expiresAt": drop.expires_at,
                    "ownerPlayerId": drop.owner_player_id,
                    "ownerExpiresAt": drop.owner_expires_at,
                })
            })
            .collect();

        let mut crafting_queue_rows: Vec<(&String, &RuntimeCraftQueueState)> = runtime
            .craft_queues
            .iter()
            .filter(|(player_id, queue)| {
                connected_set.contains(player_id.as_str())
                    && (!queue.pending.is_empty() || queue.active.is_some())
            })
            .collect();
        crafting_queue_rows.sort_by(|(left_id, _), (right_id, _)| left_id.cmp(right_id));
        let crafting_queue_count = crafting_queue_rows.len();
        if crafting_queue_rows.len() > MAX_CRAFT_QUEUES_PER_SNAPSHOT {
            crafting_queue_rows.truncate(MAX_CRAFT_QUEUES_PER_SNAPSHOT);
        }

        let crafting_queues: Vec<Value> = crafting_queue_rows
            .iter()
            .map(|(player_id, queue)| {
                let pending: Vec<Value> = queue
                    .pending
                    .iter()
                    .filter(|entry| entry.count > 0)
                    .take(MAX_CRAFT_PENDING_ENTRIES_PER_SNAPSHOT_QUEUE)
                    .map(|entry| {
                        json!({
                            "recipe": entry.recipe,
                            "count": entry.count,
                        })
                    })
                    .collect();

                let active = queue.active.as_ref().map_or(Value::Null, |active| {
                    json!({
                        "recipe": active.recipe,
                        "remainingTicks": active.remaining_ticks.max(1),
                    })
                });

                json!({
                    "playerId": *player_id,
                    "pending": pending,
                    "active": active,
                })
            })
            .collect();

        let mut enemy_rows: Vec<&RuntimeEnemyState> = runtime
            .enemies
            .values()
            .filter(|enemy| enemy.health > 0)
            .collect();
        enemy_rows.sort_by_key(|row| std::cmp::Reverse(row.updated_at));
        let enemy_count = enemy_rows.len();
        if enemy_rows.len() > MAX_ENEMIES_PER_SNAPSHOT {
            enemy_rows.truncate(MAX_ENEMIES_PER_SNAPSHOT);
        }
        let enemies: Vec<Value> = enemy_rows
            .iter()
            .map(|enemy| {
                json!({
                    "id": enemy.enemy_id,
                    "kind": enemy.kind,
                    "x": enemy.x,
                    "y": enemy.y,
                    "health": enemy.health,
                    "maxHealth": enemy.max_health,
                    "targetPlayerId": enemy.target_player_id,
                })
            })
            .collect();

        let combat_players: Vec<Value> = connected_players
            .iter()
            .map(|player_id| {
                let player_combat = runtime
                    .combat_players
                    .get(player_id.as_str())
                    .cloned()
                    .unwrap_or_default();

                json!({
                    "playerId": player_id,
                    "health": player_combat.health,
                    "maxHealth": player_combat.max_health,
                    "attackPower": player_combat.attack_power,
                    "armor": player_combat.armor,
                })
            })
            .collect();

        let include_presence = full || self.dirty_presence.get();
        let include_build = full || self.dirty_build.get();
        let include_projectiles = full || self.dirty_projectiles.get();
        let include_inventory = full || self.dirty_inventory.get();
        let include_mining =
            full || self.dirty_mining.get() || mining_discovered || !mining_active.is_empty();
        // Always emit drops so delta snapshots can clear stale client-side drop state as players
        // move across visibility boundaries.
        let include_drops = true;
        let include_crafting = full || self.dirty_crafting.get();
        let include_combat = full || self.dirty_combat.get() || enemies_discovered;
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
                    "players": inventory_players,
                    "playerCount": connected_players.len(),
                }),
            );
        }

        if include_mining {
            features.insert(
                "mining".to_string(),
                json!({
                    "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
                    "nodes": mining_nodes,
                    "nodeCount": visible_mining_node_count,
                    "active": mining_active,
                    "activeCount": mining_active_count,
                }),
            );
        }

        if include_drops {
            features.insert(
                "drops".to_string(),
                json!({
                    "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
                    "drops": drops,
                    "dropCount": visible_drop_count,
                }),
            );
        }

        if include_crafting {
            features.insert(
                "crafting".to_string(),
                json!({
                    "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
                    "queues": crafting_queues,
                    "queueCount": crafting_queue_count,
                }),
            );
        }

        if include_combat {
            features.insert(
                "combat".to_string(),
                json!({
                    "schemaVersion": GAMEPLAY_SCHEMA_VERSION,
                    "enemies": enemies,
                    "enemyCount": enemy_count,
                    "players": combat_players,
                    "playerCount": connected_players.len(),
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
        let recipient_player_id = self
            .read_socket_attachment(socket)
            .map(|attachment| attachment.player_id);
        if let Ok(payload) = self.snapshot_payload(full, recipient_player_id.as_deref()) {
            self.send_envelope(socket, "snapshot", "core", "state", None, Some(payload));
        }
    }

    fn broadcast_snapshot(&self, full: bool) {
        for socket in self.state.get_websockets() {
            self.send_snapshot_to(&socket, full);
        }
    }

    fn parse_client_message(
        &self,
        message: WebSocketIncomingMessage,
    ) -> Result<ClientCommandEnvelope> {
        parse_client_command_envelope_message(message)
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
            ProtocolFeature::Drops => self.handle_drops_command(
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
                runtime
                    .inventories
                    .entry(player_id.clone())
                    .or_insert_with(|| RuntimeInventoryState::new(DEFAULT_INVENTORY_MAX_SLOTS));
                runtime
                    .combat_players
                    .entry(player_id.clone())
                    .or_insert_with(RuntimePlayerCombatState::default);
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
            dirty_inventory: Cell::new(false),
            dirty_mining: Cell::new(false),
            dirty_drops: Cell::new(false),
            dirty_crafting: Cell::new(false),
            dirty_combat: Cell::new(false),
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
        let (room_code, endpoint) = parse_room_endpoint(url.path())
            .ok_or_else(|| Error::RustError("invalid room endpoint".into()))?;

        self.room_code.replace(room_code.clone());
        self.persist_room_code(&room_code)?;
        self.ensure_terrain_seed(&room_code)?;

        match endpoint {
            RoomApiEndpoint::CharacterProfiles => {
                self.handle_character_profiles_request(req, &url).await
            }
            RoomApiEndpoint::WebSocket => {
                let upgrade = req
                    .headers()
                    .get("Upgrade")?
                    .unwrap_or_default()
                    .to_ascii_lowercase();

                if upgrade != "websocket" {
                    return json_response(json!({ "error": "WebSocket upgrade required." }), 426);
                }

                let resume_token_hint = parse_query_param(&url, "resumeToken")
                    .or_else(|| parse_query_param(&url, "resume"));
                let player_id = authenticate_player(&url, &self.env).await?;
                let resume_token =
                    self.issue_resume_token(&player_id, resume_token_hint.as_deref())?;

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
        }
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
                    self.dirty_inventory.set(false);
                    self.dirty_mining.set(false);
                    self.dirty_drops.set(false);
                    self.dirty_crafting.set(false);
                    self.dirty_combat.set(false);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_error_contains<T: std::fmt::Debug>(result: Result<T>, expected_substring: &str) {
        let error = result.expect_err("operation should fail");
        let message = format!("{error}");
        assert!(
            message.contains(expected_substring),
            "expected error containing `{expected_substring}`, got `{message}`"
        );
    }

    fn slot_state(inventory: &RuntimeInventoryState, slot: usize) -> Option<(String, u32)> {
        inventory
            .slots
            .get(slot)
            .and_then(|stack| stack.as_ref())
            .map(|stack| (stack.resource.clone(), stack.amount))
    }

    fn resource_total(inventory: &RuntimeInventoryState, resource: &str) -> u32 {
        inventory.total_resource_amount(resource)
    }

    #[test]
    fn protocol_envelope_parser_accepts_valid_command() {
        let message = WebSocketIncomingMessage::String(
            json!({
                "v": PROTOCOL_VERSION,
                "kind": "command",
                "seq": 42,
                "feature": "inventory",
                "action": "move",
                "clientTime": 123.5,
                "payload": {
                    "fromSlot": 0,
                    "toSlot": 1
                }
            })
            .to_string(),
        );

        let envelope =
            parse_client_command_envelope_message(message).expect("envelope should parse");
        assert_eq!(envelope.seq, 42);
        assert_eq!(envelope.feature, ProtocolFeature::Inventory);
        assert_eq!(envelope.action, "move");
    }

    #[test]
    fn protocol_envelope_parser_rejects_malformed_and_invalid_messages() {
        assert_error_contains(
            parse_client_command_envelope_message(WebSocketIncomingMessage::Binary(vec![1, 2, 3])),
            "binary websocket payloads are not supported",
        );

        assert_error_contains(
            parse_client_command_envelope_message(WebSocketIncomingMessage::String(
                "not valid json".to_string(),
            )),
            "malformed protocol envelope",
        );

        let oversized = "x".repeat((32 * 1024) + 1);
        assert_error_contains(
            parse_client_command_envelope_message(WebSocketIncomingMessage::String(oversized)),
            "protocol envelope too large",
        );

        let invalid_seq = WebSocketIncomingMessage::String(
            json!({
                "v": PROTOCOL_VERSION,
                "kind": "command",
                "seq": 0,
                "feature": "movement",
                "action": "input_batch",
                "clientTime": 1.0,
                "payload": {
                    "inputs": []
                }
            })
            .to_string(),
        );
        assert_error_contains(
            parse_client_command_envelope_message(invalid_seq),
            "invalid protocol envelope",
        );

        let invalid_action = WebSocketIncomingMessage::String(
            json!({
                "v": PROTOCOL_VERSION,
                "kind": "command",
                "seq": 1,
                "feature": "movement",
                "action": "bad action",
                "clientTime": 1.0,
                "payload": {
                    "inputs": []
                }
            })
            .to_string(),
        );
        assert_error_contains(
            parse_client_command_envelope_message(invalid_action),
            "invalid protocol envelope",
        );
    }

    #[test]
    fn movement_payload_validator_rejects_invalid_shapes() {
        assert_error_contains(
            validate_movement_input_batch_payload(None),
            "missing movement payload",
        );

        assert_error_contains(
            validate_movement_input_batch_payload(Some(json!({
                "inputs": "bad-shape"
            }))),
            "invalid movement payload",
        );

        let oversized_inputs = (1..=129)
            .map(|seq| {
                json!({
                    "seq": seq,
                    "up": false,
                    "down": false,
                    "left": false,
                    "right": false
                })
            })
            .collect::<Vec<Value>>();
        assert_error_contains(
            validate_movement_input_batch_payload(Some(json!({
                "inputs": oversized_inputs
            }))),
            "input batch too large",
        );
    }

    #[test]
    fn inventory_payload_validators_reject_malformed_payloads() {
        assert_error_contains(
            validate_inventory_move_payload(Some(json!({
                "fromSlot": 2,
                "toSlot": 2
            }))),
            "invalid inventory move payload",
        );
        assert_error_contains(
            validate_inventory_move_payload(Some(json!({
                "fromSlot": 0,
                "toSlot": 1,
                "amount": 0
            }))),
            "invalid inventory move payload",
        );
        assert_error_contains(
            validate_inventory_split_payload(Some(json!({
                "slot": 0,
                "amount": 0
            }))),
            "invalid inventory split payload",
        );
        assert_error_contains(
            validate_inventory_discard_payload(Some(json!({
                "slot": 0,
                "amount": 0
            }))),
            "invalid inventory discard payload",
        );
    }

    #[test]
    fn inventory_add_and_remove_resources() {
        let mut inventory = RuntimeInventoryState::new(4);
        inventory.add_resource("iron_ore", 12).expect("add iron");
        inventory.add_resource("iron_ore", 8).expect("merge iron");
        inventory.add_resource("stone", 5).expect("add stone");

        assert_eq!(
            slot_state(&inventory, 0),
            Some(("iron_ore".to_string(), 20))
        );
        assert_eq!(slot_state(&inventory, 1), Some(("stone".to_string(), 5)));

        inventory
            .remove_resource("iron_ore", 7)
            .expect("remove iron");
        assert_eq!(
            slot_state(&inventory, 0),
            Some(("iron_ore".to_string(), 13))
        );
    }

    #[test]
    fn inventory_move_merge_split_and_discard() {
        let mut inventory = RuntimeInventoryState::new(4);
        inventory.add_resource("iron_ore", 10).expect("seed iron");
        inventory.add_resource("stone", 3).expect("seed stone");

        inventory.move_stack(1, 2, None).expect("move stone");
        assert_eq!(slot_state(&inventory, 1), None);
        assert_eq!(slot_state(&inventory, 2), Some(("stone".to_string(), 3)));

        inventory.split_stack(0, 4).expect("split iron");
        assert_eq!(slot_state(&inventory, 0), Some(("iron_ore".to_string(), 6)));
        assert_eq!(slot_state(&inventory, 1), Some(("iron_ore".to_string(), 4)));

        inventory.move_stack(1, 0, None).expect("merge split stack");
        assert_eq!(
            slot_state(&inventory, 0),
            Some(("iron_ore".to_string(), 10))
        );
        assert_eq!(slot_state(&inventory, 1), None);

        inventory
            .discard_from_slot(0, Some(6))
            .expect("discard partial stack");
        assert_eq!(slot_state(&inventory, 0), Some(("iron_ore".to_string(), 4)));

        inventory
            .discard_from_slot(0, None)
            .expect("discard full stack");
        assert_eq!(slot_state(&inventory, 0), None);
    }

    #[test]
    fn inventory_rejects_partial_swap_with_different_resource() {
        let mut inventory = RuntimeInventoryState::new(3);
        inventory.add_resource("iron_ore", 10).expect("seed iron");
        inventory.add_resource("stone", 2).expect("seed stone");

        let error = inventory
            .move_stack(0, 1, Some(4))
            .expect_err("partial swap should fail");
        assert!(format!("{error}").contains("different resource"));
    }

    #[test]
    fn structure_build_cost_consumes_inventory_requirements() {
        let mut inventory = RuntimeInventoryState::new(8);
        inventory.add_resource("iron_plate", 16).expect("seed iron");
        inventory
            .add_resource("copper_plate", 2)
            .expect("seed copper");
        inventory.add_resource("gear", 7).expect("seed gear");

        consume_structure_build_cost(&mut inventory, "beacon").expect("consume beacon cost");
        assert_eq!(resource_total(&inventory, "iron_plate"), 14);
        assert_eq!(resource_total(&inventory, "copper_plate"), 1);
        assert_eq!(resource_total(&inventory, "gear"), 7);

        consume_structure_build_cost(&mut inventory, "assembler").expect("consume assembler cost");
        assert_eq!(resource_total(&inventory, "iron_plate"), 5);
        assert_eq!(resource_total(&inventory, "gear"), 2);
    }

    #[test]
    fn structure_build_cost_rejects_without_partial_inventory_mutation() {
        let mut inventory = RuntimeInventoryState::new(8);
        inventory.add_resource("iron_plate", 9).expect("seed iron");
        inventory.add_resource("gear", 4).expect("seed gear");

        let error = consume_structure_build_cost(&mut inventory, "assembler")
            .expect_err("assembler build should fail without enough gear");
        assert!(format!("{error}").contains("insufficient inventory resources"));
        assert_eq!(resource_total(&inventory, "iron_plate"), 9);
        assert_eq!(resource_total(&inventory, "gear"), 4);
    }

    #[test]
    fn mining_node_identifier_uses_protocol_safe_shape() {
        let node_id = mining_node_id_for_grid(-12, 48);
        assert_eq!(node_id, "node:-12:48");
        assert!(is_valid_protocol_identifier(node_id.as_str()));
    }

    #[test]
    fn mining_progress_ratio_clamps_to_valid_range() {
        let progress = RuntimeMiningProgressState {
            player_id: "p1".to_string(),
            node_id: "node:0:0".to_string(),
            started_at: 1_000,
            completes_at: 2_000,
        };

        assert_eq!(mining_progress_ratio(&progress, 0), 0.0);
        assert!((mining_progress_ratio(&progress, 1_500) - 0.5).abs() < f32::EPSILON);
        assert_eq!(mining_progress_ratio(&progress, 3_000), 1.0);
    }

    #[test]
    fn drop_pickup_owner_grace_is_enforced() {
        let drop = RuntimeDropState {
            drop_id: "drop:test".to_string(),
            resource: "iron_ore".to_string(),
            amount: 3,
            x: 0.0,
            y: 0.0,
            owner_player_id: Some("owner".to_string()),
            owner_expires_at: 10_000,
            expires_at: 30_000,
            created_at: 0,
            updated_at: 0,
        };

        assert!(drop_pickup_allowed_for_player(&drop, "owner", 2_000));
        assert!(!drop_pickup_allowed_for_player(&drop, "other", 9_999));
        assert!(drop_pickup_allowed_for_player(&drop, "other", 10_000));
    }

    #[test]
    fn drop_pickup_range_uses_distance_check() {
        let drop = RuntimeDropState {
            drop_id: "drop:test".to_string(),
            resource: "iron_ore".to_string(),
            amount: 1,
            x: 0.0,
            y: 0.0,
            owner_player_id: None,
            owner_expires_at: 0,
            expires_at: 30_000,
            created_at: 0,
            updated_at: 0,
        };

        assert!(player_within_drop_pickup_range(
            DROP_PICKUP_RANGE,
            0.0,
            &drop
        ));
        assert!(!player_within_drop_pickup_range(
            DROP_PICKUP_RANGE + 0.05,
            0.0,
            &drop
        ));
    }

    #[test]
    fn drop_visibility_range_tracks_terrain_grid_radius() {
        let player = RuntimePlayerState {
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
            connected: true,
            last_seen: 0,
            last_preview_cmd_at: 0,
            last_place_cmd_at: 0,
            last_projectile_fire_at: 0,
        };

        let visible_drop = RuntimeDropState {
            drop_id: "drop:visible".to_string(),
            resource: "iron_ore".to_string(),
            amount: 1,
            x: ((MINING_NODE_SCAN_RADIUS_TILES + 4) * TERRAIN_TILE_SIZE as i32) as f32,
            y: 0.0,
            owner_player_id: None,
            owner_expires_at: 0,
            expires_at: 30_000,
            created_at: 0,
            updated_at: 0,
        };
        assert!(player_within_drop_visibility_range(&player, &visible_drop));

        let hidden_drop = RuntimeDropState {
            drop_id: "drop:hidden".to_string(),
            resource: "iron_ore".to_string(),
            amount: 1,
            x: ((MINING_NODE_SCAN_RADIUS_TILES + 5) * TERRAIN_TILE_SIZE as i32) as f32,
            y: 0.0,
            owner_player_id: None,
            owner_expires_at: 0,
            expires_at: 30_000,
            created_at: 0,
            updated_at: 0,
        };
        assert!(!player_within_drop_visibility_range(&player, &hidden_drop));
    }

    #[test]
    fn drop_visibility_scope_respects_snapshot_recipient() {
        let mut local_player = RoomDurableObject::default_runtime_player(0);
        local_player.connected = true;
        local_player.x = 0.0;
        local_player.y = 0.0;

        let mut remote_player = RoomDurableObject::default_runtime_player(0);
        remote_player.connected = true;
        remote_player.x = ((MINING_NODE_SCAN_RADIUS_TILES + 7) * TERRAIN_TILE_SIZE as i32) as f32;
        remote_player.y = 0.0;

        let mut players = HashMap::new();
        players.insert("local".to_string(), local_player);
        players.insert("remote".to_string(), remote_player);

        let drop = RuntimeDropState {
            drop_id: "drop:test".to_string(),
            resource: "gear".to_string(),
            amount: 1,
            x: 0.0,
            y: 0.0,
            owner_player_id: Some("local".to_string()),
            owner_expires_at: 5_000,
            expires_at: 30_000,
            created_at: 0,
            updated_at: 0,
        };

        let all_connected = vec!["local".to_string(), "remote".to_string()];
        assert!(drop_visible_to_recipient(
            &drop,
            Some("local"),
            &players,
            &all_connected
        ));
        assert!(!drop_visible_to_recipient(
            &drop,
            Some("remote"),
            &players,
            &all_connected
        ));
        assert!(drop_visible_to_recipient(
            &drop,
            None,
            &players,
            &all_connected
        ));

        let remote_only_connected = vec!["remote".to_string()];
        assert!(!drop_visible_to_recipient(
            &drop,
            None,
            &players,
            &remote_only_connected
        ));
    }

    #[test]
    fn crafting_tick_requires_input_resources_before_starting() {
        let mut inventory = RuntimeInventoryState::new(4);
        let mut queue = RuntimeCraftQueueState {
            pending: vec![RuntimeCraftQueueEntry {
                recipe: "craft_gear".to_string(),
                count: 1,
            }],
            active: None,
        };

        let tick_result = advance_crafting_queue(&mut inventory, &mut queue);
        assert!(!tick_result.changed);
        assert!(!tick_result.inventory_changed);
        assert_eq!(queue.pending_total_count(), 1);
        assert!(queue.active.is_none());
    }

    #[test]
    fn crafting_tick_supports_repeated_queue_and_outputs_inventory() {
        let mut inventory = RuntimeInventoryState::new(8);
        inventory.add_resource("iron_ore", 2).expect("seed ore");

        let mut queue = RuntimeCraftQueueState {
            pending: vec![RuntimeCraftQueueEntry {
                recipe: "smelt_iron_plate".to_string(),
                count: 2,
            }],
            active: None,
        };

        let tick_1 = advance_crafting_queue(&mut inventory, &mut queue);
        assert!(tick_1.changed);
        assert!(tick_1.inventory_changed);
        assert_eq!(resource_total(&inventory, "iron_ore"), 1);
        assert_eq!(queue.pending_total_count(), 1);
        assert!(queue.active.is_some());

        let tick_2 = advance_crafting_queue(&mut inventory, &mut queue);
        assert!(tick_2.changed);
        assert!(tick_2.inventory_changed);
        assert_eq!(resource_total(&inventory, "iron_plate"), 1);
        assert!(queue.active.is_none());
        assert_eq!(queue.pending_total_count(), 1);

        let tick_3 = advance_crafting_queue(&mut inventory, &mut queue);
        assert!(tick_3.changed);
        assert!(tick_3.inventory_changed);
        assert_eq!(resource_total(&inventory, "iron_ore"), 0);
        assert!(queue.active.is_some());

        let tick_4 = advance_crafting_queue(&mut inventory, &mut queue);
        assert!(tick_4.changed);
        assert!(tick_4.inventory_changed);
        assert_eq!(resource_total(&inventory, "iron_plate"), 2);
        assert!(queue.is_empty());
    }

    #[test]
    fn crafting_recipe_ids_map_to_supported_domain_recipes() {
        for recipe_id in ["smelt_iron_plate", "smelt_copper_plate", "craft_gear"] {
            let recipe = recipe_kind_from_id(recipe_id).expect("recipe should be supported");
            assert_eq!(recipe_id_from_kind(recipe), recipe_id);
        }

        assert!(recipe_kind_from_id("unknown_recipe").is_none());
    }

    #[test]
    fn enemy_spawn_sampling_is_deterministic_per_seed_and_grid() {
        let seed = deterministic_seed_from_room_code("ALPHA-ROOM");
        let sample_a = sample_enemy_spawn(seed, 12, -9);
        let sample_b = sample_enemy_spawn(seed, 12, -9);
        assert_eq!(sample_a, sample_b);

        let changed_seed = deterministic_seed_from_room_code("BRAVO-ROOM");
        let changed_sample = sample_enemy_spawn(changed_seed, 12, -9);
        if sample_a.is_some() && changed_sample.is_some() {
            assert_ne!(sample_a, changed_sample);
        }
    }

    #[test]
    fn damage_resolution_applies_armor_and_detects_defeat() {
        let (applied_1, remaining_1, defeated_1) = resolve_damage(40, 3, 9);
        assert_eq!(applied_1, 6);
        assert_eq!(remaining_1, 34);
        assert!(!defeated_1);

        let (applied_2, remaining_2, defeated_2) = resolve_damage(5, 10, 1);
        assert_eq!(applied_2, 1);
        assert_eq!(remaining_2, 4);
        assert!(!defeated_2);

        let (applied_3, remaining_3, defeated_3) = resolve_damage(4, 0, 12);
        assert_eq!(applied_3, 4);
        assert_eq!(remaining_3, 0);
        assert!(defeated_3);
    }

    #[test]
    fn projectile_velocity_clamp_enforces_max_speed() {
        let (vx, vy) = clamp_projectile_velocity(3_000.0, 4_000.0);
        let speed = (vx * vx + vy * vy).sqrt();
        assert!(speed <= PROJECTILE_MAX_SPEED as f32 + 0.001);

        let (small_vx, small_vy) = clamp_projectile_velocity(150.0, -90.0);
        assert!((small_vx - 150.0).abs() < f32::EPSILON);
        assert!((small_vy + 90.0).abs() < f32::EPSILON);
    }

    #[test]
    fn combat_attack_range_gate_uses_distance() {
        assert!(player_within_combat_attack_range(
            0.0,
            0.0,
            COMBAT_ATTACK_RANGE - 1.0,
            0.0
        ));
        assert!(!player_within_combat_attack_range(
            0.0,
            0.0,
            COMBAT_ATTACK_RANGE + 1.0,
            0.0
        ));
    }

    #[test]
    fn projectile_velocity_towards_target_normalizes_direction() {
        let (vx, vy) = projectile_velocity_towards_target(10.0, 20.0, 13.0, 24.0, 200.0)
            .expect("velocity should be generated");
        assert!((vx - 120.0).abs() < 0.001);
        assert!((vy - 160.0).abs() < 0.001);

        assert!(
            projectile_velocity_towards_target(4.0, 4.0, 4.0, 4.0, 200.0).is_none(),
            "zero-distance targeting should not generate velocity"
        );
    }
}

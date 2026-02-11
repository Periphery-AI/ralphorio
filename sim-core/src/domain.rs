use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const GAMEPLAY_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_INVENTORY_MAX_SLOTS: u8 = 24;

const fn schema_version_default() -> u32 {
    GAMEPLAY_SCHEMA_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    IronOre,
    CopperOre,
    Coal,
    Stone,
    IronPlate,
    CopperPlate,
    Gear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeKind {
    SmeltIronPlate,
    SmeltCopperPlate,
    CraftGear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceableKind {
    BurnerDrill,
    StoneFurnace,
    WoodenChest,
    AssemblerMk1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStack {
    pub resource: ResourceKind,
    pub amount: u32,
}

impl ResourceStack {
    pub fn new(resource: ResourceKind, amount: u32) -> Self {
        Self { resource, amount }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryState {
    pub max_slots: u8,
    pub stacks: Vec<ResourceStack>,
}

impl Default for InventoryState {
    fn default() -> Self {
        Self {
            max_slots: DEFAULT_INVENTORY_MAX_SLOTS,
            stacks: Vec::new(),
        }
    }
}

impl InventoryState {
    pub fn new(max_slots: u8) -> Self {
        Self {
            max_slots,
            stacks: Vec::new(),
        }
    }

    pub fn count(&self, resource: ResourceKind) -> u32 {
        self.stacks
            .iter()
            .filter(|stack| stack.resource == resource)
            .map(|stack| stack.amount)
            .sum()
    }

    pub fn can_afford(&self, requirements: &[ResourceStack]) -> bool {
        requirements
            .iter()
            .all(|requirement| self.count(requirement.resource) >= requirement.amount)
    }

    pub fn add_resource(
        &mut self,
        resource: ResourceKind,
        amount: u32,
    ) -> Result<(), InventoryOpError> {
        if amount == 0 {
            return Err(InventoryOpError::InvalidAmount);
        }

        if let Some(stack) = self
            .stacks
            .iter_mut()
            .find(|stack| stack.resource == resource)
        {
            stack.amount = stack.amount.saturating_add(amount);
            return Ok(());
        }

        if self.stacks.len() >= self.max_slots as usize {
            return Err(InventoryOpError::NoFreeSlot);
        }

        self.stacks.push(ResourceStack::new(resource, amount));
        Ok(())
    }

    pub fn remove_resource(
        &mut self,
        resource: ResourceKind,
        amount: u32,
    ) -> Result<(), InventoryOpError> {
        if amount == 0 {
            return Err(InventoryOpError::InvalidAmount);
        }

        let available = self.count(resource);
        if available < amount {
            return Err(InventoryOpError::InsufficientResource {
                resource,
                required: amount,
                available,
            });
        }

        let mut remaining = amount;
        for stack in self
            .stacks
            .iter_mut()
            .filter(|stack| stack.resource == resource)
        {
            if remaining == 0 {
                break;
            }
            let to_take = stack.amount.min(remaining);
            stack.amount -= to_take;
            remaining -= to_take;
        }

        self.stacks.retain(|stack| stack.amount > 0);
        Ok(())
    }

    pub fn consume_requirements(
        &mut self,
        requirements: &[ResourceStack],
    ) -> Result<(), InventoryOpError> {
        if !self.can_afford(requirements) {
            if let Some(missing) = requirements
                .iter()
                .find(|req| self.count(req.resource) < req.amount)
            {
                return Err(InventoryOpError::InsufficientResource {
                    resource: missing.resource,
                    required: missing.amount,
                    available: self.count(missing.resource),
                });
            }
            return Err(InventoryOpError::InvalidAmount);
        }

        for requirement in requirements {
            self.remove_resource(requirement.resource, requirement.amount)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryOpError {
    InvalidAmount,
    NoFreeSlot,
    InsufficientResource {
        resource: ResourceKind,
        required: u32,
        available: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombatStats {
    pub max_health: u16,
    pub health: u16,
    pub attack_power: u16,
    pub armor: u16,
}

impl CombatStats {
    pub fn new(max_health: u16, attack_power: u16, armor: u16) -> Self {
        Self {
            max_health,
            health: max_health,
            attack_power,
            armor,
        }
    }

    pub fn apply_damage(&mut self, incoming: u16) -> DamageResolution {
        let mitigated = incoming.saturating_sub(self.armor);
        let applied = mitigated.min(self.health);
        self.health = self.health.saturating_sub(applied);
        DamageResolution {
            applied,
            remaining_health: self.health,
            defeated: self.health == 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DamageResolution {
    pub applied: u16,
    pub remaining_health: u16,
    pub defeated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceableEntity {
    pub id: u64,
    pub owner_id: String,
    pub kind: PlaceableKind,
    pub tile_x: i32,
    pub tile_y: i32,
    pub max_health: u16,
    pub health: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedCraft {
    pub recipe: RecipeKind,
    pub count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveCraft {
    pub recipe: RecipeKind,
    pub remaining_ticks: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CraftQueueState {
    pub pending: Vec<QueuedCraft>,
    pub active: Option<ActiveCraft>,
}

impl CraftQueueState {
    fn peek_recipe(&self) -> Option<RecipeKind> {
        self.pending.first().map(|queued| queued.recipe)
    }

    fn consume_one_pending(&mut self) -> Option<RecipeKind> {
        let first = self.pending.first_mut()?;
        if first.count == 0 {
            self.pending.remove(0);
            return self.consume_one_pending();
        }

        first.count -= 1;
        let recipe = first.recipe;
        if first.count == 0 {
            self.pending.remove(0);
        }

        Some(recipe)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameplayState {
    #[serde(default = "schema_version_default")]
    pub schema_version: u32,
    pub tick: u64,
    pub inventories: BTreeMap<String, InventoryState>,
    pub craft_queues: BTreeMap<String, CraftQueueState>,
    pub combatants: BTreeMap<String, CombatStats>,
    pub placeables: BTreeMap<u64, PlaceableEntity>,
    pub next_placeable_id: u64,
}

impl Default for GameplayState {
    fn default() -> Self {
        Self {
            schema_version: GAMEPLAY_SCHEMA_VERSION,
            tick: 0,
            inventories: BTreeMap::new(),
            craft_queues: BTreeMap::new(),
            combatants: BTreeMap::new(),
            placeables: BTreeMap::new(),
            next_placeable_id: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationStepInput {
    #[serde(default = "schema_version_default")]
    pub schema_version: u32,
    pub tick: u64,
    #[serde(default)]
    pub commands: Vec<SimulationCommand>,
}

impl Default for SimulationStepInput {
    fn default() -> Self {
        Self {
            schema_version: GAMEPLAY_SCHEMA_VERSION,
            tick: 1,
            commands: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SimulationCommand {
    GrantResource {
        actor_id: String,
        resource: ResourceKind,
        amount: u32,
    },
    QueueCraft {
        actor_id: String,
        recipe: RecipeKind,
        count: u16,
    },
    RegisterCombatant {
        actor_id: String,
        stats: CombatStats,
    },
    DealDamage {
        target_id: String,
        amount: u16,
    },
    PlaceEntity {
        actor_id: String,
        kind: PlaceableKind,
        tile_x: i32,
        tile_y: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationStepOutput {
    #[serde(default = "schema_version_default")]
    pub schema_version: u32,
    pub tick: u64,
    pub events: Vec<SimulationEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SimulationEvent {
    ResourceGranted {
        actor_id: String,
        resource: ResourceKind,
        amount: u32,
    },
    CraftQueued {
        actor_id: String,
        recipe: RecipeKind,
        count: u16,
    },
    CraftStarted {
        actor_id: String,
        recipe: RecipeKind,
        remaining_ticks: u16,
    },
    CraftCompleted {
        actor_id: String,
        recipe: RecipeKind,
    },
    PlaceablePlaced {
        placeable_id: u64,
        owner_id: String,
        kind: PlaceableKind,
        tile_x: i32,
        tile_y: i32,
    },
    CombatantRegistered {
        actor_id: String,
    },
    DamageApplied {
        target_id: String,
        applied: u16,
        remaining_health: u16,
        defeated: bool,
    },
    Rejected {
        command_index: Option<usize>,
        reason: SimulationRejectReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SimulationRejectReason {
    SchemaVersionMismatch {
        expected: u32,
        received: u32,
    },
    TickNotAdvancing {
        last_tick: u64,
        received_tick: u64,
    },
    InvalidAmount,
    InventoryNoFreeSlot {
        actor_id: String,
    },
    InsufficientResource {
        actor_id: String,
        resource: ResourceKind,
        required: u32,
        available: u32,
    },
    OccupiedTile {
        tile_x: i32,
        tile_y: i32,
    },
    UnknownCombatant {
        target_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeDefinition {
    pub craft_ticks: u16,
    pub inputs: Vec<ResourceStack>,
    pub outputs: Vec<ResourceStack>,
}

pub fn recipe_definition(recipe: RecipeKind) -> RecipeDefinition {
    match recipe {
        RecipeKind::SmeltIronPlate => RecipeDefinition {
            craft_ticks: 2,
            inputs: vec![ResourceStack::new(ResourceKind::IronOre, 1)],
            outputs: vec![ResourceStack::new(ResourceKind::IronPlate, 1)],
        },
        RecipeKind::SmeltCopperPlate => RecipeDefinition {
            craft_ticks: 2,
            inputs: vec![ResourceStack::new(ResourceKind::CopperOre, 1)],
            outputs: vec![ResourceStack::new(ResourceKind::CopperPlate, 1)],
        },
        RecipeKind::CraftGear => RecipeDefinition {
            craft_ticks: 3,
            inputs: vec![ResourceStack::new(ResourceKind::IronPlate, 2)],
            outputs: vec![ResourceStack::new(ResourceKind::Gear, 1)],
        },
    }
}

pub fn placeable_build_cost(kind: PlaceableKind) -> Vec<ResourceStack> {
    match kind {
        PlaceableKind::BurnerDrill => vec![
            ResourceStack::new(ResourceKind::IronPlate, 3),
            ResourceStack::new(ResourceKind::Gear, 2),
        ],
        PlaceableKind::StoneFurnace => vec![ResourceStack::new(ResourceKind::Stone, 6)],
        PlaceableKind::WoodenChest => vec![ResourceStack::new(ResourceKind::Stone, 2)],
        PlaceableKind::AssemblerMk1 => vec![
            ResourceStack::new(ResourceKind::IronPlate, 9),
            ResourceStack::new(ResourceKind::Gear, 5),
        ],
    }
}

fn placeable_max_health(kind: PlaceableKind) -> u16 {
    match kind {
        PlaceableKind::WoodenChest => 120,
        PlaceableKind::StoneFurnace => 180,
        PlaceableKind::BurnerDrill => 220,
        PlaceableKind::AssemblerMk1 => 320,
    }
}

pub fn simulate_step(
    state: &mut GameplayState,
    input: SimulationStepInput,
) -> SimulationStepOutput {
    let mut events = Vec::new();

    if state.schema_version != GAMEPLAY_SCHEMA_VERSION {
        events.push(SimulationEvent::Rejected {
            command_index: None,
            reason: SimulationRejectReason::SchemaVersionMismatch {
                expected: GAMEPLAY_SCHEMA_VERSION,
                received: state.schema_version,
            },
        });

        return SimulationStepOutput {
            schema_version: GAMEPLAY_SCHEMA_VERSION,
            tick: state.tick,
            events,
        };
    }

    if input.schema_version != GAMEPLAY_SCHEMA_VERSION {
        events.push(SimulationEvent::Rejected {
            command_index: None,
            reason: SimulationRejectReason::SchemaVersionMismatch {
                expected: GAMEPLAY_SCHEMA_VERSION,
                received: input.schema_version,
            },
        });

        return SimulationStepOutput {
            schema_version: GAMEPLAY_SCHEMA_VERSION,
            tick: state.tick,
            events,
        };
    }

    if input.tick <= state.tick {
        events.push(SimulationEvent::Rejected {
            command_index: None,
            reason: SimulationRejectReason::TickNotAdvancing {
                last_tick: state.tick,
                received_tick: input.tick,
            },
        });

        return SimulationStepOutput {
            schema_version: GAMEPLAY_SCHEMA_VERSION,
            tick: state.tick,
            events,
        };
    }

    state.tick = input.tick;

    for (command_index, command) in input.commands.into_iter().enumerate() {
        match command {
            SimulationCommand::GrantResource {
                actor_id,
                resource,
                amount,
            } => {
                let inventory = state
                    .inventories
                    .entry(actor_id.clone())
                    .or_insert_with(InventoryState::default);
                match inventory.add_resource(resource, amount) {
                    Ok(()) => events.push(SimulationEvent::ResourceGranted {
                        actor_id,
                        resource,
                        amount,
                    }),
                    Err(InventoryOpError::InvalidAmount) => {
                        events.push(SimulationEvent::Rejected {
                            command_index: Some(command_index),
                            reason: SimulationRejectReason::InvalidAmount,
                        })
                    }
                    Err(InventoryOpError::NoFreeSlot) => events.push(SimulationEvent::Rejected {
                        command_index: Some(command_index),
                        reason: SimulationRejectReason::InventoryNoFreeSlot { actor_id },
                    }),
                    Err(InventoryOpError::InsufficientResource { .. }) => {
                        events.push(SimulationEvent::Rejected {
                            command_index: Some(command_index),
                            reason: SimulationRejectReason::InvalidAmount,
                        })
                    }
                }
            }
            SimulationCommand::QueueCraft {
                actor_id,
                recipe,
                count,
            } => {
                if count == 0 {
                    events.push(SimulationEvent::Rejected {
                        command_index: Some(command_index),
                        reason: SimulationRejectReason::InvalidAmount,
                    });
                    continue;
                }

                state
                    .craft_queues
                    .entry(actor_id.clone())
                    .or_default()
                    .pending
                    .push(QueuedCraft { recipe, count });

                events.push(SimulationEvent::CraftQueued {
                    actor_id,
                    recipe,
                    count,
                });
            }
            SimulationCommand::RegisterCombatant { actor_id, stats } => {
                state.combatants.insert(actor_id.clone(), stats);
                events.push(SimulationEvent::CombatantRegistered { actor_id });
            }
            SimulationCommand::DealDamage { target_id, amount } => {
                if amount == 0 {
                    events.push(SimulationEvent::Rejected {
                        command_index: Some(command_index),
                        reason: SimulationRejectReason::InvalidAmount,
                    });
                    continue;
                }

                let Some(stats) = state.combatants.get_mut(&target_id) else {
                    events.push(SimulationEvent::Rejected {
                        command_index: Some(command_index),
                        reason: SimulationRejectReason::UnknownCombatant { target_id },
                    });
                    continue;
                };

                let outcome = stats.apply_damage(amount);
                events.push(SimulationEvent::DamageApplied {
                    target_id,
                    applied: outcome.applied,
                    remaining_health: outcome.remaining_health,
                    defeated: outcome.defeated,
                });
            }
            SimulationCommand::PlaceEntity {
                actor_id,
                kind,
                tile_x,
                tile_y,
            } => {
                if state
                    .placeables
                    .values()
                    .any(|placeable| placeable.tile_x == tile_x && placeable.tile_y == tile_y)
                {
                    events.push(SimulationEvent::Rejected {
                        command_index: Some(command_index),
                        reason: SimulationRejectReason::OccupiedTile { tile_x, tile_y },
                    });
                    continue;
                }

                let build_cost = placeable_build_cost(kind);
                let inventory = state
                    .inventories
                    .entry(actor_id.clone())
                    .or_insert_with(InventoryState::default);

                if let Err(error) = inventory.consume_requirements(&build_cost) {
                    events.push(SimulationEvent::Rejected {
                        command_index: Some(command_index),
                        reason: inventory_error_to_reject_reason(error, actor_id.clone()),
                    });
                    continue;
                }

                let placeable_id = state.next_placeable_id;
                state.next_placeable_id = state.next_placeable_id.saturating_add(1);

                let max_health = placeable_max_health(kind);
                state.placeables.insert(
                    placeable_id,
                    PlaceableEntity {
                        id: placeable_id,
                        owner_id: actor_id.clone(),
                        kind,
                        tile_x,
                        tile_y,
                        max_health,
                        health: max_health,
                    },
                );

                events.push(SimulationEvent::PlaceablePlaced {
                    placeable_id,
                    owner_id: actor_id,
                    kind,
                    tile_x,
                    tile_y,
                });
            }
        }
    }

    resolve_crafting(state, &mut events);

    SimulationStepOutput {
        schema_version: GAMEPLAY_SCHEMA_VERSION,
        tick: state.tick,
        events,
    }
}

fn resolve_crafting(state: &mut GameplayState, events: &mut Vec<SimulationEvent>) {
    let actor_ids: Vec<String> = state.craft_queues.keys().cloned().collect();

    for actor_id in actor_ids {
        start_pending_craft(state, &actor_id, events);

        let completed_recipe = {
            let Some(queue) = state.craft_queues.get_mut(&actor_id) else {
                continue;
            };

            let Some(active) = queue.active.as_mut() else {
                continue;
            };

            if active.remaining_ticks > 0 {
                active.remaining_ticks -= 1;
            }

            if active.remaining_ticks == 0 {
                let recipe = active.recipe;
                queue.active = None;
                Some(recipe)
            } else {
                None
            }
        };

        if let Some(recipe) = completed_recipe {
            let definition = recipe_definition(recipe);
            let inventory = state
                .inventories
                .entry(actor_id.clone())
                .or_insert_with(InventoryState::default);

            let mut output_error = None;
            for output in definition.outputs {
                if let Err(error) = inventory.add_resource(output.resource, output.amount) {
                    output_error = Some(error);
                    break;
                }
            }

            if let Some(error) = output_error {
                events.push(SimulationEvent::Rejected {
                    command_index: None,
                    reason: inventory_error_to_reject_reason(error, actor_id.clone()),
                });
                continue;
            }

            events.push(SimulationEvent::CraftCompleted { actor_id, recipe });
        }
    }
}

fn start_pending_craft(
    state: &mut GameplayState,
    actor_id: &str,
    events: &mut Vec<SimulationEvent>,
) {
    let next_recipe = {
        let Some(queue) = state.craft_queues.get(actor_id) else {
            return;
        };

        if queue.active.is_some() {
            return;
        }

        queue.peek_recipe()
    };

    let Some(recipe) = next_recipe else {
        return;
    };

    let definition = recipe_definition(recipe);

    let can_start = {
        let inventory = state
            .inventories
            .entry(actor_id.to_string())
            .or_insert_with(InventoryState::default);
        inventory.can_afford(&definition.inputs)
    };

    if !can_start {
        return;
    }

    let inventory = state
        .inventories
        .entry(actor_id.to_string())
        .or_insert_with(InventoryState::default);
    if inventory.consume_requirements(&definition.inputs).is_err() {
        return;
    }

    let queue = match state.craft_queues.get_mut(actor_id) {
        Some(queue) => queue,
        None => return,
    };

    if queue.active.is_some() {
        return;
    }

    let Some(consumed_recipe) = queue.consume_one_pending() else {
        return;
    };

    let remaining_ticks = definition.craft_ticks.max(1);
    queue.active = Some(ActiveCraft {
        recipe: consumed_recipe,
        remaining_ticks,
    });

    events.push(SimulationEvent::CraftStarted {
        actor_id: actor_id.to_string(),
        recipe: consumed_recipe,
        remaining_ticks,
    });
}

fn inventory_error_to_reject_reason(
    error: InventoryOpError,
    actor_id: String,
) -> SimulationRejectReason {
    match error {
        InventoryOpError::InvalidAmount => SimulationRejectReason::InvalidAmount,
        InventoryOpError::NoFreeSlot => SimulationRejectReason::InventoryNoFreeSlot { actor_id },
        InventoryOpError::InsufficientResource {
            resource,
            required,
            available,
        } => SimulationRejectReason::InsufficientResource {
            actor_id,
            resource,
            required,
            available,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn craft_and_place_input(tick: u64) -> SimulationStepInput {
        SimulationStepInput {
            schema_version: GAMEPLAY_SCHEMA_VERSION,
            tick,
            commands: vec![
                SimulationCommand::GrantResource {
                    actor_id: "player-a".to_string(),
                    resource: ResourceKind::IronOre,
                    amount: 4,
                },
                SimulationCommand::GrantResource {
                    actor_id: "player-a".to_string(),
                    resource: ResourceKind::Stone,
                    amount: 8,
                },
                SimulationCommand::QueueCraft {
                    actor_id: "player-a".to_string(),
                    recipe: RecipeKind::SmeltIronPlate,
                    count: 2,
                },
                SimulationCommand::QueueCraft {
                    actor_id: "player-a".to_string(),
                    recipe: RecipeKind::CraftGear,
                    count: 1,
                },
            ],
        }
    }

    #[test]
    fn deterministic_step_outputs_match() {
        let mut left = GameplayState::default();
        let mut right = GameplayState::default();

        let mut left_output = simulate_step(&mut left, craft_and_place_input(1));
        let mut right_output = simulate_step(&mut right, craft_and_place_input(1));

        assert_eq!(left_output, right_output);

        // Tick until queued crafting resolves.
        for tick in 2..=8 {
            left_output = simulate_step(
                &mut left,
                SimulationStepInput {
                    tick,
                    ..SimulationStepInput::default()
                },
            );
            right_output = simulate_step(
                &mut right,
                SimulationStepInput {
                    tick,
                    ..SimulationStepInput::default()
                },
            );
            assert_eq!(left_output, right_output);
        }

        assert_eq!(left, right);
    }

    #[test]
    fn crafting_and_placement_flow_consumes_inventory() {
        let mut state = GameplayState::default();

        simulate_step(
            &mut state,
            SimulationStepInput {
                schema_version: GAMEPLAY_SCHEMA_VERSION,
                tick: 1,
                commands: vec![
                    SimulationCommand::GrantResource {
                        actor_id: "builder".to_string(),
                        resource: ResourceKind::IronPlate,
                        amount: 3,
                    },
                    SimulationCommand::GrantResource {
                        actor_id: "builder".to_string(),
                        resource: ResourceKind::Gear,
                        amount: 2,
                    },
                    SimulationCommand::PlaceEntity {
                        actor_id: "builder".to_string(),
                        kind: PlaceableKind::BurnerDrill,
                        tile_x: 10,
                        tile_y: -4,
                    },
                ],
            },
        );

        let inventory = state.inventories.get("builder").expect("inventory exists");
        assert_eq!(inventory.count(ResourceKind::IronPlate), 0);
        assert_eq!(inventory.count(ResourceKind::Gear), 0);
        assert_eq!(state.placeables.len(), 1);
    }

    #[test]
    fn schema_version_is_explicit_and_rejected_when_mismatched() {
        let mut state = GameplayState::default();

        let mismatch = simulate_step(
            &mut state,
            SimulationStepInput {
                schema_version: GAMEPLAY_SCHEMA_VERSION + 1,
                tick: 1,
                commands: vec![],
            },
        );

        assert!(matches!(
            mismatch.events.first(),
            Some(SimulationEvent::Rejected {
                reason: SimulationRejectReason::SchemaVersionMismatch { .. },
                ..
            })
        ));

        let serialized = serde_json::to_value(SimulationStepInput::default()).unwrap();
        assert_eq!(serialized["schemaVersion"], GAMEPLAY_SCHEMA_VERSION);
    }

    #[test]
    fn damage_application_is_server_authoritative() {
        let mut state = GameplayState::default();

        simulate_step(
            &mut state,
            SimulationStepInput {
                schema_version: GAMEPLAY_SCHEMA_VERSION,
                tick: 1,
                commands: vec![SimulationCommand::RegisterCombatant {
                    actor_id: "enemy-1".to_string(),
                    stats: CombatStats::new(120, 14, 3),
                }],
            },
        );

        let output = simulate_step(
            &mut state,
            SimulationStepInput {
                schema_version: GAMEPLAY_SCHEMA_VERSION,
                tick: 2,
                commands: vec![SimulationCommand::DealDamage {
                    target_id: "enemy-1".to_string(),
                    amount: 20,
                }],
            },
        );

        assert!(matches!(
            output.events.last(),
            Some(SimulationEvent::DamageApplied {
                target_id,
                applied,
                remaining_health,
                defeated,
            }) if target_id == "enemy-1" && *applied == 17 && *remaining_health == 103 && !*defeated
        ));
    }
}

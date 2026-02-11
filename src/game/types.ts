export const PROTOCOL_VERSION = 2 as const;

export const PROTOCOL_FEATURES = [
  'core',
  'movement',
  'build',
  'projectile',
  'inventory',
  'mining',
  'drops',
  'crafting',
  'combat',
  'character',
] as const;

export type ProtocolFeature = (typeof PROTOCOL_FEATURES)[number];

export const SERVER_ENVELOPE_KINDS = [
  'welcome',
  'ack',
  'snapshot',
  'event',
  'error',
  'pong',
] as const;

export type ServerEnvelopeKind = (typeof SERVER_ENVELOPE_KINDS)[number];

export type PlayerState = {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  connected: boolean;
};

export type BuildStructure = {
  id: string;
  x: number;
  y: number;
  kind: string;
  ownerId: string;
};

export type BuildPreview = {
  playerId: string;
  x: number;
  y: number;
  kind: string;
};

export type PresenceSnapshot = {
  online: string[];
  onlineCount: number;
};

export type InputAckMap = Record<string, number>;

export type MovementSnapshot = {
  players: PlayerState[];
  inputAcks: InputAckMap;
  speed: number;
};

export type BuildSnapshot = {
  structures: BuildStructure[];
  structureCount: number;
  previews: BuildPreview[];
  previewCount: number;
};

export type ProjectileState = {
  id: string;
  ownerId: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  clientProjectileId: string | null;
};

export type ProjectileSnapshot = {
  projectiles: ProjectileState[];
  projectileCount: number;
};

export type TerrainSnapshot = {
  seed: string;
  generatorVersion: number;
  tileSize: number;
};

export type InventoryStack = {
  slot: number;
  resource: string;
  amount: number;
};

export type InventoryPlayerState = {
  playerId: string;
  maxSlots: number;
  stacks: InventoryStack[];
};

export type InventorySnapshot = {
  schemaVersion: number;
  revision: number;
  players: InventoryPlayerState[];
  playerCount: number;
};

export type MiningNodeState = {
  id: string;
  kind: string;
  x: number;
  y: number;
  remaining: number;
};

export type MiningProgressState = {
  playerId: string;
  nodeId: string;
  startedAt: number;
  completesAt: number;
  progress: number;
};

export type MiningSnapshot = {
  schemaVersion: number;
  nodes: MiningNodeState[];
  nodeCount: number;
  active: MiningProgressState[];
  activeCount: number;
};

export type DropState = {
  id: string;
  resource: string;
  amount: number;
  x: number;
  y: number;
  spawnedAt: number;
  expiresAt: number;
  ownerPlayerId: string | null;
  ownerExpiresAt: number;
};

export type DropSnapshot = {
  schemaVersion: number;
  drops: DropState[];
  dropCount: number;
};

export type CraftQueueEntry = {
  recipe: string;
  count: number;
};

export type ActiveCraftState = {
  recipe: string;
  remainingTicks: number;
};

export type CraftQueueState = {
  playerId: string;
  pending: CraftQueueEntry[];
  active: ActiveCraftState | null;
};

export type CraftingSnapshot = {
  schemaVersion: number;
  queues: CraftQueueState[];
  queueCount: number;
};

export type EnemyState = {
  id: string;
  kind: string;
  x: number;
  y: number;
  health: number;
  maxHealth: number;
  targetPlayerId?: string;
};

export type PlayerCombatState = {
  playerId: string;
  health: number;
  maxHealth: number;
  attackPower: number;
  armor: number;
};

export type CombatSnapshot = {
  schemaVersion: number;
  enemies: EnemyState[];
  enemyCount: number;
  players: PlayerCombatState[];
  playerCount: number;
};

export type CharacterProfileState = {
  playerId: string;
  name: string;
  spriteId: string;
};

export type CharacterSnapshot = {
  schemaVersion: number;
  players: CharacterProfileState[];
  playerCount: number;
};

export type RoomSnapshot = {
  roomCode: string;
  serverTick: number;
  simRateHz: number;
  snapshotRateHz: number;
  serverTime: number;
  mode: 'full' | 'delta';
  features: {
    presence?: PresenceSnapshot;
    movement?: MovementSnapshot;
    build?: BuildSnapshot;
    projectile?: ProjectileSnapshot;
    terrain?: TerrainSnapshot;
    inventory?: InventorySnapshot;
    mining?: MiningSnapshot;
    drops?: DropSnapshot;
    crafting?: CraftingSnapshot;
    combat?: CombatSnapshot;
    character?: CharacterSnapshot;
  };
};

export type WelcomePayload = {
  roomCode: string;
  playerId: string;
  simRateHz: number;
  snapshotRateHz: number;
  resumeToken?: string;
};

export type ServerEnvelope = {
  v: typeof PROTOCOL_VERSION;
  kind: ServerEnvelopeKind;
  tick: number;
  serverTime: number;
  feature: ProtocolFeature;
  action: string;
  seq?: number;
  payload?: unknown;
};

export type ClientCommandEnvelope = {
  v: typeof PROTOCOL_VERSION;
  kind: 'command';
  seq: number;
  feature: ProtocolFeature;
  action: string;
  clientTime: number;
  payload?: unknown;
};

export type InputState = {
  up: boolean;
  down: boolean;
  left: boolean;
  right: boolean;
};

export type InputCommand = InputState & {
  seq: number;
};

export type OutboundFeatureCommand = {
  feature: ProtocolFeature;
  action: string;
  payload?: unknown;
};

export type RenderSnapshotPayload = {
  serverTick: number;
  simRateHz: number;
  localAckSeq: number;
  renderDelayMs: number;
  players: PlayerState[];
  structures: BuildStructure[];
  previews: BuildPreview[];
  projectiles: ProjectileState[];
  terrain: TerrainSnapshot | null;
  mining: MiningSnapshot | null;
  drops: DropSnapshot | null;
  character: CharacterSnapshot | null;
};

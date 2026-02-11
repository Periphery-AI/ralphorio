import type {
  ClientCommandEnvelope,
  InputCommand,
  ProtocolFeature,
  RoomSnapshot,
  ServerEnvelope,
  WelcomePayload,
} from './types';
import { PROTOCOL_FEATURES, PROTOCOL_VERSION, SERVER_ENVELOPE_KINDS } from './types';

type Handlers = {
  onWelcome: (payload: WelcomePayload) => void;
  onSnapshot: (snapshot: RoomSnapshot) => void;
  onAck: (seq: number, feature: ProtocolFeature, action: string) => void;
  onStatus: (status: string) => void;
  onEvent: (feature: ProtocolFeature, action: string, payload: unknown) => void;
  onPong?: (latencyMs: number) => void;
};

type SnapshotFeatures = RoomSnapshot['features'];
type PresenceSnapshot = NonNullable<SnapshotFeatures['presence']>;
type MovementSnapshot = NonNullable<SnapshotFeatures['movement']>;
type BuildSnapshot = NonNullable<SnapshotFeatures['build']>;
type ProjectileSnapshot = NonNullable<SnapshotFeatures['projectile']>;
type TerrainSnapshot = NonNullable<SnapshotFeatures['terrain']>;
type InventorySnapshot = NonNullable<SnapshotFeatures['inventory']>;
type MiningSnapshot = NonNullable<SnapshotFeatures['mining']>;
type DropSnapshot = NonNullable<SnapshotFeatures['drops']>;
type CraftingSnapshot = NonNullable<SnapshotFeatures['crafting']>;
type CombatSnapshot = NonNullable<SnapshotFeatures['combat']>;
type CharacterSnapshot = NonNullable<SnapshotFeatures['character']>;

const MAX_SNAPSHOT_LIST_ITEMS = 4096;
const protocolFeatureSet = new Set<string>(PROTOCOL_FEATURES);
const envelopeKindSet = new Set<string>(SERVER_ENVELOPE_KINDS);

function buildWebSocketUrl(roomCode: string, playerId: string) {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const host = window.location.host;
  const encodedRoom = encodeURIComponent(roomCode);
  const encodedPlayer = encodeURIComponent(playerId);
  return `${protocol}//${host}/api/rooms/${encodedRoom}/ws?playerId=${encodedPlayer}`;
}

function appendResumeToken(url: string, resumeToken: string | null) {
  if (!resumeToken) {
    return url;
  }

  const separator = url.includes('?') ? '&' : '?';
  return `${url}${separator}resumeToken=${encodeURIComponent(resumeToken)}`;
}

function appendAuthToken(url: string, token: string | null) {
  if (!token) {
    return url;
  }

  const separator = url.includes('?') ? '&' : '?';
  return `${url}${separator}token=${encodeURIComponent(token)}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isNonNegativeInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0;
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value > 0;
}

function isProtocolFeature(value: unknown): value is ProtocolFeature {
  return typeof value === 'string' && protocolFeatureSet.has(value);
}

function isEnvelopeKind(value: unknown): value is ServerEnvelope['kind'] {
  return typeof value === 'string' && envelopeKindSet.has(value);
}

function parseArray<T>(
  value: unknown,
  parseItem: (entry: unknown) => T | null,
  maxItems = MAX_SNAPSHOT_LIST_ITEMS,
): T[] | null {
  if (!Array.isArray(value) || value.length > maxItems) {
    return null;
  }

  const parsedItems: T[] = [];
  for (const entry of value) {
    const parsed = parseItem(entry);
    if (!parsed) {
      return null;
    }
    parsedItems.push(parsed);
  }

  return parsedItems;
}

function parseCount(value: unknown, minimum: number) {
  if (!isNonNegativeInteger(value) || value < minimum) {
    return null;
  }

  return value;
}

function parsePlayerState(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.id !== 'string' ||
    !isFiniteNumber(payload.x) ||
    !isFiniteNumber(payload.y) ||
    !isFiniteNumber(payload.vx) ||
    !isFiniteNumber(payload.vy) ||
    typeof payload.connected !== 'boolean'
  ) {
    return null;
  }

  return {
    id: payload.id,
    x: payload.x,
    y: payload.y,
    vx: payload.vx,
    vy: payload.vy,
    connected: payload.connected,
  };
}

function parseBuildStructure(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.id !== 'string' ||
    !isFiniteNumber(payload.x) ||
    !isFiniteNumber(payload.y) ||
    typeof payload.kind !== 'string' ||
    typeof payload.ownerId !== 'string'
  ) {
    return null;
  }

  return {
    id: payload.id,
    x: payload.x,
    y: payload.y,
    kind: payload.kind,
    ownerId: payload.ownerId,
  };
}

function parseBuildPreview(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.playerId !== 'string' ||
    !isFiniteNumber(payload.x) ||
    !isFiniteNumber(payload.y) ||
    typeof payload.kind !== 'string'
  ) {
    return null;
  }

  if (payload.canPlace !== undefined && typeof payload.canPlace !== 'boolean') {
    return null;
  }

  if (payload.reason !== undefined && payload.reason !== null && typeof payload.reason !== 'string') {
    return null;
  }

  return {
    playerId: payload.playerId,
    x: payload.x,
    y: payload.y,
    kind: payload.kind,
    canPlace: payload.canPlace ?? true,
    reason: typeof payload.reason === 'string' ? payload.reason : null,
  };
}

function parseErrorMessage(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  return typeof payload.message === 'string' ? payload.message : null;
}

function parseProjectileState(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.id !== 'string' ||
    typeof payload.ownerId !== 'string' ||
    !isFiniteNumber(payload.x) ||
    !isFiniteNumber(payload.y) ||
    !isFiniteNumber(payload.vx) ||
    !isFiniteNumber(payload.vy)
  ) {
    return null;
  }

  if (
    payload.clientProjectileId !== undefined &&
    payload.clientProjectileId !== null &&
    typeof payload.clientProjectileId !== 'string'
  ) {
    return null;
  }

  return {
    id: payload.id,
    ownerId: payload.ownerId,
    x: payload.x,
    y: payload.y,
    vx: payload.vx,
    vy: payload.vy,
    clientProjectileId:
      payload.clientProjectileId === undefined ? null : payload.clientProjectileId,
  };
}

function parseInputAcks(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  const inputAcks: Record<string, number> = {};
  for (const [playerId, seq] of Object.entries(payload)) {
    if (playerId.length === 0 || !isNonNegativeInteger(seq)) {
      return null;
    }
    inputAcks[playerId] = seq;
  }

  return inputAcks;
}

function parsePresenceSnapshot(payload: unknown): PresenceSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const online = parseArray(payload.online, (entry) =>
    typeof entry === 'string' ? entry : null,
  );
  if (!online || !isNonNegativeInteger(payload.onlineCount) || payload.onlineCount < online.length) {
    return null;
  }

  return {
    online,
    onlineCount: payload.onlineCount,
  };
}

function parseMovementSnapshot(payload: unknown): MovementSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const players = parseArray(payload.players, parsePlayerState);
  const inputAcks = parseInputAcks(payload.inputAcks);
  if (!players || !inputAcks || !isFiniteNumber(payload.speed)) {
    return null;
  }

  return {
    players,
    inputAcks,
    speed: payload.speed,
  };
}

function parseBuildSnapshot(payload: unknown): BuildSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const structures = parseArray(payload.structures, parseBuildStructure);
  const previews = parseArray(payload.previews, parseBuildPreview);
  const structureCount = structures ? parseCount(payload.structureCount, structures.length) : null;
  const previewCount = previews ? parseCount(payload.previewCount, previews.length) : null;
  if (!structures || !previews || structureCount === null || previewCount === null) {
    return null;
  }

  return {
    structures,
    structureCount,
    previews,
    previewCount,
  };
}

function parseProjectileSnapshot(payload: unknown): ProjectileSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const projectiles = parseArray(payload.projectiles, parseProjectileState);
  const projectileCount = projectiles ? parseCount(payload.projectileCount, projectiles.length) : null;
  if (!projectiles || projectileCount === null) {
    return null;
  }

  return {
    projectiles,
    projectileCount,
  };
}

function parseTerrainSnapshot(payload: unknown): TerrainSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.seed !== 'string' ||
    !isPositiveInteger(payload.generatorVersion) ||
    !isPositiveInteger(payload.tileSize)
  ) {
    return null;
  }

  return {
    seed: payload.seed,
    generatorVersion: payload.generatorVersion,
    tileSize: payload.tileSize,
  };
}

function parseInventoryStack(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    !isNonNegativeInteger(payload.slot) ||
    typeof payload.resource !== 'string' ||
    !isNonNegativeInteger(payload.amount)
  ) {
    return null;
  }

  return {
    slot: payload.slot,
    resource: payload.resource,
    amount: payload.amount,
  };
}

function parseInventoryPlayer(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  const stacks = parseArray(payload.stacks, parseInventoryStack);
  if (typeof payload.playerId !== 'string' || !isPositiveInteger(payload.maxSlots) || !stacks) {
    return null;
  }

  return {
    playerId: payload.playerId,
    maxSlots: payload.maxSlots,
    stacks,
  };
}

function parseInventorySnapshot(payload: unknown): InventorySnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const players = parseArray(payload.players, parseInventoryPlayer);
  const playerCount = players ? parseCount(payload.playerCount, players.length) : null;
  if (
    !isPositiveInteger(payload.schemaVersion) ||
    !isNonNegativeInteger(payload.revision) ||
    !players ||
    playerCount === null
  ) {
    return null;
  }

  return {
    schemaVersion: payload.schemaVersion,
    revision: payload.revision,
    players,
    playerCount,
  };
}

function parseMiningNode(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.id !== 'string' ||
    typeof payload.kind !== 'string' ||
    !isFiniteNumber(payload.x) ||
    !isFiniteNumber(payload.y) ||
    !isNonNegativeInteger(payload.remaining)
  ) {
    return null;
  }

  return {
    id: payload.id,
    kind: payload.kind,
    x: payload.x,
    y: payload.y,
    remaining: payload.remaining,
  };
}

function parseMiningProgress(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.playerId !== 'string' ||
    typeof payload.nodeId !== 'string' ||
    !isFiniteNumber(payload.startedAt) ||
    !isFiniteNumber(payload.completesAt) ||
    !isFiniteNumber(payload.progress) ||
    payload.progress < 0 ||
    payload.progress > 1
  ) {
    return null;
  }

  return {
    playerId: payload.playerId,
    nodeId: payload.nodeId,
    startedAt: payload.startedAt,
    completesAt: payload.completesAt,
    progress: payload.progress,
  };
}

function parseMiningSnapshot(payload: unknown): MiningSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const nodes = parseArray(payload.nodes, parseMiningNode);
  const active = parseArray(payload.active, parseMiningProgress);
  const nodeCount = nodes ? parseCount(payload.nodeCount, nodes.length) : null;
  const activeCount = active ? parseCount(payload.activeCount, active.length) : null;
  if (
    !isPositiveInteger(payload.schemaVersion) ||
    !nodes ||
    !active ||
    nodeCount === null ||
    activeCount === null
  ) {
    return null;
  }

  return {
    schemaVersion: payload.schemaVersion,
    nodes,
    nodeCount,
    active,
    activeCount,
  };
}

function parseDropState(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.id !== 'string' ||
    typeof payload.resource !== 'string' ||
    !isNonNegativeInteger(payload.amount) ||
    !isFiniteNumber(payload.x) ||
    !isFiniteNumber(payload.y) ||
    !isFiniteNumber(payload.spawnedAt) ||
    !isFiniteNumber(payload.expiresAt) ||
    !isFiniteNumber(payload.ownerExpiresAt)
  ) {
    return null;
  }

  if (
    payload.ownerPlayerId !== null &&
    payload.ownerPlayerId !== undefined &&
    typeof payload.ownerPlayerId !== 'string'
  ) {
    return null;
  }

  return {
    id: payload.id,
    resource: payload.resource,
    amount: payload.amount,
    x: payload.x,
    y: payload.y,
    spawnedAt: payload.spawnedAt,
    expiresAt: payload.expiresAt,
    ownerPlayerId:
      payload.ownerPlayerId === undefined ? null : (payload.ownerPlayerId as string | null),
    ownerExpiresAt: payload.ownerExpiresAt,
  };
}

function parseDropSnapshot(payload: unknown): DropSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const drops = parseArray(payload.drops, parseDropState);
  const dropCount = drops ? parseCount(payload.dropCount, drops.length) : null;
  if (!isPositiveInteger(payload.schemaVersion) || !drops || dropCount === null) {
    return null;
  }

  return {
    schemaVersion: payload.schemaVersion,
    drops,
    dropCount,
  };
}

function parseCraftQueueEntry(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (typeof payload.recipe !== 'string' || !isPositiveInteger(payload.count)) {
    return null;
  }

  return {
    recipe: payload.recipe,
    count: payload.count,
  };
}

function parseActiveCraft(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (typeof payload.recipe !== 'string' || !isPositiveInteger(payload.remainingTicks)) {
    return null;
  }

  return {
    recipe: payload.recipe,
    remainingTicks: payload.remainingTicks,
  };
}

function parseCraftQueue(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  const pending = parseArray(payload.pending, parseCraftQueueEntry);
  if (typeof payload.playerId !== 'string' || !pending) {
    return null;
  }

  let active = null;
  if (payload.active !== null && payload.active !== undefined) {
    active = parseActiveCraft(payload.active);
    if (!active) {
      return null;
    }
  }

  return {
    playerId: payload.playerId,
    pending,
    active,
  };
}

function parseCraftingSnapshot(payload: unknown): CraftingSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const queues = parseArray(payload.queues, parseCraftQueue);
  const queueCount = queues ? parseCount(payload.queueCount, queues.length) : null;
  if (!isPositiveInteger(payload.schemaVersion) || !queues || queueCount === null) {
    return null;
  }

  return {
    schemaVersion: payload.schemaVersion,
    queues,
    queueCount,
  };
}

function parseEnemy(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.id !== 'string' ||
    typeof payload.kind !== 'string' ||
    !isFiniteNumber(payload.x) ||
    !isFiniteNumber(payload.y) ||
    !isNonNegativeInteger(payload.health) ||
    !isPositiveInteger(payload.maxHealth)
  ) {
    return null;
  }

  if (
    payload.targetPlayerId !== undefined &&
    payload.targetPlayerId !== null &&
    typeof payload.targetPlayerId !== 'string'
  ) {
    return null;
  }

  return {
    id: payload.id,
    kind: payload.kind,
    x: payload.x,
    y: payload.y,
    health: payload.health,
    maxHealth: payload.maxHealth,
    targetPlayerId: typeof payload.targetPlayerId === 'string' ? payload.targetPlayerId : undefined,
  };
}

function parsePlayerCombat(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.playerId !== 'string' ||
    !isNonNegativeInteger(payload.health) ||
    !isPositiveInteger(payload.maxHealth) ||
    !isNonNegativeInteger(payload.attackPower) ||
    !isNonNegativeInteger(payload.armor)
  ) {
    return null;
  }

  return {
    playerId: payload.playerId,
    health: payload.health,
    maxHealth: payload.maxHealth,
    attackPower: payload.attackPower,
    armor: payload.armor,
  };
}

function parseCombatSnapshot(payload: unknown): CombatSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const enemies = parseArray(payload.enemies, parseEnemy);
  const players = parseArray(payload.players, parsePlayerCombat);
  const enemyCount = enemies ? parseCount(payload.enemyCount, enemies.length) : null;
  const playerCount = players ? parseCount(payload.playerCount, players.length) : null;
  if (
    !isPositiveInteger(payload.schemaVersion) ||
    !enemies ||
    !players ||
    enemyCount === null ||
    playerCount === null
  ) {
    return null;
  }

  return {
    schemaVersion: payload.schemaVersion,
    enemies,
    enemyCount,
    players,
    playerCount,
  };
}

function parseCharacterProfile(payload: unknown) {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.playerId !== 'string' ||
    typeof payload.name !== 'string' ||
    typeof payload.spriteId !== 'string'
  ) {
    return null;
  }

  return {
    playerId: payload.playerId,
    name: payload.name,
    spriteId: payload.spriteId,
  };
}

function parseCharacterSnapshot(payload: unknown): CharacterSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  const players = parseArray(payload.players, parseCharacterProfile);
  const playerCount = players ? parseCount(payload.playerCount, players.length) : null;
  if (!isPositiveInteger(payload.schemaVersion) || !players || playerCount === null) {
    return null;
  }

  return {
    schemaVersion: payload.schemaVersion,
    players,
    playerCount,
  };
}

function parseServerEnvelope(raw: string): ServerEnvelope | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }

  if (!isRecord(parsed)) {
    return null;
  }

  if (parsed.v !== PROTOCOL_VERSION) {
    return null;
  }

  if (
    !isEnvelopeKind(parsed.kind) ||
    !isProtocolFeature(parsed.feature) ||
    typeof parsed.action !== 'string' ||
    parsed.action.length === 0 ||
    parsed.action.length > 64 ||
    !isNonNegativeInteger(parsed.tick) ||
    !isFiniteNumber(parsed.serverTime)
  ) {
    return null;
  }

  if (parsed.seq !== undefined && !isPositiveInteger(parsed.seq)) {
    return null;
  }

  return {
    v: PROTOCOL_VERSION,
    kind: parsed.kind,
    tick: parsed.tick,
    serverTime: parsed.serverTime,
    feature: parsed.feature,
    action: parsed.action,
    seq: parsed.seq,
    payload: parsed.payload,
  };
}

function parseWelcomePayload(payload: unknown): WelcomePayload | null {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.roomCode !== 'string' ||
    typeof payload.playerId !== 'string' ||
    typeof payload.simRateHz !== 'number' ||
    typeof payload.snapshotRateHz !== 'number'
  ) {
    return null;
  }

  return {
    roomCode: payload.roomCode,
    playerId: payload.playerId,
    simRateHz: payload.simRateHz,
    snapshotRateHz: payload.snapshotRateHz,
    resumeToken: typeof payload.resumeToken === 'string' ? payload.resumeToken : undefined,
  };
}

function parseRoomSnapshot(payload: unknown): RoomSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.roomCode !== 'string' ||
    typeof payload.serverTick !== 'number' ||
    !isNonNegativeInteger(payload.serverTick) ||
    !isPositiveInteger(payload.simRateHz) ||
    !isPositiveInteger(payload.snapshotRateHz) ||
    !isFiniteNumber(payload.serverTime) ||
    (payload.mode !== 'full' && payload.mode !== 'delta') ||
    !isRecord(payload.features)
  ) {
    return null;
  }

  const features: RoomSnapshot['features'] = {};
  const rawFeatures = payload.features;

  if ('presence' in rawFeatures) {
    const presence = parsePresenceSnapshot(rawFeatures.presence);
    if (!presence) {
      return null;
    }
    features.presence = presence;
  }

  if ('movement' in rawFeatures) {
    const movement = parseMovementSnapshot(rawFeatures.movement);
    if (!movement) {
      return null;
    }
    features.movement = movement;
  }

  if ('build' in rawFeatures) {
    const build = parseBuildSnapshot(rawFeatures.build);
    if (!build) {
      return null;
    }
    features.build = build;
  }

  if ('projectile' in rawFeatures) {
    const projectile = parseProjectileSnapshot(rawFeatures.projectile);
    if (!projectile) {
      return null;
    }
    features.projectile = projectile;
  }

  if ('terrain' in rawFeatures) {
    const terrain = parseTerrainSnapshot(rawFeatures.terrain);
    if (!terrain) {
      return null;
    }
    features.terrain = terrain;
  }

  if ('inventory' in rawFeatures) {
    const inventory = parseInventorySnapshot(rawFeatures.inventory);
    if (!inventory) {
      return null;
    }
    features.inventory = inventory;
  }

  if ('mining' in rawFeatures) {
    const mining = parseMiningSnapshot(rawFeatures.mining);
    if (!mining) {
      return null;
    }
    features.mining = mining;
  }

  if ('drops' in rawFeatures) {
    const drops = parseDropSnapshot(rawFeatures.drops);
    if (!drops) {
      return null;
    }
    features.drops = drops;
  }

  if ('crafting' in rawFeatures) {
    const crafting = parseCraftingSnapshot(rawFeatures.crafting);
    if (!crafting) {
      return null;
    }
    features.crafting = crafting;
  }

  if ('combat' in rawFeatures) {
    const combat = parseCombatSnapshot(rawFeatures.combat);
    if (!combat) {
      return null;
    }
    features.combat = combat;
  }

  if ('character' in rawFeatures) {
    const character = parseCharacterSnapshot(rawFeatures.character);
    if (!character) {
      return null;
    }
    features.character = character;
  }

  return {
    roomCode: payload.roomCode,
    serverTick: payload.serverTick,
    simRateHz: payload.simRateHz,
    snapshotRateHz: payload.snapshotRateHz,
    serverTime: payload.serverTime,
    mode: payload.mode,
    features,
  };
}

export class RoomSocket {
  private socket: WebSocket | null = null;
  private readonly roomCode: string;
  private readonly playerId: string;
  private readonly handlers: Handlers;
  private readonly authToken: string | null;
  private readonly resumeToken: string | null;
  private seq = 1;
  private pingTimer: number | null = null;
  private pingSentAt = new Map<number, number>();

  constructor(
    roomCode: string,
    playerId: string,
    handlers: Handlers,
    authToken: string | null = null,
    resumeToken: string | null = null,
  ) {
    this.roomCode = roomCode;
    this.playerId = playerId;
    this.handlers = handlers;
    this.authToken = authToken;
    this.resumeToken = resumeToken;
  }

  async connect() {
    const baseUrl = buildWebSocketUrl(this.roomCode, this.playerId);
    const withResume = appendResumeToken(baseUrl, this.resumeToken);
    const url = appendAuthToken(withResume, this.authToken);
    this.handlers.onStatus('Connecting...');

    this.socket = new WebSocket(url);

    this.socket.addEventListener('open', () => {
      this.handlers.onStatus('Connected');
      this.startPingLoop();
    });

    this.socket.addEventListener('message', (event) => {
      if (typeof event.data !== 'string') {
        return;
      }

      const envelope = parseServerEnvelope(event.data);
      if (!envelope) {
        return;
      }

      if (envelope.kind === 'welcome') {
        const payload = parseWelcomePayload(envelope.payload);
        if (!payload) {
          return;
        }
        this.handlers.onWelcome(payload);
        return;
      }

      if (envelope.kind === 'snapshot') {
        const snapshot = parseRoomSnapshot(envelope.payload);
        if (!snapshot) {
          return;
        }

        this.handlers.onSnapshot(snapshot);
        return;
      }

      if (envelope.kind === 'ack') {
        if (typeof envelope.seq === 'number') {
          this.handlers.onAck(envelope.seq, envelope.feature, envelope.action);
        }
        return;
      }

      if (envelope.kind === 'event') {
        this.handlers.onEvent(envelope.feature, envelope.action, envelope.payload);
        return;
      }

      if (envelope.kind === 'pong') {
        if (typeof envelope.seq === 'number') {
          const sentAt = this.pingSentAt.get(envelope.seq);
          if (sentAt !== undefined && this.handlers.onPong) {
            this.handlers.onPong(performance.now() - sentAt);
          }
          this.pingSentAt.delete(envelope.seq);
        }
        return;
      }

      if (envelope.kind === 'error') {
        const message = parseErrorMessage(envelope.payload);
        this.handlers.onStatus(
          message
            ? `Error: ${envelope.feature}.${envelope.action} - ${message}`
            : `Error: ${envelope.feature}.${envelope.action}`,
        );
      }
    });

    this.socket.addEventListener('close', () => {
      this.stopPingLoop();
      this.handlers.onStatus('Disconnected');
    });

    this.socket.addEventListener('error', () => {
      this.stopPingLoop();
      this.handlers.onStatus('Connection error');
    });
  }

  sendBuildPlace(x: number, y: number, kind = 'beacon') {
    this.sendFeatureCommand('build', 'place', {
      x,
      y,
      kind,
      clientBuildId: `build_${crypto.randomUUID()}`,
    });
  }

  sendFeatureCommand(feature: ProtocolFeature, action: string, payload?: unknown) {
    return this.sendCommand({
      feature,
      action,
      payload,
    });
  }

  sendBuildRemove(id: string) {
    this.sendFeatureCommand('build', 'remove', { id });
  }

  sendCorePing() {
    return this.sendFeatureCommand('core', 'ping', null);
  }

  sendMovementInputBatch(inputs: InputCommand[]) {
    if (inputs.length === 0) {
      return null;
    }

    return this.sendFeatureCommand('movement', 'input_batch', { inputs });
  }

  sendInputCommands(inputs: InputCommand[]) {
    this.sendMovementInputBatch(inputs);
  }

  private sendPing() {
    const seq = this.sendCorePing();

    if (seq !== null) {
      this.pingSentAt.set(seq, performance.now());
    }
  }

  private startPingLoop() {
    this.stopPingLoop();
    this.sendPing();
    this.pingTimer = window.setInterval(() => {
      this.sendPing();
    }, 2000);
  }

  private stopPingLoop() {
    if (this.pingTimer !== null) {
      window.clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
    this.pingSentAt.clear();
  }

  private sendCommand(params: {
    feature: ProtocolFeature;
    action: string;
    payload?: unknown;
  }): number | null {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) {
      return null;
    }

    const currentSeq = this.seq;
    const envelope: ClientCommandEnvelope = {
      v: PROTOCOL_VERSION,
      kind: 'command',
      seq: currentSeq,
      feature: params.feature,
      action: params.action,
      payload: params.payload,
      clientTime: performance.now(),
    };

    this.seq += 1;
    this.socket.send(JSON.stringify(envelope));
    return currentSeq;
  }

  disconnect() {
    this.stopPingLoop();
    if (this.socket) {
      this.socket.close();
      this.socket = null;
    }
  }
}

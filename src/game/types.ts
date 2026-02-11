export const PROTOCOL_VERSION = 2 as const;

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

export type RoomSnapshot = {
  roomCode: string;
  serverTick: number;
  simRateHz: number;
  snapshotRateHz: number;
  serverTime: number;
  features: {
    presence?: PresenceSnapshot;
    movement?: MovementSnapshot;
    build?: BuildSnapshot;
    projectile?: ProjectileSnapshot;
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
  kind: 'welcome' | 'ack' | 'snapshot' | 'event' | 'error' | 'pong';
  tick: number;
  serverTime: number;
  feature: string;
  action: string;
  seq?: number;
  payload?: unknown;
};

export type ClientCommandEnvelope = {
  v: typeof PROTOCOL_VERSION;
  kind: 'command';
  seq: number;
  feature: string;
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
  feature: string;
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
};

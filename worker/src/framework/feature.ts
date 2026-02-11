export type FeatureMigration = {
  id: string;
  statements: string[];
};

export type FeatureEvent = {
  target: 'room' | 'self' | 'player';
  playerId?: string;
  feature: string;
  action: string;
  payload: unknown;
};

export type FeatureCommandResult = {
  stateChanged?: boolean;
  events?: FeatureEvent[];
};

export type FeatureTickResult = {
  stateChanged?: boolean;
  events?: FeatureEvent[];
};

export type FeatureContext = {
  sql: SqlStorage;
  roomCode: string;
  now: number;
  tick: number;
  tickDeltaSeconds: number;
  connectedPlayerIds: string[];
};

export interface RoomFeature {
  key: string;
  migrations: FeatureMigration[];
  onConnect(ctx: FeatureContext, playerId: string): FeatureCommandResult | void;
  onDisconnect(ctx: FeatureContext, playerId: string): FeatureCommandResult | void;
  onCommand(
    ctx: FeatureContext,
    playerId: string,
    action: string,
    payload: unknown,
    seq: number,
  ): FeatureCommandResult | void;
  onTick(ctx: FeatureContext): FeatureTickResult | void;
  createSnapshot(ctx: FeatureContext): unknown;
}

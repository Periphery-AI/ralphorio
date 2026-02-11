import type {
  FeatureCommandResult,
  FeatureContext,
  FeatureMigration,
  FeatureTickResult,
  RoomFeature,
} from '../framework/feature';
import { movementStep } from '../sim-core/runtime';

type InputState = {
  up: boolean;
  down: boolean;
  left: boolean;
  right: boolean;
};

type InputCommand = InputState & {
  seq: number;
};

type InputBatchPayload = {
  inputs: InputCommand[];
};

type PlayerState = {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  connected: boolean;
};

type RuntimeState = {
  input: InputState;
  lastInputSeq: number;
};

const MOVE_SPEED = 220;
const MAP_LIMIT = 5000;
const ZERO_INPUT: InputState = {
  up: false,
  down: false,
  left: false,
  right: false,
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function parseInputCommand(value: unknown): InputCommand | null {
  if (!isRecord(value)) {
    return null;
  }

  const seq = value.seq;
  const up = value.up;
  const down = value.down;
  const left = value.left;
  const right = value.right;

  if (!Number.isInteger(seq) || Number(seq) < 1) {
    return null;
  }

  if (typeof up !== 'boolean' || typeof down !== 'boolean' || typeof left !== 'boolean' || typeof right !== 'boolean') {
    return null;
  }

  return {
    seq: Number(seq),
    up,
    down,
    left,
    right,
  };
}

function parseInputBatchPayload(payload: unknown): InputBatchPayload | null {
  if (!isRecord(payload) || !Array.isArray(payload.inputs)) {
    return null;
  }

  if (payload.inputs.length > 128) {
    return null;
  }

  const inputs: InputCommand[] = [];
  for (const item of payload.inputs) {
    const parsed = parseInputCommand(item);
    if (!parsed) {
      return null;
    }
    inputs.push(parsed);
  }

  return {
    inputs,
  };
}

export class MovementFeature implements RoomFeature {
  key = 'movement';

  migrations: FeatureMigration[] = [
    {
      id: 'movement_v1',
      statements: [
        `
          CREATE TABLE IF NOT EXISTS movement_state (
            player_id TEXT PRIMARY KEY,
            x REAL NOT NULL DEFAULT 0,
            y REAL NOT NULL DEFAULT 0,
            vx REAL NOT NULL DEFAULT 0,
            vy REAL NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
          )
        `,
      ],
    },
  ];

  private readonly runtimeByPlayer = new Map<string, RuntimeState>();

  onConnect(ctx: FeatureContext, playerId: string): FeatureCommandResult {
    if (!this.runtimeByPlayer.has(playerId)) {
      this.runtimeByPlayer.set(playerId, {
        input: { ...ZERO_INPUT },
        lastInputSeq: 0,
      });
    }

    ctx.sql.exec(
      `
      INSERT INTO movement_state (player_id, x, y, vx, vy, updated_at)
      VALUES (?1, 0, 0, 0, 0, ?2)
      ON CONFLICT(player_id) DO UPDATE
      SET
        updated_at = excluded.updated_at
      `,
      playerId,
      ctx.now,
    );

    return { stateChanged: true };
  }

  onDisconnect(_ctx: FeatureContext, playerId: string): FeatureCommandResult {
    this.runtimeByPlayer.delete(playerId);
    return { stateChanged: true };
  }

  onCommand(
    _ctx: FeatureContext,
    playerId: string,
    action: string,
    payload: unknown,
  ): FeatureCommandResult {
    if (action !== 'input_batch') {
      return {
        events: [
          {
            target: 'self',
            feature: this.key,
            action: 'invalid_action',
            payload: { action },
          },
        ],
      };
    }

    const parsed = parseInputBatchPayload(payload);
    if (!parsed) {
      return {
        events: [
          {
            target: 'self',
            feature: this.key,
            action: 'invalid_payload',
            payload: {},
          },
        ],
      };
    }

    if (parsed.inputs.length === 0) {
      return { stateChanged: false };
    }

    const runtime = this.runtimeByPlayer.get(playerId) ?? {
      input: { ...ZERO_INPUT },
      lastInputSeq: 0,
    };

    for (const input of parsed.inputs) {
      if (input.seq <= runtime.lastInputSeq) {
        continue;
      }

      runtime.lastInputSeq = input.seq;
      runtime.input = {
        up: input.up,
        down: input.down,
        left: input.left,
        right: input.right,
      };
    }

    this.runtimeByPlayer.set(playerId, runtime);
    return { stateChanged: false };
  }

  onTick(ctx: FeatureContext): FeatureTickResult {
    const dt = ctx.tickDeltaSeconds;

    for (const playerId of ctx.connectedPlayerIds) {
      const runtime = this.runtimeByPlayer.get(playerId) ?? {
        input: { ...ZERO_INPUT },
        lastInputSeq: 0,
      };

      this.runtimeByPlayer.set(playerId, runtime);

      const rows = ctx.sql.exec(
        'SELECT x, y FROM movement_state WHERE player_id = ?1 LIMIT 1',
        playerId,
      );

      let currentX = 0;
      let currentY = 0;
      for (const row of rows) {
        currentX = Number(row.x);
        currentY = Number(row.y);
      }

      const simStep = movementStep({
        x: currentX,
        y: currentY,
        input: runtime.input,
        dtSeconds: dt,
        speed: MOVE_SPEED,
        mapLimit: MAP_LIMIT,
      });

      ctx.sql.exec(
        `
        INSERT INTO movement_state (player_id, x, y, vx, vy, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(player_id) DO UPDATE
        SET
          x = excluded.x,
          y = excluded.y,
          vx = excluded.vx,
          vy = excluded.vy,
          updated_at = excluded.updated_at
        `,
        playerId,
        simStep.x,
        simStep.y,
        simStep.vx,
        simStep.vy,
        ctx.now,
      );
    }

    return { stateChanged: true };
  }

  createSnapshot(ctx: FeatureContext) {
    const connected = new Set(ctx.connectedPlayerIds);
    const rows = ctx.sql.exec('SELECT player_id, x, y, vx, vy FROM movement_state ORDER BY player_id ASC');
    const players: PlayerState[] = [];

    for (const row of rows) {
      const id = String(row.player_id);
      if (!connected.has(id)) {
        continue;
      }

      players.push({
        id,
        x: Number(row.x),
        y: Number(row.y),
        vx: Number(row.vx),
        vy: Number(row.vy),
        connected: true,
      });
    }

    const inputAcks: Record<string, number> = {};
    for (const [playerId, runtime] of this.runtimeByPlayer.entries()) {
      inputAcks[playerId] = runtime.lastInputSeq;
    }

    return {
      players,
      inputAcks,
      speed: MOVE_SPEED,
    };
  }
}

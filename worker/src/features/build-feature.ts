import type { FeatureCommandResult, FeatureContext, FeatureMigration, RoomFeature } from '../framework/feature';

type PlacePayload = {
  x: number;
  y: number;
  kind: string;
  clientBuildId?: string;
};

type BuildStructure = {
  id: string;
  x: number;
  y: number;
  kind: string;
  ownerId: string;
};

const ALLOWED_KINDS = new Set(['beacon', 'miner', 'assembler']);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function parsePlacePayload(payload: unknown): PlacePayload | null {
  if (!isRecord(payload)) {
    return null;
  }

  const x = payload.x;
  const y = payload.y;
  const kind = payload.kind;
  const clientBuildId = payload.clientBuildId;

  if (typeof x !== 'number' || typeof y !== 'number' || !Number.isFinite(x) || !Number.isFinite(y)) {
    return null;
  }

  if (typeof kind !== 'string' || !ALLOWED_KINDS.has(kind)) {
    return null;
  }

  if (clientBuildId !== undefined && typeof clientBuildId !== 'string') {
    return null;
  }

  return {
    x,
    y,
    kind,
    clientBuildId,
  };
}

function clamp(value: number) {
  return Math.max(-5000, Math.min(5000, value));
}

export class BuildFeature implements RoomFeature {
  key = 'build';

  migrations: FeatureMigration[] = [
    {
      id: 'build_v1',
      statements: [
        `
          CREATE TABLE IF NOT EXISTS build_structures (
            structure_id TEXT PRIMARY KEY,
            owner_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            x REAL NOT NULL,
            y REAL NOT NULL,
            created_at INTEGER NOT NULL
          )
        `,
      ],
    },
  ];

  onConnect(): FeatureCommandResult | void {}

  onDisconnect(): FeatureCommandResult | void {}

  onTick(): FeatureCommandResult | void {}

  onCommand(ctx: FeatureContext, playerId: string, action: string, payload: unknown): FeatureCommandResult {
    if (action === 'place') {
      const parsed = parsePlacePayload(payload);
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

      const structureId = parsed.clientBuildId && parsed.clientBuildId.length > 0
        ? parsed.clientBuildId
        : crypto.randomUUID();

      const x = clamp(parsed.x);
      const y = clamp(parsed.y);

      ctx.sql.exec(
        `
        INSERT INTO build_structures (structure_id, owner_id, kind, x, y, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(structure_id) DO NOTHING
        `,
        structureId,
        playerId,
        parsed.kind,
        x,
        y,
        ctx.now,
      );

      return {
        stateChanged: true,
        events: [
          {
            target: 'room',
            feature: this.key,
            action: 'placed',
            payload: {
              id: structureId,
              ownerId: playerId,
              kind: parsed.kind,
              x,
              y,
            },
          },
        ],
      };
    }

    if (action === 'remove') {
      if (!isRecord(payload) || typeof payload.id !== 'string') {
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

      ctx.sql.exec('DELETE FROM build_structures WHERE structure_id = ?1', payload.id);
      return { stateChanged: true };
    }

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

  createSnapshot(ctx: FeatureContext) {
    const rows = ctx.sql.exec(
      'SELECT structure_id, owner_id, kind, x, y FROM build_structures ORDER BY created_at DESC LIMIT 1024',
    );

    const structures: BuildStructure[] = [];
    for (const row of rows) {
      structures.push({
        id: String(row.structure_id),
        ownerId: String(row.owner_id),
        kind: String(row.kind),
        x: Number(row.x),
        y: Number(row.y),
      });
    }

    return {
      structures,
      structureCount: structures.length,
    };
  }
}

import type {
  FeatureCommandResult,
  FeatureContext,
  FeatureMigration,
  FeatureTickResult,
  RoomFeature,
} from '../framework/feature';
import { clampPosition, projectileStep } from '../sim-core/runtime';

type FirePayload = {
  x: number;
  y: number;
  vx: number;
  vy: number;
  clientProjectileId?: string;
};

type ProjectileState = {
  id: string;
  ownerId: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  clientProjectileId: string | null;
};

const MAX_PROJECTILES = 4096;
const PROJECTILE_TTL_MS = 1800;
const MAX_SPEED = 700;
const MAP_LIMIT = 5500;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function parseFirePayload(payload: unknown): FirePayload | null {
  if (!isRecord(payload)) {
    return null;
  }

  const x = payload.x;
  const y = payload.y;
  const vx = payload.vx;
  const vy = payload.vy;
  const clientProjectileId = payload.clientProjectileId;

  if (
    typeof x !== 'number' ||
    typeof y !== 'number' ||
    typeof vx !== 'number' ||
    typeof vy !== 'number' ||
    !Number.isFinite(x) ||
    !Number.isFinite(y) ||
    !Number.isFinite(vx) ||
    !Number.isFinite(vy)
  ) {
    return null;
  }

  if (clientProjectileId !== undefined && typeof clientProjectileId !== 'string') {
    return null;
  }

  return { x, y, vx, vy, clientProjectileId };
}

export class ProjectileFeature implements RoomFeature {
  key = 'projectile';

  migrations: FeatureMigration[] = [
    {
      id: 'projectile_v1',
      statements: [
        `
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
        `,
      ],
    },
    {
      id: 'projectile_v2_client_id',
      statements: [
        'ALTER TABLE projectile_state ADD COLUMN client_projectile_id TEXT',
      ],
    },
  ];

  onConnect(): FeatureCommandResult | void {}

  onDisconnect(): FeatureCommandResult | void {}

  onCommand(ctx: FeatureContext, playerId: string, action: string, payload: unknown): FeatureCommandResult {
    if (action !== 'fire') {
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

    const parsed = parseFirePayload(payload);
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

    const projectileId = crypto.randomUUID();

    const speed = Math.hypot(parsed.vx, parsed.vy);
    const scale = speed > MAX_SPEED && speed > 0 ? MAX_SPEED / speed : 1;

    const vx = parsed.vx * scale;
    const vy = parsed.vy * scale;

    ctx.sql.exec(
      `
      INSERT INTO projectile_state (projectile_id, owner_id, x, y, vx, vy, expires_at, updated_at, client_projectile_id)
      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
      `,
      projectileId,
      playerId,
      clampPosition(parsed.x, MAP_LIMIT),
      clampPosition(parsed.y, MAP_LIMIT),
      vx,
      vy,
      ctx.now + PROJECTILE_TTL_MS,
      ctx.now,
      parsed.clientProjectileId ?? null,
    );

    ctx.sql.exec(
      `
      DELETE FROM projectile_state
      WHERE projectile_id IN (
        SELECT projectile_id FROM projectile_state
        ORDER BY updated_at ASC
        LIMIT (
          SELECT MAX(0, COUNT(*) - ?1) FROM projectile_state
        )
      )
      `,
      MAX_PROJECTILES,
    );

    return { stateChanged: true };
  }

  onTick(ctx: FeatureContext): FeatureTickResult {
    const dt = ctx.tickDeltaSeconds;
    const rows = ctx.sql.exec('SELECT projectile_id, x, y, vx, vy, expires_at FROM projectile_state');

    let updatedCount = 0;

    for (const row of rows) {
      const projectileId = String(row.projectile_id);
      const expiresAt = Number(row.expires_at);

      if (expiresAt <= ctx.now) {
        ctx.sql.exec('DELETE FROM projectile_state WHERE projectile_id = ?1', projectileId);
        updatedCount += 1;
        continue;
      }

      const x = Number(row.x);
      const y = Number(row.y);
      const vx = Number(row.vx);
      const vy = Number(row.vy);

      const step = projectileStep({
        x,
        y,
        vx,
        vy,
        dtSeconds: dt,
        mapLimit: MAP_LIMIT,
      });

      ctx.sql.exec(
        'UPDATE projectile_state SET x = ?1, y = ?2, updated_at = ?3 WHERE projectile_id = ?4',
        step.x,
        step.y,
        ctx.now,
        projectileId,
      );
      updatedCount += 1;
    }

    return { stateChanged: updatedCount > 0 };
  }

  createSnapshot(ctx: FeatureContext) {
    const rows = ctx.sql.exec(
      `
      SELECT projectile_id, owner_id, x, y, vx, vy, client_projectile_id
      FROM projectile_state
      WHERE expires_at > ?1
      ORDER BY updated_at DESC
      LIMIT 2048
      `,
      ctx.now,
    );

    const projectiles: ProjectileState[] = [];
    for (const row of rows) {
      projectiles.push({
        id: String(row.projectile_id),
        ownerId: String(row.owner_id),
        x: Number(row.x),
        y: Number(row.y),
        vx: Number(row.vx),
        vy: Number(row.vy),
        clientProjectileId: row.client_projectile_id === null ? null : String(row.client_projectile_id),
      });
    }

    return {
      projectiles,
      projectileCount: projectiles.length,
    };
  }
}

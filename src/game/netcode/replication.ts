import type {
  BuildStructure,
  PlayerState,
  ProjectileState,
  RenderSnapshotPayload,
  RoomSnapshot,
} from '../types';

const DEFAULT_INTERPOLATION_DELAY_MS = 110;
const MAX_BUFFERED_SNAPSHOTS = 90;

function lerp(a: number, b: number, t: number) {
  return a + (b - a) * t;
}

function interpolatePlayers(
  older: PlayerState[],
  newer: PlayerState[],
  alpha: number,
  localPlayerId: string,
  latestPlayers: PlayerState[],
) {
  const olderById = new Map(older.map((player) => [player.id, player]));
  const newerById = new Map(newer.map((player) => [player.id, player]));
  const latestById = new Map(latestPlayers.map((player) => [player.id, player]));

  const allIds = new Set<string>();
  for (const id of olderById.keys()) {
    allIds.add(id);
  }
  for (const id of newerById.keys()) {
    allIds.add(id);
  }

  const output: PlayerState[] = [];
  for (const id of allIds.values()) {
    if (id === localPlayerId) {
      const local = latestById.get(id) ?? newerById.get(id) ?? olderById.get(id);
      if (local) {
        output.push(local);
      }
      continue;
    }

    const from = olderById.get(id);
    const to = newerById.get(id) ?? from;
    if (!from && !to) {
      continue;
    }

    if (!from || !to) {
      output.push((from ?? to) as PlayerState);
      continue;
    }

    output.push({
      id,
      x: lerp(from.x, to.x, alpha),
      y: lerp(from.y, to.y, alpha),
      vx: lerp(from.vx, to.vx, alpha),
      vy: lerp(from.vy, to.vy, alpha),
      connected: to.connected,
    });
  }

  return output;
}

function interpolateProjectiles(older: ProjectileState[], newer: ProjectileState[], alpha: number) {
  const olderById = new Map(older.map((projectile) => [projectile.id, projectile]));
  const newerById = new Map(newer.map((projectile) => [projectile.id, projectile]));

  const allIds = new Set<string>();
  for (const id of olderById.keys()) {
    allIds.add(id);
  }
  for (const id of newerById.keys()) {
    allIds.add(id);
  }

  const output: ProjectileState[] = [];
  for (const id of allIds.values()) {
    const from = olderById.get(id);
    const to = newerById.get(id) ?? from;
    if (!from && !to) {
      continue;
    }

    if (!from || !to) {
      output.push((from ?? to) as ProjectileState);
      continue;
    }

    output.push({
      id,
      ownerId: to.ownerId,
      clientProjectileId: to.clientProjectileId,
      x: lerp(from.x, to.x, alpha),
      y: lerp(from.y, to.y, alpha),
      vx: lerp(from.vx, to.vx, alpha),
      vy: lerp(from.vy, to.vy, alpha),
    });
  }

  return output;
}

function copyStructures(structures: BuildStructure[]) {
  return structures.slice();
}

function snapshotTime(snapshot: RoomSnapshot) {
  return snapshot.serverTime;
}

export class ReplicationPipeline {
  private readonly interpolationDelayMs: number;
  private readonly snapshots: RoomSnapshot[] = [];
  private clockOffsetMs = 0;
  private hasClockSync = false;

  constructor(interpolationDelayMs = DEFAULT_INTERPOLATION_DELAY_MS) {
    this.interpolationDelayMs = interpolationDelayMs;
  }

  ingestSnapshot(snapshot: RoomSnapshot) {
    const offsetSample = snapshot.serverTime - Date.now();
    if (!this.hasClockSync) {
      this.clockOffsetMs = offsetSample;
      this.hasClockSync = true;
    } else {
      this.clockOffsetMs = this.clockOffsetMs * 0.9 + offsetSample * 0.1;
    }

    this.snapshots.push(snapshot);
    this.snapshots.sort((a, b) => a.serverTick - b.serverTick);

    if (this.snapshots.length > MAX_BUFFERED_SNAPSHOTS) {
      const overflow = this.snapshots.length - MAX_BUFFERED_SNAPSHOTS;
      this.snapshots.splice(0, overflow);
    }
  }

  buildRenderSnapshot(localPlayerId: string): RenderSnapshotPayload | null {
    const latest = this.snapshots[this.snapshots.length - 1];
    if (!latest) {
      return null;
    }

    const latestMovement = latest.features.movement;
    if (!latestMovement) {
      return null;
    }

    const renderTargetTime = Date.now() + this.clockOffsetMs - this.interpolationDelayMs;

    let older = latest;
    let newer = latest;

    for (let index = 0; index < this.snapshots.length; index += 1) {
      const candidate = this.snapshots[index];
      if (snapshotTime(candidate) <= renderTargetTime) {
        older = candidate;
      }
      if (snapshotTime(candidate) >= renderTargetTime) {
        newer = candidate;
        break;
      }
    }

    const olderMovement = older.features.movement ?? latestMovement;
    const newerMovement = newer.features.movement ?? latestMovement;
    const olderProjectiles = older.features.projectile?.projectiles ?? [];
    const newerProjectiles = newer.features.projectile?.projectiles ?? [];

    const span = Math.max(1, snapshotTime(newer) - snapshotTime(older));
    const alphaRaw = (renderTargetTime - snapshotTime(older)) / span;
    const alpha = Math.max(0, Math.min(1, alphaRaw));

    const players = interpolatePlayers(
      olderMovement.players,
      newerMovement.players,
      alpha,
      localPlayerId,
      latestMovement.players,
    );

    const projectiles = interpolateProjectiles(olderProjectiles, newerProjectiles, alpha);
    const structures = copyStructures(latest.features.build?.structures ?? []);

    return {
      serverTick: latest.serverTick,
      simRateHz: latest.simRateHz,
      localAckSeq: latestMovement.inputAcks[localPlayerId] ?? 0,
      players,
      structures,
      projectiles,
    };
  }
}

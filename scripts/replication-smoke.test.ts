import assert from 'node:assert/strict';
import test from 'node:test';

import { ReplicationPipeline } from '../src/game/netcode/replication.ts';

type SnapshotOptions = {
  tick: number;
  serverTime: number;
  playerX?: number;
  combat?:
    | {
        enemies: Array<{
          id: string;
          kind: string;
          x: number;
          y: number;
          health: number;
          maxHealth: number;
          targetPlayerId?: string;
        }>;
        enemyCount: number;
      }
    | null;
};

function withMockedNow(now: number, run: () => void) {
  const originalNow = Date.now;
  Date.now = () => now;
  try {
    run();
  } finally {
    Date.now = originalNow;
  }
}

function buildSnapshot(options: SnapshotOptions) {
  const movement = {
    players: [
      {
        id: 'player-local',
        x: options.playerX ?? 0,
        y: 0,
        vx: 0,
        vy: 0,
        connected: true,
      },
    ],
    inputAcks: {
      'player-local': options.tick,
    },
    speed: 220,
  };

  const features: Record<string, unknown> = { movement };
  if (options.combat !== undefined) {
    features.combat = {
      schemaVersion: 1,
      enemies: options.combat?.enemies ?? [],
      enemyCount: options.combat?.enemyCount ?? 0,
      players: [
        {
          playerId: 'player-local',
          health: 100,
          maxHealth: 100,
          attackPower: 10,
          armor: 2,
        },
      ],
      playerCount: 1,
    };
  }

  return {
    roomCode: 'TEST',
    serverTick: options.tick,
    simRateHz: 20,
    snapshotRateHz: 12,
    serverTime: options.serverTime,
    mode: options.tick === 1 ? 'full' : 'delta',
    features,
  };
}

test('replication forwards combat enemies into render payload', () => {
  const pipeline = new ReplicationPipeline(0);
  withMockedNow(1_000, () => {
    pipeline.ingestSnapshot(
      buildSnapshot({
        tick: 1,
        serverTime: 1_000,
        combat: {
          enemies: [
            {
              id: 'enemy:4:-2',
              kind: 'biter_small',
              x: 64,
              y: -32,
              health: 16,
              maxHealth: 20,
              targetPlayerId: 'player-local',
            },
          ],
          enemyCount: 1,
        },
      }),
    );
  });

  let renderSnapshot = null;
  withMockedNow(1_000, () => {
    renderSnapshot = pipeline.buildRenderSnapshot('player-local');
  });
  assert.ok(renderSnapshot);
  assert.equal(renderSnapshot.serverTick, 1);
  assert.ok(renderSnapshot.combat);
  assert.equal(renderSnapshot.combat.enemyCount, 1);
  assert.equal(renderSnapshot.combat.enemies[0]?.id, 'enemy:4:-2');
});

test('delta snapshots keep previous combat payload when combat feature is omitted', () => {
  const pipeline = new ReplicationPipeline(0);
  withMockedNow(2_000, () => {
    pipeline.ingestSnapshot(
      buildSnapshot({
        tick: 1,
        serverTime: 2_000,
        combat: {
          enemies: [
            {
              id: 'enemy:1:1',
              kind: 'spitter_small',
              x: 32,
              y: 32,
              health: 30,
              maxHealth: 30,
            },
          ],
          enemyCount: 1,
        },
      }),
    );
  });
  withMockedNow(2_016, () => {
    pipeline.ingestSnapshot(
      buildSnapshot({
        tick: 2,
        serverTime: 2_016,
        playerX: 8,
      }),
    );
  });

  let renderSnapshot = null;
  withMockedNow(2_016, () => {
    renderSnapshot = pipeline.buildRenderSnapshot('player-local');
  });
  assert.ok(renderSnapshot);
  assert.equal(renderSnapshot.serverTick, 2);
  assert.ok(renderSnapshot.combat);
  assert.equal(renderSnapshot.combat.enemyCount, 1);
  assert.equal(renderSnapshot.combat.enemies[0]?.id, 'enemy:1:1');
});

test('explicit combat snapshots can clear stale enemies', () => {
  const pipeline = new ReplicationPipeline(0);
  withMockedNow(3_000, () => {
    pipeline.ingestSnapshot(
      buildSnapshot({
        tick: 1,
        serverTime: 3_000,
        combat: {
          enemies: [
            {
              id: 'enemy:clear-me',
              kind: 'biter_medium',
              x: 40,
              y: 40,
              health: 45,
              maxHealth: 45,
            },
          ],
          enemyCount: 1,
        },
      }),
    );
  });
  withMockedNow(3_016, () => {
    pipeline.ingestSnapshot(
      buildSnapshot({
        tick: 2,
        serverTime: 3_016,
        combat: {
          enemies: [],
          enemyCount: 0,
        },
      }),
    );
  });

  let renderSnapshot = null;
  withMockedNow(3_016, () => {
    renderSnapshot = pipeline.buildRenderSnapshot('player-local');
  });
  assert.ok(renderSnapshot);
  assert.ok(renderSnapshot.combat);
  assert.equal(renderSnapshot.combat.enemyCount, 0);
  assert.deepEqual(renderSnapshot.combat.enemies, []);
});

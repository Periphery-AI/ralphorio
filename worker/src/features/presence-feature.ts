import type { FeatureCommandResult, FeatureContext, FeatureMigration, RoomFeature } from '../framework/feature';

function connectedPlayers(ctx: FeatureContext) {
  const rows = ctx.sql.exec(
    'SELECT player_id FROM presence_players WHERE connected = 1 ORDER BY last_seen DESC',
  );

  const online: string[] = [];
  for (const row of rows) {
    online.push(String(row.player_id));
  }

  return online;
}

export class PresenceFeature implements RoomFeature {
  key = 'presence';

  migrations: FeatureMigration[] = [
    {
      id: 'presence_v1',
      statements: [
        `
          CREATE TABLE IF NOT EXISTS presence_players (
            player_id TEXT PRIMARY KEY,
            connected INTEGER NOT NULL DEFAULT 0,
            last_seen INTEGER NOT NULL
          )
        `,
      ],
    },
  ];

  onConnect(ctx: FeatureContext, playerId: string): FeatureCommandResult {
    ctx.sql.exec(
      `
      INSERT INTO presence_players (player_id, connected, last_seen)
      VALUES (?1, 1, ?2)
      ON CONFLICT(player_id) DO UPDATE
      SET
        connected = 1,
        last_seen = excluded.last_seen
      `,
      playerId,
      ctx.now,
    );

    return { stateChanged: true };
  }

  onDisconnect(ctx: FeatureContext, playerId: string): FeatureCommandResult {
    ctx.sql.exec(
      'UPDATE presence_players SET connected = 0, last_seen = ?1 WHERE player_id = ?2',
      ctx.now,
      playerId,
    );

    return { stateChanged: true };
  }

  onCommand(): FeatureCommandResult | void {}

  onTick(): FeatureCommandResult | void {}

  createSnapshot(ctx: FeatureContext) {
    const online = connectedPlayers(ctx);
    return {
      online,
      onlineCount: online.length,
    };
  }
}

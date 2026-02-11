import type { RoomFeature } from './feature';

const MIGRATION_TABLE_SQL = `
  CREATE TABLE IF NOT EXISTS _feature_migrations (
    feature_key TEXT NOT NULL,
    migration_id TEXT NOT NULL,
    applied_at INTEGER NOT NULL,
    PRIMARY KEY (feature_key, migration_id)
  )
`;

export function applyFeatureMigrations(sql: SqlStorage, features: RoomFeature[]) {
  sql.exec(MIGRATION_TABLE_SQL);

  for (const feature of features) {
    for (const migration of feature.migrations) {
      const rows = sql.exec(
        'SELECT 1 FROM _feature_migrations WHERE feature_key = ?1 AND migration_id = ?2 LIMIT 1',
        feature.key,
        migration.id,
      );

      let alreadyApplied = false;
      for (const _row of rows) {
        alreadyApplied = true;
      }
      if (alreadyApplied) {
        continue;
      }

      for (const statement of migration.statements) {
        try {
          sql.exec(statement);
        } catch (error) {
          const message = String(error);
          // Cloudflare SQLite does not support IF NOT EXISTS for ADD COLUMN.
          // Allow migrations to remain idempotent across partial/legacy states.
          if (message.includes('duplicate column name')) {
            continue;
          }
          throw error;
        }
      }

      sql.exec(
        'INSERT INTO _feature_migrations (feature_key, migration_id, applied_at) VALUES (?1, ?2, ?3)',
        feature.key,
        migration.id,
        Date.now(),
      );
    }
  }
}

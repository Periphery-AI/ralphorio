# Multiplayer Networking Architecture (Rust DO)

This project uses a server-authoritative Cloudflare Durable Object implemented in Rust, with client prediction and reconciliation in the Bevy WASM client.

## Goals

- Server-authoritative room state with SQLite durability and in-memory hot loop
- Fixed-step simulation (`30Hz`) + snapshot stream (`10Hz`)
- Client prediction + server reconciliation for local movement
- Interpolated remote rendering for smooth visuals
- Stable protocol and extensible feature surface for future systems

## Protocol v2

All websocket messages are envelope-based.

### Client -> Server

```json
{
  "v": 2,
  "kind": "command",
  "seq": 42,
  "feature": "movement",
  "action": "input_batch",
  "clientTime": 1234.56,
  "payload": {
    "inputs": [{ "seq": 101, "up": true, "down": false, "left": false, "right": true }]
  }
}
```

### Server -> Client

- `welcome`: room metadata + rates
- `welcome.resumeToken`: resumable session token for reconnect/restart recovery
- `ack`: command sequencing ack
- `snapshot`: authoritative room state (`mode = full|delta`)
- `pong`: ping response for latency
- `error`: protocol/auth/validation failures
- `event`: reserved for feature event channels

## Authority Runtime (Rust)

File: `worker/src/lib.rs`

- Entrypoint routes:
  - `/api/health`
  - `/api/rooms/:roomCode/ws` (DO websocket)
  - static assets via `ASSETS`
- One room code maps to one `RoomDurableObject`
- SQLite schema initialized inside the DO
- Runtime state is in-memory (`RoomRuntimeState`):
  - players/input
  - structures
  - build previews
  - projectiles
- Tick loop uses accumulator + bounded catch-up steps
- Hot simulation and snapshot assembly avoid per-tick SQL reads
- Player state checkpoints flush to SQLite every `~1000ms` and on connect/disconnect
- On DO startup/hydration, runtime state is rebuilt from SQLite checkpoints
- Movement/projectile integration call `sim-core`
- Snapshot payload supports full/delta feature channels:
  - `features.presence`
  - `features.movement`
  - `features.build`
  - `features.projectile`

### Durable vs Ephemeral Data

- **Durable (SQLite):**
  - room metadata
  - structures
  - player checkpoints (position, velocity, input, presence)
  - character profiles (`character_profiles`) keyed by `(user_id, character_id)`
  - active character selection (`active_character_profiles`) keyed by `user_id`
  - resumable session tokens
- **Ephemeral (in-memory):**
  - build previews
  - active projectiles
  - high-frequency simulation state

### Character Profile Migration/Backfill

- Schema initialization creates `character_profiles` and `active_character_profiles` with `CREATE TABLE IF NOT EXISTS`, so deploys are idempotent.
- On startup, legacy users discovered in `presence_players` are backfilled into `character_profiles` with the default character id (`default`), preserving migration safety for existing rooms.
- Missing active selections are backfilled to `default`, and snapshot assembly repairs partial/corrupt rows by recreating/selecting a valid default profile.

## Identity / Auth

- Client sends `playerId` and Clerk session token (`token`) in websocket query params.
- Client also sends optional `resumeToken` to recover a previous room session quickly.
- DO validates token claims and session status using `CLERK_SECRET_KEY`.
- If no secret is configured, DO falls back to permissive `playerId` mode for local/dev workflows.

## Client Netcode

### Transport

File: `src/game/network-client.ts`

- Protocol envelope encode/decode
- Sequenced commands and ping loop
- Movement input batch send (`movement.input_batch`)
- Resume token transport (`resumeToken` query param)

### Replication

File: `src/game/netcode/replication.ts`

- Buffers snapshots and tracks clock offset
- Uses interpolation delay (~110ms)
- Keeps local player authoritative correction via `localAckSeq`
- Merges delta snapshots with previous feature state to keep render continuity

### Room orchestration

File: `src/routes/room-route.tsx`

- Boots Bevy client
- Connects websocket with Clerk token
- Input pump + render pump (`~16ms`)

## Bevy WASM client

File: `game-client/src/lib.rs`

- Local fixed-step sim (`60Hz`)
- Predicts local movement using same `sim-core` math as server
- Replays unacked input history after authoritative correction
- Renders players, structures, and projectiles

## Extension strategy

1. Add command handling branch in `worker/src/lib.rs` for new feature/action.
2. Add SQLite tables/queries for authoritative state.
3. Extend snapshot feature payload shape.
4. Add transport command sender in `src/game/network-client.ts`.
5. Add interpolation/reconciliation policy in `src/game/netcode/replication.ts`.
6. Render/state handling in `game-client/src/lib.rs`.

This keeps a single authoritative runtime while preserving clean protocol boundaries for parallel feature development.

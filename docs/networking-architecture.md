# Multiplayer Networking Architecture (Rust DO)

This project uses a server-authoritative Cloudflare Durable Object implemented in Rust, with client prediction and reconciliation in the Bevy WASM client.

## Goals

- Server-authoritative room state with SQLite persistence
- Fixed-step simulation (`60Hz`) + snapshot stream (`20Hz`)
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
- `ack`: command sequencing ack
- `snapshot`: authoritative room state
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
- Tick loop uses accumulator + bounded catch-up steps
- Movement and projectile integration call `sim-core`
- Snapshot payload includes:
  - `features.presence`
  - `features.movement`
  - `features.build`
  - `features.projectile`

## Identity / Auth

- Client sends `playerId` and Clerk session token (`token`) in websocket query params.
- DO validates token claims and session status using `CLERK_SECRET_KEY`.
- If no secret is configured, DO falls back to permissive `playerId` mode for local/dev workflows.

## Client Netcode

### Transport

File: `src/game/network-client.ts`

- Protocol envelope encode/decode
- Sequenced commands and ping loop
- Movement input batch send (`movement.input_batch`)

### Replication

File: `src/game/netcode/replication.ts`

- Buffers snapshots and tracks clock offset
- Uses interpolation delay (~110ms)
- Keeps local player authoritative correction via `localAckSeq`

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

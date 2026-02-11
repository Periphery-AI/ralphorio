# Multiplayer Networking Architecture (Rewrite v2)

This project now uses a deterministic server-authoritative loop with explicit client prediction and reconciliation hooks.

## Goals

- Server-authoritative gameplay state in Durable Objects
- Fixed-step simulation (`60Hz`) with snapshot fanout (`20Hz`)
- Client prediction + server reconciliation for local movement
- Snapshot interpolation for remote entities/projectiles
- Feature-modular Durable Object runtime with per-feature SQLite migrations
- Shared Rust simulation core (`sim-core`) reused by DO and Bevy client

## Protocol v2

All WebSocket messages are envelope-based.

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
    "inputs": [
      { "seq": 101, "up": true, "down": false, "left": false, "right": true }
    ]
  }
}
```

### Server -> Client

- `welcome`: room metadata and simulation rates
- `ack`: envelope-level ack for command sequencing
- `snapshot`: authoritative world snapshot bundle
- `event`: feature event stream
- `pong`: latency/clock-sync support
- `error`: protocol or validation errors

## Durable Object Runtime

File: `worker/src/index.ts`

- Fixed simulation rate: `SIM_RATE_HZ = 60`
- Snapshot broadcast rate: `SNAPSHOT_RATE_HZ = 20`
- Main loop uses accumulated time and bounded catch-up steps
- On each simulation tick:
  1. call every feature `onTick` with fixed `dt`
  2. emit feature tick events
  3. broadcast snapshots on cadence (or dirty state)

### Shared simulation runtime (Rust in DO)

- Rust crate: `sim-core/`
- Built as raw WASM and loaded by worker runtime: `worker/src/sim-core/runtime.ts`
- Movement and projectile integration call Rust exports instead of duplicate TypeScript math:
  - `worker/src/features/movement-feature.ts`
  - `worker/src/features/projectile-feature.ts`
- Build command: `npm run sim:build:worker`

## Feature Modules

Each feature implements `RoomFeature` (`worker/src/framework/feature.ts`).

- `presence`: online roster + count
- `movement`: authoritative movement integration from input batches + input ack map
- `build`: authoritative structure placement/removal
- `projectile`: authoritative projectile creation/motion/expiry with client projectile id echo

Migrations are tracked in `_feature_migrations` and applied by `worker/src/framework/migrations.ts`.

## Client Netcode Pipeline

### Transport

File: `src/game/network-client.ts`

- Protocol v2 envelopes
- batched movement input commands (`movement.input_batch`)
- periodic ping/pong latency measurement

### Replication

File: `src/game/netcode/replication.ts`

- Maintains snapshot ring buffer
- Estimates server clock offset
- Renders remote state with interpolation delay (`~110ms`)
- Uses latest authoritative local ack for reconciliation

### Room orchestration

File: `src/routes/room-route.tsx`

- Input pump (`16ms`): drains WASM input commands and sends batches
- Render pump (`16ms`): builds interpolated render snapshot and pushes into WASM

## Bevy WASM Client

File: `game-client/src/lib.rs`

- Fixed-step local simulation (`60Hz`)
- Uses `sim-core` Rust crate directly for movement stepping parity
- Emits sequenced input commands for server processing
- Maintains local input history
- Reconciles local actor from authoritative position + replay of unacked inputs
- Smooths remote actor motion and renders authoritative structures/projectiles

## Extending the Game

1. Add a feature module in `worker/src/features/`.
2. Define feature migrations and snapshot shape.
3. Add client command sender in `src/game/network-client.ts`.
4. Add interpolation/reconciliation policy in `src/game/netcode/replication.ts`.
5. Render state in Bevy (`game-client/src/lib.rs`).

This layout keeps DO entrypoint stable while allowing parallel feature development.

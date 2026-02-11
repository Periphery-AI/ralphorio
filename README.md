# Ralph Island Multiplayer Foundation

A production-grade skeleton for a large co-op browser game:

- Frontend: Vite + React + Tailwind + TanStack Router
- Auth: Clerk
- Game client: Rust + Bevy (WASM)
- Shared simulation: Rust `sim-core` (WASM in DO + native crate in Bevy)
- Multiplayer backend: Cloudflare Worker + Durable Objects + SQLite
- Transport: WebSockets
- Domain target: `will.ralph-island.com`

## What is implemented now

- Room-based multiplayer (`/room/:roomCode`)
- Server-authoritative simulation loop in Durable Objects
- Rust `sim-core` executes authoritative movement/projectile integration in DO
- Deterministic fixed simulation tick (`60Hz`) + snapshot stream (`20Hz`)
- Client prediction + local reconciliation pipeline for movement
- Snapshot interpolation buffer for smoother remote rendering
- Authoritative structures and projectiles
- Protocol v2 envelopes with command acks + ping/pong latency tracking

## Runtime architecture

### Durable Object (authority)

- One room code => one `RoomDurableObject`
- Fixed-step simulation loop in `worker/src/index.ts`
- Feature modules:
  - `presence`
  - `movement`
  - `build`
  - `projectile`
- Per-feature SQLite migrations via `worker/src/framework/migrations.ts`
- Shared Rust simulation runtime loaded in Worker from `worker/src/sim-core/sim_core.wasm`

### Browser client

- `src/game/network-client.ts`: protocol transport and command sending
- `src/game/netcode/replication.ts`: clock sync + interpolation buffering
- `src/routes/room-route.tsx`: room session orchestration, pumps input/render data
- `src/game/bridge.ts`: JS <-> Bevy WASM bridge

### Bevy WASM

- `game-client/src/lib.rs`
- Uses `sim-core` crate directly for deterministic movement stepping parity
- Local fixed-step simulation and sequenced input emission
- Authoritative correction + replay of unacked input history
- Remote smoothing + rendering of structures/projectiles

## Development

### Prerequisites

- Node.js 20+
- Rust toolchain
- `wasm-pack`
- `wasm32-unknown-unknown` target
- Cloudflare account + `wrangler login`

Install Rust target and wasm-pack once:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

### Env setup

```bash
cp .env.example .env
```

Set at least:

- `VITE_CLERK_PUBLISHABLE_KEY`

### Run locally

Install deps:

```bash
npm install
```

Run worker (terminal 1):

```bash
npm run worker:dev
```

(`worker:dev` builds `sim-core` WASM first.)

Run frontend (terminal 2):

```bash
npm run dev
```

## Build

```bash
npm run build
```

This builds Bevy WASM first, then the web app.

## Deploy

`worker/wrangler.toml` is configured for:

- Durable Object binding `ROOMS`
- SQLite DO class migration
- Static assets from `dist/`
- custom domain route for `will.ralph-island.com`

Deploy:

```bash
npm run deploy
```

## Notes

- This rewrite is server-authoritative and netcode-focused so gameplay systems can be added safely.
- Architecture details: `docs/networking-architecture.md`.

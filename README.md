# Ralph Island Multiplayer Foundation

Production skeleton for a large co-op browser game:

- Frontend: Vite + React + Tailwind + TanStack Router
- Auth: Clerk
- Game client: Rust + Bevy (WASM)
- Shared simulation math: Rust `sim-core`
- Multiplayer backend: Rust Cloudflare Worker + Rust Durable Object + SQLite
- Transport: WebSockets
- Domain: `will.ralph-island.com`

## Current capabilities

- Room-based multiplayer (`/room/:roomCode`)
- Rust Durable Object room authority with SQLite persistence
- Server-authoritative movement, build objects, and projectiles
- Fixed-step simulation (`60Hz`) and snapshots (`20Hz`)
- Client prediction + server reconciliation for local player movement
- Snapshot interpolation for smooth remote rendering
- Protocol v2 envelopes with command ack and ping/pong
- Clerk-backed websocket identity (token + user id validation in Rust)

## Runtime architecture

### Rust Worker / Durable Object

- Entry point: `worker/src/lib.rs`
- One room code => one `RoomDurableObject`
- SQLite tables for presence, movement, builds, and projectiles
- Protocol command handling and snapshot broadcast
- Clerk session verification in the websocket connect path

### Browser app

- `src/routes/room-route.tsx`: session bootstrap and HUD
- `src/game/network-client.ts`: websocket protocol transport
- `src/game/netcode/replication.ts`: interpolation and render snapshots
- `src/game/bridge.ts`: JS <-> Bevy WASM bridge

### Bevy WASM client

- `game-client/src/lib.rs`
- Local fixed-step simulation and sequenced input emission
- Reconciliation against authoritative snapshots
- Rendering of players, structures, and projectiles

## Development

### Prerequisites

- Node.js 20+
- Rust toolchain
- `wasm-pack`
- `wasm32-unknown-unknown` target
- Cloudflare account + `wrangler login`

Install once:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

### Local env

Frontend:

- `.env.development` should include `VITE_CLERK_PUBLISHABLE_KEY`

Worker (local dev):

- `worker/.dev.vars` should include `CLERK_SECRET_KEY`

### Run locally

Install dependencies:

```bash
npm install
```

Start frontend:

```bash
npm run dev
```

Start worker:

```bash
npm run worker:dev
```

## Build

```bash
npm run build
```

## Deploy

`worker/wrangler.toml` is configured for:

- Durable Object namespace `ROOMS`
- SQLite DO migrations
- Static assets binding from `dist/`
- Custom domain route `will.ralph-island.com`

Ensure production secret exists:

```bash
wrangler secret put CLERK_SECRET_KEY --config worker/wrangler.toml
```

Deploy:

```bash
npm run deploy
```

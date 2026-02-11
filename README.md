# Ralph Island Multiplayer Skeleton

A production-oriented starter for a large multiplayer browser game:

- Frontend: Vite + React + Tailwind + TanStack Router
- Game client: Rust + Bevy compiled to WASM
- Multiplayer backend: Cloudflare Worker + Durable Objects + SQLite storage
- Transport: WebSockets
- Hosting target: `will.ralph-island.com`

## Current Scope

- Connect to a room code (`/`)
- Join shared world (`/room/:roomCode`)
- Move with WASD/arrow keys
- See all connected players in the same room
- Room state persisted in Durable Object SQLite storage (`players` table)

## Architecture

### Browser

1. React route `/room/:roomCode` boots Bevy WASM in a `<canvas>`.
2. Browser opens `ws(s)://<host>/api/rooms/:roomCode/ws?playerId=<stable-id>`.
3. Worker Durable Object sends snapshots of connected players.
4. React forwards snapshots into Rust (`push_snapshot`).
5. Rust emits local movement events (`drain_move_events`) which are sent back to Worker.

### Cloudflare Durable Object

- One DO instance per room code (`idFromName(roomCode)`)
- SQLite table in DO storage:
  - `player_id` (PK)
  - `x`, `y`
  - `connected`
  - `updated_at`
- On connect/move/close:
  - writes are persisted in SQLite
  - snapshot broadcasts to all room sockets

## Prerequisites

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

## Local Development

Install deps:

```bash
npm install
```

Run Worker (terminal 1):

```bash
npm run worker:dev
```

Run frontend (terminal 2):

```bash
npm run dev
```

Open the Vite URL from terminal 2. Use the same room code in multiple tabs.

## Build

```bash
npm run build
```

This runs:

1. Rust WASM build (`wasm-pack` into `src/game/wasm`)
2. TypeScript + Vite web build into `dist/`

## Deploy To Cloudflare

`worker/wrangler.toml` is already configured with:

- Durable Object binding `ROOMS`
- SQLite migration (`new_sqlite_classes = ["RoomDurableObject"]`)
- static assets from `../dist`
- custom domain route: `will.ralph-island.com/*`

Deploy:

```bash
npm run deploy
```

## Notes On Server Runtime Choice

You asked about running Rust inside Durable Objects and using Bevy ECS server-side. That is possible later with a dedicated Rust/WASM server module (for example using `bevy_ecs` only), but for this skeleton the DO authoritative loop is TypeScript for fastest iteration and simplest Cloudflare deployment path.

## Project Layout

- `src/` React app + WASM bridge
- `game-client/` Rust Bevy game crate
- `worker/` Cloudflare Worker + Durable Object backend

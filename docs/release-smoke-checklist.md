# Ralph Production Release Smoke Checklist

Target domain: `https://will.ralph-island.com`

## 1. Deploy gate

1. Export production env (`CLERK_SECRET_KEY`, `VITE_CLERK_PUBLISHABLE_KEY`).
2. Run deploy:
   - `npm run deploy`
3. Confirm health endpoint returns `ok: true`:
   - `curl https://will.ralph-island.com/api/health`

## 2. Automated two-player production smoke

Run:

```bash
npm run smoke:prod
```

The smoke runner performs these checks against production:

1. Creates two temporary Clerk users.
2. Mints two real session JWTs through Clerk ticket sign-in flow.
3. Connects both users to one room over `wss://will.ralph-island.com/api/rooms/:room/ws`.
4. Verifies welcome envelopes for both players.
5. Verifies authoritative snapshots include both player ids.
6. Sends `core.ping` from each player and verifies `ack` + `pong`.
7. Deletes the temporary Clerk users.

Optional controls:

- `RALPH_SMOKE_ROOM_CODE=SMOKE123 npm run smoke:prod`
- `RALPH_SMOKE_TIMEOUT_MS=45000 npm run smoke:prod`
- `RALPH_SMOKE_KEEP_USERS=1 npm run smoke:prod` (debug only)
- `RALPH_PROD_ORIGIN=https://will.ralph-island.com npm run smoke:prod`

## 3. Manual gameplay smoke (release checklist)

Two real players in one shared room should complete this loop on production:

1. Join the same `/room/:roomCode`.
2. Gather starter resources (ore pickups visible to both players).
3. Craft intermediate items.
4. Place at least one crafted structure (placement visible to both clients).
5. Engage at least one enemy and confirm health/damage/death feedback.
6. Reconnect one client and confirm controls/snapshots recover without hard refresh.

## 4. Known limitations (current vertical slice)

1. Building automation (belts/inserters/power/assembler behavior) is intentionally deferred.
2. Production smoke script validates transport/auth/authoritative snapshot-command path, not full combat economy balance.
3. Manual two-player playtest remains required before final release sign-off.

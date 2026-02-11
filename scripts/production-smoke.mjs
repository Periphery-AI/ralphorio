#!/usr/bin/env node

const PROTOCOL_VERSION = 2;
const ROOM_CODE_PREFIX = 'SMOKE';

function nowIso() {
  return new Date().toISOString();
}

function randomSuffix(length = 8) {
  return Math.random().toString(36).slice(2, 2 + length);
}

function decodePublishableKeyHost(publishableKey) {
  if (typeof publishableKey !== 'string' || publishableKey.length === 0) {
    throw new Error('missing Clerk publishable key');
  }

  const parts = publishableKey.split('_');
  if (parts.length < 3) {
    throw new Error('invalid Clerk publishable key format');
  }

  const encodedHost = parts.slice(2).join('_');
  const normalized = encodedHost.replace(/-/g, '+').replace(/_/g, '/');
  const decoded = Buffer.from(normalized, 'base64').toString('utf8');
  const host = decoded.endsWith('$') ? decoded.slice(0, -1) : decoded;
  if (!host) {
    throw new Error('invalid Clerk publishable key host');
  }

  return host;
}

async function parseJsonResponse(response) {
  const raw = await response.text();
  let payload = null;

  if (raw.length > 0) {
    try {
      payload = JSON.parse(raw);
    } catch {
      throw new Error(`Expected JSON but received: ${raw.slice(0, 200)}`);
    }
  }

  if (!response.ok) {
    const message =
      payload && Array.isArray(payload.errors) && payload.errors[0]?.long_message
        ? payload.errors[0].long_message
        : payload && Array.isArray(payload.errors) && payload.errors[0]?.message
          ? payload.errors[0].message
          : `HTTP ${response.status}`;
    throw new Error(message);
  }

  return payload;
}

async function postJson(url, token, body) {
  const response = await fetch(url, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(body),
  });

  return parseJsonResponse(response);
}

async function postForm(url, token, form) {
  const response = await fetch(url, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/x-www-form-urlencoded',
    },
    body: new URLSearchParams(form),
  });

  return parseJsonResponse(response);
}

async function deleteUser(secretKey, userId) {
  const response = await fetch(`https://api.clerk.com/v1/users/${encodeURIComponent(userId)}`, {
    method: 'DELETE',
    headers: {
      Authorization: `Bearer ${secretKey}`,
    },
  });

  if (!response.ok) {
    const raw = await response.text();
    throw new Error(`Failed to delete user ${userId}: HTTP ${response.status} ${raw.slice(0, 200)}`);
  }
}

function withTimeout(promise, timeoutMs, description) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`Timed out waiting for ${description} after ${timeoutMs}ms`));
    }, timeoutMs);

    promise
      .then((value) => {
        clearTimeout(timer);
        resolve(value);
      })
      .catch((error) => {
        clearTimeout(timer);
        reject(error);
      });
  });
}

function createEnvelope(seq, feature, action, payload) {
  return {
    v: PROTOCOL_VERSION,
    kind: 'command',
    seq,
    feature,
    action,
    clientTime: Date.now(),
    payload,
  };
}

function createSmokeClient({ label, roomCode, playerId, sessionToken, origin }) {
  const wsOrigin = new URL(origin);
  wsOrigin.protocol = wsOrigin.protocol === 'https:' ? 'wss:' : 'ws:';
  wsOrigin.pathname = `/api/rooms/${encodeURIComponent(roomCode)}/ws`;
  wsOrigin.search = `playerId=${encodeURIComponent(playerId)}&token=${encodeURIComponent(sessionToken)}`;

  const url = wsOrigin.toString();
  const websocket = new WebSocket(url);
  const state = {
    label,
    playerId,
    websocket,
    welcomed: false,
    gotPong: false,
    ackedSeqs: new Set(),
    snapshotPlayerIds: new Set(),
    closeCode: null,
    closeReason: '',
  };

  const listeners = new Set();

  function notify() {
    for (const listener of listeners) {
      listener();
    }
  }

  websocket.addEventListener('message', (event) => {
    let envelope;
    try {
      envelope = JSON.parse(String(event.data));
    } catch {
      return;
    }

    if (envelope.kind === 'welcome') {
      state.welcomed = true;
    }

    if (envelope.kind === 'ack' && Number.isInteger(envelope.seq)) {
      state.ackedSeqs.add(envelope.seq);
    }

    if (envelope.kind === 'pong') {
      state.gotPong = true;
    }

    if (envelope.kind === 'snapshot' && envelope.payload && envelope.payload.features?.movement) {
      const players = envelope.payload.features.movement.players;
      if (Array.isArray(players)) {
        for (const player of players) {
          if (player && typeof player.id === 'string') {
            state.snapshotPlayerIds.add(player.id);
          }
        }
      }
    }

    notify();
  });

  websocket.addEventListener('close', (event) => {
    state.closeCode = event.code;
    state.closeReason = event.reason;
    notify();
  });

  websocket.addEventListener('error', () => {
    notify();
  });

  function waitFor(predicate, description, timeoutMs) {
    return withTimeout(
      new Promise((resolve, reject) => {
        const maybeResolve = () => {
          if (predicate(state)) {
            listeners.delete(maybeResolve);
            resolve(undefined);
            return;
          }

          if (
            state.closeCode !== null &&
            state.closeCode !== 1000 &&
            !predicate(state)
          ) {
            listeners.delete(maybeResolve);
            reject(
              new Error(
                `${label} websocket closed early (code=${state.closeCode}, reason=${state.closeReason || 'none'})`,
              ),
            );
          }
        };

        listeners.add(maybeResolve);
        maybeResolve();
      }),
      timeoutMs,
      `${label}: ${description}`,
    );
  }

  function sendCommand(seq, feature, action, payload) {
    const envelope = createEnvelope(seq, feature, action, payload);
    websocket.send(JSON.stringify(envelope));
  }

  function close() {
    if (websocket.readyState === WebSocket.OPEN || websocket.readyState === WebSocket.CONNECTING) {
      websocket.close(1000, 'smoke complete');
    }
  }

  return {
    state,
    waitFor,
    sendCommand,
    close,
  };
}

async function ensureHealth(origin) {
  const response = await fetch(`${origin.replace(/\/$/, '')}/api/health`);
  const payload = await parseJsonResponse(response);
  if (!payload || payload.ok !== true) {
    throw new Error('production health endpoint did not return ok=true');
  }
}

async function createSessionJwtForUser({ secretKey, publishableKey, frontendApiOrigin, userIndex }) {
  const email = `ralph-smoke-${Date.now()}-${userIndex}-${randomSuffix(6)}@example.com`;
  const password = `TmpPassw0rd!${randomSuffix(12)}`;

  const createdUser = await postJson('https://api.clerk.com/v1/users', secretKey, {
    email_address: [email],
    password,
  });

  const userId = createdUser?.id;
  if (typeof userId !== 'string' || userId.length === 0) {
    throw new Error('Clerk user creation returned no user id');
  }

  const signInToken = await postJson('https://api.clerk.com/v1/sign_in_tokens', secretKey, {
    user_id: userId,
    expires_in_seconds: 600,
  });

  if (typeof signInToken?.token !== 'string' || signInToken.token.length === 0) {
    throw new Error(`Failed to create sign-in token for ${userId}`);
  }

  const signInAttempt = await postForm(`${frontendApiOrigin}/v1/client/sign_ins`, publishableKey, {
    strategy: 'ticket',
    ticket: signInToken.token,
  });

  const signInResponse = signInAttempt?.response ?? signInAttempt;
  const sessionId = signInResponse?.created_session_id;
  const signInStatus = signInResponse?.status;
  if (signInStatus !== 'complete' || typeof sessionId !== 'string' || sessionId.length === 0) {
    throw new Error(`Ticket sign-in did not complete for ${userId}`);
  }

  const sessionToken = await postJson(
    `https://api.clerk.com/v1/sessions/${encodeURIComponent(sessionId)}/tokens`,
    secretKey,
    {},
  );

  if (typeof sessionToken?.jwt !== 'string' || sessionToken.jwt.length === 0) {
    throw new Error(`Failed to mint session JWT for ${userId}`);
  }

  return {
    userId,
    sessionId,
    sessionToken: sessionToken.jwt,
  };
}

async function main() {
  const origin = (process.env.RALPH_PROD_ORIGIN ?? 'https://will.ralph-island.com').replace(/\/$/, '');
  const timeoutMs = Number.parseInt(process.env.RALPH_SMOKE_TIMEOUT_MS ?? '30000', 10);
  const keepUsers = process.env.RALPH_SMOKE_KEEP_USERS === '1';
  const roomCode = process.env.RALPH_SMOKE_ROOM_CODE ?? `${ROOM_CODE_PREFIX}${Date.now().toString(36).toUpperCase()}`;
  const secretKey = process.env.CLERK_SECRET_KEY ?? '';
  const publishableKey =
    process.env.CLERK_PUBLISHABLE_KEY ?? process.env.VITE_CLERK_PUBLISHABLE_KEY ?? '';

  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
    throw new Error('RALPH_SMOKE_TIMEOUT_MS must be a positive integer');
  }

  if (!secretKey) {
    throw new Error('CLERK_SECRET_KEY is required');
  }

  if (!publishableKey) {
    throw new Error('CLERK_PUBLISHABLE_KEY or VITE_CLERK_PUBLISHABLE_KEY is required');
  }

  const frontendHost = decodePublishableKeyHost(publishableKey);
  const frontendApiOrigin = `https://${frontendHost}`;

  console.log(`[${nowIso()}] smoke start origin=${origin} room=${roomCode}`);
  console.log(`[${nowIso()}] verifying health endpoint`);
  await ensureHealth(origin);

  const createdUsers = [];
  const clients = [];

  try {
    console.log(`[${nowIso()}] creating authenticated smoke users`);
    const userSessions = await Promise.all([
      createSessionJwtForUser({
        secretKey,
        publishableKey,
        frontendApiOrigin,
        userIndex: 1,
      }),
      createSessionJwtForUser({
        secretKey,
        publishableKey,
        frontendApiOrigin,
        userIndex: 2,
      }),
    ]);

    for (const session of userSessions) {
      createdUsers.push(session.userId);
    }

    const playerIds = userSessions.map((session) => session.userId);
    console.log(`[${nowIso()}] connecting websocket clients`);

    const first = createSmokeClient({
      label: 'player-1',
      roomCode,
      playerId: userSessions[0].userId,
      sessionToken: userSessions[0].sessionToken,
      origin,
    });
    const second = createSmokeClient({
      label: 'player-2',
      roomCode,
      playerId: userSessions[1].userId,
      sessionToken: userSessions[1].sessionToken,
      origin,
    });

    clients.push(first, second);

    await Promise.all([
      first.waitFor((state) => state.welcomed, 'welcome envelope', timeoutMs),
      second.waitFor((state) => state.welcomed, 'welcome envelope', timeoutMs),
    ]);
    console.log(`[${nowIso()}] welcome envelopes received for both players`);

    await Promise.all([
      first.waitFor(
        (state) => playerIds.every((playerId) => state.snapshotPlayerIds.has(playerId)),
        'snapshot containing both players',
        timeoutMs,
      ),
      second.waitFor(
        (state) => playerIds.every((playerId) => state.snapshotPlayerIds.has(playerId)),
        'snapshot containing both players',
        timeoutMs,
      ),
    ]);
    console.log(`[${nowIso()}] authoritative snapshots include both players`);

    first.sendCommand(1, 'core', 'ping', null);
    second.sendCommand(1, 'core', 'ping', null);

    await Promise.all([
      first.waitFor((state) => state.ackedSeqs.has(1), 'ack for ping command seq=1', timeoutMs),
      second.waitFor((state) => state.ackedSeqs.has(1), 'ack for ping command seq=1', timeoutMs),
      first.waitFor((state) => state.gotPong, 'pong response', timeoutMs),
      second.waitFor((state) => state.gotPong, 'pong response', timeoutMs),
    ]);
    console.log(`[${nowIso()}] command ack + pong verified for both players`);

    console.log(`[${nowIso()}] smoke PASS room=${roomCode}`);
  } finally {
    for (const client of clients) {
      client.close();
    }

    if (!keepUsers) {
      const cleanupResults = await Promise.allSettled(
        createdUsers.map((userId) => deleteUser(secretKey, userId)),
      );
      const failures = cleanupResults.filter((result) => result.status === 'rejected');
      if (failures.length > 0) {
        const message = failures
          .map((result) => (result.status === 'rejected' ? result.reason?.message : ''))
          .join('; ');
        throw new Error(`Smoke succeeded but cleanup failed: ${message}`);
      }
      if (createdUsers.length > 0) {
        console.log(`[${nowIso()}] cleaned up ${createdUsers.length} temporary users`);
      }
    } else {
      console.log(
        `[${nowIso()}] skipped user cleanup because RALPH_SMOKE_KEEP_USERS=1 (users=${createdUsers.join(',')})`,
      );
    }
  }
}

main().catch((error) => {
  console.error(`[${nowIso()}] smoke FAIL ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});

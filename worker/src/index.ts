import { DurableObject } from 'cloudflare:workers';

export interface Env {
  ASSETS: Fetcher;
  ROOMS: DurableObjectNamespace;
}

type PlayerState = {
  id: string;
  x: number;
  y: number;
  connected: boolean;
};

type MoveMessage = {
  type: 'move';
  x: number;
  y: number;
};

type WelcomeMessage = {
  type: 'welcome';
  roomCode: string;
  playerId: string;
  players: PlayerState[];
};

type SnapshotMessage = {
  type: 'snapshot';
  players: PlayerState[];
};

const PLAYER_ID_RE = /^[a-zA-Z0-9_-]{3,40}$/;
const ROOM_RE = /^[a-zA-Z0-9_-]{1,24}$/;

function jsonResponse(payload: unknown, init?: ResponseInit) {
  return new Response(JSON.stringify(payload), {
    ...init,
    headers: {
      'content-type': 'application/json; charset=utf-8',
      ...(init?.headers ?? {}),
    },
  });
}

function parseRoomCode(pathname: string) {
  const parts = pathname.split('/');
  if (parts.length !== 5) {
    return null;
  }
  if (parts[1] !== 'api' || parts[2] !== 'rooms' || parts[4] !== 'ws') {
    return null;
  }

  const candidate = decodeURIComponent(parts[3] ?? '').toUpperCase();
  return ROOM_RE.test(candidate) ? candidate : null;
}

function sanitizePlayerId(value: string | null) {
  if (!value) {
    return null;
  }

  return PLAYER_ID_RE.test(value) ? value : null;
}

function clampCoordinate(value: number) {
  if (!Number.isFinite(value)) {
    return 0;
  }

  return Math.max(-5000, Math.min(5000, value));
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.pathname === '/api/health') {
      return jsonResponse({ ok: true, timestamp: Date.now() });
    }

    const roomCode = parseRoomCode(url.pathname);
    if (roomCode) {
      if (request.headers.get('Upgrade')?.toLowerCase() !== 'websocket') {
        return jsonResponse({ error: 'Expected websocket upgrade.' }, { status: 426 });
      }

      const id = env.ROOMS.idFromName(roomCode);
      const stub = env.ROOMS.get(id);
      return stub.fetch(request);
    }

    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler<Env>;

export class RoomDurableObject extends DurableObject {
  private readonly sql: SqlStorage;
  private roomCode = 'UNKNOWN';

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;

    ctx.blockConcurrencyWhile(async () => {
      this.sql.exec(`
        CREATE TABLE IF NOT EXISTS players (
          player_id TEXT PRIMARY KEY,
          x REAL NOT NULL DEFAULT 0,
          y REAL NOT NULL DEFAULT 0,
          connected INTEGER NOT NULL DEFAULT 0,
          updated_at INTEGER NOT NULL
        )
      `);
    });
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const roomCode = parseRoomCode(url.pathname);

    if (!roomCode) {
      return jsonResponse({ error: 'Invalid room endpoint.' }, { status: 404 });
    }

    this.roomCode = roomCode;

    if (request.headers.get('Upgrade')?.toLowerCase() !== 'websocket') {
      return jsonResponse({ error: 'WebSocket upgrade required.' }, { status: 426 });
    }

    const playerId =
      sanitizePlayerId(url.searchParams.get('playerId')) ?? crypto.randomUUID().replace(/-/g, '').slice(0, 20);

    const pair = new WebSocketPair();
    const client = pair[0];
    const server = pair[1];

    this.upsertPlayer(playerId, 0, 0, true);
    this.ctx.acceptWebSocket(server, [playerId]);
    server.serializeAttachment({ playerId });

    const welcome: WelcomeMessage = {
      type: 'welcome',
      roomCode: this.roomCode,
      playerId,
      players: this.connectedPlayers(),
    };
    server.send(JSON.stringify(welcome));

    this.broadcastSnapshot();

    return new Response(null, {
      status: 101,
      webSocket: client,
    });
  }

  webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): void {
    if (typeof message !== 'string') {
      return;
    }

    let parsed: MoveMessage;
    try {
      parsed = JSON.parse(message) as MoveMessage;
    } catch {
      return;
    }

    if (parsed.type !== 'move') {
      return;
    }

    const playerId = this.playerIdForSocket(ws);
    if (!playerId) {
      return;
    }

    const x = clampCoordinate(parsed.x);
    const y = clampCoordinate(parsed.y);

    this.upsertPlayer(playerId, x, y, true);
    this.broadcastSnapshot();
  }

  webSocketClose(ws: WebSocket): void {
    const playerId = this.playerIdForSocket(ws);
    if (!playerId) {
      return;
    }

    this.setConnected(playerId, false);
    this.broadcastSnapshot();
  }

  webSocketError(ws: WebSocket): void {
    const playerId = this.playerIdForSocket(ws);
    if (!playerId) {
      return;
    }

    this.setConnected(playerId, false);
    this.broadcastSnapshot();
  }

  private playerIdForSocket(ws: WebSocket) {
    const attachment = ws.deserializeAttachment() as { playerId?: string } | null;
    const playerId = attachment?.playerId;
    return playerId && PLAYER_ID_RE.test(playerId) ? playerId : null;
  }

  private connectedPlayers() {
    const rows = this.sql.exec(
      'SELECT player_id AS id, x, y, connected FROM players WHERE connected = 1 ORDER BY updated_at ASC',
    );

    const players: PlayerState[] = [];
    for (const row of rows) {
      players.push({
        id: String(row.id),
        x: Number(row.x),
        y: Number(row.y),
        connected: Number(row.connected) === 1,
      });
    }

    return players;
  }

  private upsertPlayer(playerId: string, x: number, y: number, connected: boolean) {
    const now = Date.now();
    this.sql.exec(
      `
      INSERT INTO players (player_id, x, y, connected, updated_at)
      VALUES (?1, ?2, ?3, ?4, ?5)
      ON CONFLICT(player_id) DO UPDATE
      SET
        x = excluded.x,
        y = excluded.y,
        connected = excluded.connected,
        updated_at = excluded.updated_at
      `,
      playerId,
      x,
      y,
      connected ? 1 : 0,
      now,
    );
  }

  private setConnected(playerId: string, connected: boolean) {
    this.sql.exec(
      'UPDATE players SET connected = ?1, updated_at = ?2 WHERE player_id = ?3',
      connected ? 1 : 0,
      Date.now(),
      playerId,
    );
  }

  private broadcastSnapshot() {
    const payload: SnapshotMessage = {
      type: 'snapshot',
      players: this.connectedPlayers(),
    };

    const encoded = JSON.stringify(payload);
    for (const socket of this.ctx.getWebSockets()) {
      try {
        socket.send(encoded);
      } catch {
        // Ignore dead sockets; close callback handles persistence cleanup.
      }
    }
  }
}

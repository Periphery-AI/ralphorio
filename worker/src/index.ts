import { DurableObject } from 'cloudflare:workers';
import type { FeatureCommandResult, FeatureContext, RoomFeature } from './framework/feature';
import { applyFeatureMigrations } from './framework/migrations';
import {
  PROTOCOL_VERSION,
  parseClientEnvelope,
  type ClientCommandEnvelope,
  type ServerEnvelope,
} from './framework/protocol';
import { PresenceFeature } from './features/presence-feature';
import { MovementFeature } from './features/movement-feature';
import { BuildFeature } from './features/build-feature';
import { ProjectileFeature } from './features/projectile-feature';

export interface Env {
  ASSETS: Fetcher;
  ROOMS: DurableObjectNamespace;
  CLERK_SECRET_KEY?: string;
}

type Session = {
  playerId: string;
  lastSeq: number;
  joinedAt: number;
};

type SocketAttachment = {
  playerId: string;
  lastSeq: number;
};

const PLAYER_ID_RE = /^[a-zA-Z0-9_-]{3,120}$/;
const ROOM_RE = /^[a-zA-Z0-9_-]{1,24}$/;

const SIM_RATE_HZ = 60;
const SNAPSHOT_RATE_HZ = 20;
const SIM_DT_SECONDS = 1 / SIM_RATE_HZ;
const SIM_DT_MS = Math.round(1000 / SIM_RATE_HZ);
const SNAPSHOT_INTERVAL_TICKS = Math.max(1, Math.round(SIM_RATE_HZ / SNAPSHOT_RATE_HZ));
const LOOP_INTERVAL_MS = 16;
const MAX_CATCHUP_STEPS = 8;

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
  private tick = 0;

  private tickTimer: ReturnType<typeof setInterval> | null = null;
  private accumulatorMs = 0;
  private lastLoopAt = Date.now();
  private snapshotDirty = false;

  private readonly features: RoomFeature[];
  private readonly featureByKey: Map<string, RoomFeature>;
  private readonly sessions = new Map<WebSocket, Session>();

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;

    this.features = [
      new PresenceFeature(),
      new MovementFeature(),
      new BuildFeature(),
      new ProjectileFeature(),
    ];
    this.featureByKey = new Map(this.features.map((feature) => [feature.key, feature]));

    for (const socket of ctx.getWebSockets()) {
      const attachment = socket.deserializeAttachment() as SocketAttachment | null;
      if (!attachment || !sanitizePlayerId(attachment.playerId)) {
        continue;
      }

      this.sessions.set(socket, {
        playerId: attachment.playerId,
        lastSeq: Number.isInteger(attachment.lastSeq) ? attachment.lastSeq : 0,
        joinedAt: Date.now(),
      });
    }

    ctx.blockConcurrencyWhile(async () => {
      applyFeatureMigrations(this.sql, this.features);
      if (this.sessions.size > 0) {
        this.startTickLoop();
      }
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

    this.ctx.acceptWebSocket(server, [playerId]);

    const session: Session = {
      playerId,
      lastSeq: 0,
      joinedAt: Date.now(),
    };

    server.serializeAttachment({ playerId, lastSeq: 0 } satisfies SocketAttachment);
    this.sessions.set(server, session);

    const ctx = this.createFeatureContext(0);
    for (const feature of this.features) {
      const result = feature.onConnect(ctx, playerId);
      this.dispatchFeatureResult(server, result);
    }

    this.sendEnvelope(server, {
      v: PROTOCOL_VERSION,
      kind: 'welcome',
      tick: this.tick,
      serverTime: Date.now(),
      feature: 'core',
      action: 'connected',
      payload: {
        roomCode: this.roomCode,
        playerId,
        simRateHz: SIM_RATE_HZ,
        snapshotRateHz: SNAPSHOT_RATE_HZ,
      },
    });

    this.sendSnapshot(server);
    this.broadcastSnapshot();
    this.startTickLoop();

    return new Response(null, {
      status: 101,
      webSocket: client,
    });
  }

  webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): void {
    const session = this.sessions.get(ws);
    if (!session) {
      return;
    }

    if (typeof message !== 'string') {
      this.sendError(ws, 'core', 'invalid_message', 'WebSocket payload must be text JSON.');
      return;
    }

    const envelope = parseClientEnvelope(message);
    if (!envelope) {
      this.sendError(ws, 'core', 'invalid_message', 'Malformed protocol envelope.');
      return;
    }

    this.handleClientCommand(ws, session, envelope);
  }

  webSocketClose(ws: WebSocket): void {
    this.handleDisconnect(ws);
  }

  webSocketError(ws: WebSocket): void {
    this.handleDisconnect(ws);
  }

  private handleClientCommand(ws: WebSocket, session: Session, envelope: ClientCommandEnvelope) {
    if (envelope.seq <= session.lastSeq) {
      this.sendAck(ws, envelope.feature, envelope.action, envelope.seq);
      return;
    }

    session.lastSeq = envelope.seq;
    ws.serializeAttachment({ playerId: session.playerId, lastSeq: session.lastSeq } satisfies SocketAttachment);

    if (envelope.feature === 'core' && envelope.action === 'ping') {
      this.sendEnvelope(ws, {
        v: PROTOCOL_VERSION,
        kind: 'pong',
        tick: this.tick,
        serverTime: Date.now(),
        feature: 'core',
        action: 'pong',
        seq: envelope.seq,
        payload: {
          clientTime: envelope.clientTime,
        },
      });
      this.sendAck(ws, envelope.feature, envelope.action, envelope.seq);
      return;
    }

    const feature = this.featureByKey.get(envelope.feature);
    if (!feature) {
      this.sendError(ws, envelope.feature, envelope.action, `Unknown feature '${envelope.feature}'.`);
      this.sendAck(ws, envelope.feature, envelope.action, envelope.seq);
      return;
    }

    const ctx = this.createFeatureContext(0);
    const result = feature.onCommand(ctx, session.playerId, envelope.action, envelope.payload, envelope.seq);

    this.dispatchFeatureResult(ws, result);
    this.sendAck(ws, envelope.feature, envelope.action, envelope.seq);

    if (result?.stateChanged) {
      this.snapshotDirty = true;
      this.broadcastSnapshot();
      this.snapshotDirty = false;
    }
  }

  private handleDisconnect(ws: WebSocket) {
    const session = this.sessions.get(ws);
    if (!session) {
      return;
    }

    this.sessions.delete(ws);

    const ctx = this.createFeatureContext(0);
    for (const feature of this.features) {
      const result = feature.onDisconnect(ctx, session.playerId);
      this.dispatchFeatureResult(ws, result);
    }

    this.broadcastSnapshot();
    this.stopTickLoopIfIdle();
  }

  private dispatchFeatureResult(ws: WebSocket, result: FeatureCommandResult | void) {
    if (!result?.events || result.events.length === 0) {
      return;
    }

    for (const event of result.events) {
      if (event.target === 'room') {
        this.broadcastEnvelope({
          v: PROTOCOL_VERSION,
          kind: 'event',
          tick: this.tick,
          serverTime: Date.now(),
          feature: event.feature,
          action: event.action,
          payload: event.payload,
        });
        continue;
      }

      if (event.target === 'self') {
        this.sendEnvelope(ws, {
          v: PROTOCOL_VERSION,
          kind: 'event',
          tick: this.tick,
          serverTime: Date.now(),
          feature: event.feature,
          action: event.action,
          payload: event.payload,
        });
        continue;
      }

      if (event.target === 'player' && event.playerId) {
        this.sendToPlayer(event.playerId, {
          v: PROTOCOL_VERSION,
          kind: 'event',
          tick: this.tick,
          serverTime: Date.now(),
          feature: event.feature,
          action: event.action,
          payload: event.payload,
        });
      }
    }
  }

  private createFeatureContext(tickDeltaSeconds: number): FeatureContext {
    const connectedPlayerIds = this.connectedPlayerIds();
    return {
      sql: this.sql,
      roomCode: this.roomCode,
      now: Date.now(),
      tick: this.tick,
      tickDeltaSeconds,
      connectedPlayerIds,
    };
  }

  private connectedPlayerIds() {
    const ids = new Set<string>();
    for (const session of this.sessions.values()) {
      ids.add(session.playerId);
    }
    return [...ids.values()];
  }

  private sendAck(ws: WebSocket, feature: string, action: string, seq: number) {
    this.sendEnvelope(ws, {
      v: PROTOCOL_VERSION,
      kind: 'ack',
      tick: this.tick,
      serverTime: Date.now(),
      feature,
      action,
      seq,
      payload: {
        serverTime: Date.now(),
      },
    });
  }

  private sendError(ws: WebSocket, feature: string, action: string, message: string) {
    this.sendEnvelope(ws, {
      v: PROTOCOL_VERSION,
      kind: 'error',
      tick: this.tick,
      serverTime: Date.now(),
      feature,
      action,
      payload: {
        message,
      },
    });
  }

  private sendEnvelope(ws: WebSocket, envelope: ServerEnvelope) {
    try {
      ws.send(JSON.stringify(envelope));
    } catch {
      // ignore closed sockets
    }
  }

  private broadcastEnvelope(envelope: ServerEnvelope) {
    const encoded = JSON.stringify(envelope);
    for (const socket of this.sessions.keys()) {
      try {
        socket.send(encoded);
      } catch {
        // ignore
      }
    }
  }

  private sendToPlayer(playerId: string, envelope: ServerEnvelope) {
    for (const [socket, session] of this.sessions.entries()) {
      if (session.playerId === playerId) {
        this.sendEnvelope(socket, envelope);
      }
    }
  }

  private snapshotPayload() {
    const ctx = this.createFeatureContext(0);
    const features: Record<string, unknown> = {};

    for (const feature of this.features) {
      features[feature.key] = feature.createSnapshot(ctx);
    }

    return {
      roomCode: this.roomCode,
      serverTick: this.tick,
      simRateHz: SIM_RATE_HZ,
      snapshotRateHz: SNAPSHOT_RATE_HZ,
      serverTime: Date.now(),
      features,
    };
  }

  private sendSnapshot(ws: WebSocket) {
    this.sendEnvelope(ws, {
      v: PROTOCOL_VERSION,
      kind: 'snapshot',
      tick: this.tick,
      serverTime: Date.now(),
      feature: 'core',
      action: 'state',
      payload: this.snapshotPayload(),
    });
  }

  private broadcastSnapshot() {
    this.broadcastEnvelope({
      v: PROTOCOL_VERSION,
      kind: 'snapshot',
      tick: this.tick,
      serverTime: Date.now(),
      feature: 'core',
      action: 'state',
      payload: this.snapshotPayload(),
    });
  }

  private startTickLoop() {
    if (this.tickTimer || this.sessions.size === 0) {
      return;
    }

    this.lastLoopAt = Date.now();
    this.accumulatorMs = 0;

    this.tickTimer = setInterval(() => {
      if (this.sessions.size === 0) {
        this.stopTickLoopIfIdle();
        return;
      }

      const now = Date.now();
      const elapsed = Math.max(0, now - this.lastLoopAt);
      this.lastLoopAt = now;

      this.accumulatorMs += Math.min(elapsed, 250);
      let simulatedSteps = 0;

      while (this.accumulatorMs >= SIM_DT_MS && simulatedSteps < MAX_CATCHUP_STEPS) {
        this.runSimulationStep();
        this.accumulatorMs -= SIM_DT_MS;
        simulatedSteps += 1;
      }

      if (simulatedSteps === MAX_CATCHUP_STEPS && this.accumulatorMs >= SIM_DT_MS) {
        // Drop excess accumulated time to avoid permanent spiral-of-death after stalls.
        this.accumulatorMs = 0;
      }
    }, LOOP_INTERVAL_MS);
  }

  private runSimulationStep() {
    this.tick += 1;

    const ctx = this.createFeatureContext(SIM_DT_SECONDS);
    for (const feature of this.features) {
      const result = feature.onTick(ctx);

      if (result?.events && result.events.length > 0) {
        for (const event of result.events) {
          if (event.target !== 'room') {
            continue;
          }

          this.broadcastEnvelope({
            v: PROTOCOL_VERSION,
            kind: 'event',
            tick: this.tick,
            serverTime: Date.now(),
            feature: event.feature,
            action: event.action,
            payload: event.payload,
          });
        }
      }

      if (result?.stateChanged) {
        this.snapshotDirty = true;
      }
    }

    if (this.tick % SNAPSHOT_INTERVAL_TICKS === 0 || this.snapshotDirty) {
      this.broadcastSnapshot();
      this.snapshotDirty = false;
    }
  }

  private stopTickLoopIfIdle() {
    if (this.sessions.size > 0) {
      return;
    }

    if (this.tickTimer) {
      clearInterval(this.tickTimer);
      this.tickTimer = null;
    }
  }
}

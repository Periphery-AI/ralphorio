import type {
  ClientCommandEnvelope,
  InputCommand,
  RoomSnapshot,
  ServerEnvelope,
  WelcomePayload,
} from './types';
import { PROTOCOL_VERSION } from './types';

type Handlers = {
  onWelcome: (payload: WelcomePayload) => void;
  onSnapshot: (snapshot: RoomSnapshot) => void;
  onAck: (seq: number, feature: string, action: string) => void;
  onStatus: (status: string) => void;
  onEvent: (feature: string, action: string, payload: unknown) => void;
  onPong?: (latencyMs: number) => void;
};

function buildWebSocketUrl(roomCode: string, playerId: string) {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const host = window.location.host;
  const encodedRoom = encodeURIComponent(roomCode);
  const encodedPlayer = encodeURIComponent(playerId);
  return `${protocol}//${host}/api/rooms/${encodedRoom}/ws?playerId=${encodedPlayer}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function parseServerEnvelope(raw: string): ServerEnvelope | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }

  if (!isRecord(parsed)) {
    return null;
  }

  if (parsed.v !== PROTOCOL_VERSION) {
    return null;
  }

  if (
    typeof parsed.kind !== 'string' ||
    typeof parsed.feature !== 'string' ||
    typeof parsed.action !== 'string' ||
    typeof parsed.tick !== 'number' ||
    typeof parsed.serverTime !== 'number'
  ) {
    return null;
  }

  return {
    v: PROTOCOL_VERSION,
    kind: parsed.kind as ServerEnvelope['kind'],
    tick: parsed.tick,
    serverTime: parsed.serverTime,
    feature: parsed.feature,
    action: parsed.action,
    seq: typeof parsed.seq === 'number' ? parsed.seq : undefined,
    payload: parsed.payload,
  };
}

function parseWelcomePayload(payload: unknown): WelcomePayload | null {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.roomCode !== 'string' ||
    typeof payload.playerId !== 'string' ||
    typeof payload.simRateHz !== 'number' ||
    typeof payload.snapshotRateHz !== 'number'
  ) {
    return null;
  }

  return {
    roomCode: payload.roomCode,
    playerId: payload.playerId,
    simRateHz: payload.simRateHz,
    snapshotRateHz: payload.snapshotRateHz,
  };
}

function parseRoomSnapshot(payload: unknown): RoomSnapshot | null {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    typeof payload.roomCode !== 'string' ||
    typeof payload.serverTick !== 'number' ||
    typeof payload.simRateHz !== 'number' ||
    typeof payload.snapshotRateHz !== 'number' ||
    typeof payload.serverTime !== 'number' ||
    !isRecord(payload.features)
  ) {
    return null;
  }

  return payload as RoomSnapshot;
}

export class RoomSocket {
  private socket: WebSocket | null = null;
  private readonly roomCode: string;
  private readonly playerId: string;
  private readonly handlers: Handlers;
  private seq = 1;
  private pingTimer: number | null = null;
  private pingSentAt = new Map<number, number>();

  constructor(roomCode: string, playerId: string, handlers: Handlers) {
    this.roomCode = roomCode;
    this.playerId = playerId;
    this.handlers = handlers;
  }

  connect() {
    const url = buildWebSocketUrl(this.roomCode, this.playerId);
    this.handlers.onStatus('Connecting...');

    this.socket = new WebSocket(url);

    this.socket.addEventListener('open', () => {
      this.handlers.onStatus('Connected');
      this.startPingLoop();
    });

    this.socket.addEventListener('message', (event) => {
      if (typeof event.data !== 'string') {
        return;
      }

      const envelope = parseServerEnvelope(event.data);
      if (!envelope) {
        return;
      }

      if (envelope.kind === 'welcome') {
        const payload = parseWelcomePayload(envelope.payload);
        if (!payload) {
          return;
        }
        this.handlers.onWelcome(payload);
        return;
      }

      if (envelope.kind === 'snapshot') {
        const snapshot = parseRoomSnapshot(envelope.payload);
        if (!snapshot) {
          return;
        }

        this.handlers.onSnapshot(snapshot);
        return;
      }

      if (envelope.kind === 'ack') {
        if (typeof envelope.seq === 'number') {
          this.handlers.onAck(envelope.seq, envelope.feature, envelope.action);
        }
        return;
      }

      if (envelope.kind === 'event') {
        this.handlers.onEvent(envelope.feature, envelope.action, envelope.payload);
        return;
      }

      if (envelope.kind === 'pong') {
        if (typeof envelope.seq === 'number') {
          const sentAt = this.pingSentAt.get(envelope.seq);
          if (sentAt !== undefined && this.handlers.onPong) {
            this.handlers.onPong(performance.now() - sentAt);
          }
          this.pingSentAt.delete(envelope.seq);
        }
        return;
      }

      if (envelope.kind === 'error') {
        this.handlers.onStatus(`Error: ${envelope.feature}.${envelope.action}`);
      }
    });

    this.socket.addEventListener('close', () => {
      this.stopPingLoop();
      this.handlers.onStatus('Disconnected');
    });

    this.socket.addEventListener('error', () => {
      this.stopPingLoop();
      this.handlers.onStatus('Connection error');
    });
  }

  sendInputCommands(inputs: InputCommand[]) {
    if (inputs.length === 0) {
      return;
    }

    this.sendCommand({
      feature: 'movement',
      action: 'input_batch',
      payload: {
        inputs,
      },
    });
  }

  sendBuildPlace(x: number, y: number, kind = 'beacon') {
    this.sendCommand({
      feature: 'build',
      action: 'place',
      payload: {
        x,
        y,
        kind,
        clientBuildId: `build_${crypto.randomUUID()}`,
      },
    });
  }

  sendProjectileFire(params: { x: number; y: number; vx: number; vy: number; clientProjectileId: string }) {
    this.sendCommand({
      feature: 'projectile',
      action: 'fire',
      payload: params,
    });
  }

  private sendPing() {
    const seq = this.sendCommand({
      feature: 'core',
      action: 'ping',
      payload: null,
    });

    if (seq !== null) {
      this.pingSentAt.set(seq, performance.now());
    }
  }

  private startPingLoop() {
    this.stopPingLoop();
    this.sendPing();
    this.pingTimer = window.setInterval(() => {
      this.sendPing();
    }, 2000);
  }

  private stopPingLoop() {
    if (this.pingTimer !== null) {
      window.clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
    this.pingSentAt.clear();
  }

  private sendCommand(params: { feature: string; action: string; payload?: unknown }): number | null {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) {
      return null;
    }

    const currentSeq = this.seq;
    const envelope: ClientCommandEnvelope = {
      v: PROTOCOL_VERSION,
      kind: 'command',
      seq: currentSeq,
      feature: params.feature,
      action: params.action,
      payload: params.payload,
      clientTime: performance.now(),
    };

    this.seq += 1;
    this.socket.send(JSON.stringify(envelope));
    return currentSeq;
  }

  disconnect() {
    this.stopPingLoop();
    if (this.socket) {
      this.socket.close();
      this.socket = null;
    }
  }
}

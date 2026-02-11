export const PROTOCOL_VERSION = 2;

export type ClientCommandEnvelope = {
  v: typeof PROTOCOL_VERSION;
  kind: 'command';
  seq: number;
  feature: string;
  action: string;
  clientTime: number;
  payload?: unknown;
};

export type ServerEnvelopeKind = 'welcome' | 'ack' | 'snapshot' | 'event' | 'error' | 'pong';

export type ServerEnvelope = {
  v: typeof PROTOCOL_VERSION;
  kind: ServerEnvelopeKind;
  tick: number;
  serverTime: number;
  feature: string;
  action: string;
  seq?: number;
  payload?: unknown;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

export function parseClientEnvelope(raw: string): ClientCommandEnvelope | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }

  if (!isRecord(parsed)) {
    return null;
  }

  const v = parsed.v;
  const kind = parsed.kind;
  const seq = parsed.seq;
  const feature = parsed.feature;
  const action = parsed.action;
  const clientTime = parsed.clientTime;

  if (v !== PROTOCOL_VERSION) {
    return null;
  }
  if (kind !== 'command') {
    return null;
  }
  if (typeof seq !== 'number' || !Number.isInteger(seq) || seq < 1) {
    return null;
  }
  if (typeof feature !== 'string' || feature.length < 1 || feature.length > 64) {
    return null;
  }
  if (typeof action !== 'string' || action.length < 1 || action.length > 64) {
    return null;
  }
  if (typeof clientTime !== 'number' || !Number.isFinite(clientTime)) {
    return null;
  }

  return {
    v,
    kind,
    seq,
    feature,
    action,
    clientTime,
    payload: parsed.payload,
  };
}

import type { MoveEvent, PlayerState } from './types';
import init, {
  boot_game,
  set_player_id,
  push_snapshot,
  drain_move_events,
} from './wasm/client';

let booted = false;
let initialized = false;

function isControlFlowException(error: unknown) {
  if (!(error instanceof Error)) {
    return false;
  }

  return error.message.includes("Using exceptions for control flow, don't mind me.");
}

async function initialize() {
  if (initialized) {
    return;
  }
  await init();
  initialized = true;
}

export async function bootGame(canvasId: string) {
  await initialize();

  if (booted) {
    return;
  }

  try {
    boot_game(canvasId);
  } catch (error) {
    if (!isControlFlowException(error)) {
      throw error;
    }
  }
  booted = true;
}

export async function setPlayerId(playerId: string) {
  await initialize();
  set_player_id(playerId);
}

export async function pushSnapshot(players: PlayerState[]) {
  await initialize();
  push_snapshot(JSON.stringify({ players }));
}

export async function drainMoveEvents() {
  await initialize();

  try {
    const raw = drain_move_events();
    return JSON.parse(raw) as MoveEvent[];
  } catch (error) {
    console.error('Failed to parse move events from WASM.', error);
    return [];
  }
}

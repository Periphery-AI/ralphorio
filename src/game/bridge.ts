import type { InputCommand, RenderSnapshotPayload } from './types';
import init, {
  boot_game,
  set_player_id,
  push_snapshot,
  drain_input_events,
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

export async function pushRenderSnapshot(payload: RenderSnapshotPayload) {
  await initialize();
  push_snapshot(JSON.stringify(payload));
}

export async function drainInputCommands() {
  await initialize();

  try {
    const raw = drain_input_events();
    return JSON.parse(raw) as InputCommand[];
  } catch (error) {
    console.error('Failed to parse input commands from WASM.', error);
    return [];
  }
}

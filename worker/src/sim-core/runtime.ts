import simCoreWasm from './sim_core.wasm';

type SimCoreExports = {
  sim_compute_velocity_x: (up: number, down: number, left: number, right: number, speed: number) => number;
  sim_compute_velocity_y: (up: number, down: number, left: number, right: number, speed: number) => number;
  sim_integrate_position: (position: number, velocity: number, dtSeconds: number, mapLimit: number) => number;
};

type InputState = {
  up: boolean;
  down: boolean;
  left: boolean;
  right: boolean;
};

let exportsCache: SimCoreExports | null = null;

function instantiateExports(): SimCoreExports {
  if (exportsCache) {
    return exportsCache;
  }

  const wasmSource = simCoreWasm as unknown;
  let module: WebAssembly.Module;

  if (wasmSource instanceof WebAssembly.Module) {
    module = wasmSource;
  } else if (wasmSource instanceof ArrayBuffer) {
    module = new WebAssembly.Module(wasmSource);
  } else {
    module = wasmSource as WebAssembly.Module;
  }

  const instance = new WebAssembly.Instance(module, {});
  exportsCache = instance.exports as unknown as SimCoreExports;

  return exportsCache;
}

function bit(value: boolean) {
  return value ? 1 : 0;
}

export function movementStep(params: {
  x: number;
  y: number;
  input: InputState;
  dtSeconds: number;
  speed: number;
  mapLimit: number;
}) {
  const sim = instantiateExports();
  const up = bit(params.input.up);
  const down = bit(params.input.down);
  const left = bit(params.input.left);
  const right = bit(params.input.right);

  const vx = sim.sim_compute_velocity_x(up, down, left, right, params.speed);
  const vy = sim.sim_compute_velocity_y(up, down, left, right, params.speed);

  return {
    vx,
    vy,
    x: sim.sim_integrate_position(params.x, vx, params.dtSeconds, params.mapLimit),
    y: sim.sim_integrate_position(params.y, vy, params.dtSeconds, params.mapLimit),
  };
}

export function projectileStep(params: {
  x: number;
  y: number;
  vx: number;
  vy: number;
  dtSeconds: number;
  mapLimit: number;
}) {
  const sim = instantiateExports();

  return {
    x: sim.sim_integrate_position(params.x, params.vx, params.dtSeconds, params.mapLimit),
    y: sim.sim_integrate_position(params.y, params.vy, params.dtSeconds, params.mapLimit),
  };
}

export function clampPosition(value: number, mapLimit: number) {
  const sim = instantiateExports();
  return sim.sim_integrate_position(value, 0, 0, mapLimit);
}

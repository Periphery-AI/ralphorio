import { Link, useParams } from '@tanstack/react-router';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useUser } from '@clerk/clerk-react';
import { bootGame, drainInputCommands, pushRenderSnapshot, setPlayerId } from '../game/bridge';
import { RoomSocket } from '../game/network-client';
import { ReplicationPipeline } from '../game/netcode/replication';
import type { InputCommand, InputState, PlayerState, RoomSnapshot } from '../game/types';

const CANVAS_ID = 'bevy-game-canvas';
const PROJECTILE_SPEED = 620;

function displayNameForUser(userName: string | null, firstName: string | null, userId: string) {
  if (userName) {
    return userName;
  }

  if (firstName) {
    return firstName;
  }

  return userId.slice(0, 16);
}

function localPlayerPosition(players: PlayerState[], playerId: string) {
  const local = players.find((player) => player.id === playerId);
  if (!local) {
    return { x: 0, y: 0 };
  }

  return {
    x: local.x,
    y: local.y,
  };
}

function directionFromInput(input: InputState | InputCommand) {
  let dx = 0;
  let dy = 0;

  if (input.right) {
    dx += 1;
  }
  if (input.left) {
    dx -= 1;
  }
  if (input.up) {
    dy += 1;
  }
  if (input.down) {
    dy -= 1;
  }

  if (dx === 0 && dy === 0) {
    return { x: 1, y: 0 };
  }

  const mag = Math.hypot(dx, dy);
  return {
    x: dx / mag,
    y: dy / mag,
  };
}

export function RoomRoute() {
  const { roomCode } = useParams({ from: '/room/$roomCode' });
  const { isLoaded, isSignedIn, user } = useUser();

  const clientPlayerId = user?.id ?? '';
  const playerLabel = useMemo(() => {
    if (!user) {
      return 'Unknown';
    }

    return displayNameForUser(user.username, user.firstName, user.id);
  }, [user]);

  const [connectionStatus, setConnectionStatus] = useState('Booting game...');
  const [activePlayers, setActivePlayers] = useState(0);
  const [serverPlayerId, setServerPlayerId] = useState('');
  const [serverTick, setServerTick] = useState(0);
  const [simRateHz, setSimRateHz] = useState(60);
  const [snapshotRateHz, setSnapshotRateHz] = useState(20);
  const [lastAckSeq, setLastAckSeq] = useState(0);
  const [latencyMs, setLatencyMs] = useState(0);
  const [structureCount, setStructureCount] = useState(0);
  const [projectileCount, setProjectileCount] = useState(0);
  const [optimisticPlacements, setOptimisticPlacements] = useState(0);

  const socketRef = useRef<RoomSocket | null>(null);
  const replicationRef = useRef(new ReplicationPipeline());
  const localPosRef = useRef({ x: 0, y: 0 });
  const lastInputRef = useRef<InputState>({
    up: false,
    down: false,
    left: false,
    right: false,
  });

  useEffect(() => {
    if (!clientPlayerId) {
      return;
    }

    replicationRef.current = new ReplicationPipeline();

    let inputPump: number | null = null;
    let renderPump: number | null = null;
    let disposed = false;

    const start = async () => {
      setConnectionStatus('Booting game...');
      await bootGame(CANVAS_ID);
      await setPlayerId(clientPlayerId);

      if (disposed) {
        return;
      }

      const socket = new RoomSocket(roomCode, clientPlayerId, {
        onStatus: (status) => {
          setConnectionStatus(status);
        },
        onWelcome: (payload) => {
          setServerPlayerId(payload.playerId);
          setSimRateHz(payload.simRateHz);
          setSnapshotRateHz(payload.snapshotRateHz);
          setConnectionStatus(`Connected to ${payload.roomCode}`);
          void setPlayerId(payload.playerId);
        },
        onSnapshot: (snapshot: RoomSnapshot) => {
          replicationRef.current.ingestSnapshot(snapshot);

          setServerTick(snapshot.serverTick);
          setSimRateHz(snapshot.simRateHz);
          setSnapshotRateHz(snapshot.snapshotRateHz);

          const presence = snapshot.features.presence;
          if (presence) {
            setActivePlayers(presence.onlineCount);
          }

          const movement = snapshot.features.movement;
          const build = snapshot.features.build;
          const projectile = snapshot.features.projectile;

          if (movement) {
            localPosRef.current = localPlayerPosition(movement.players, clientPlayerId);
          }

          if (build) {
            setStructureCount(build.structureCount);
            setOptimisticPlacements(0);
          }

          if (projectile) {
            setProjectileCount(projectile.projectileCount);
          }
        },
        onAck: (seq) => {
          setLastAckSeq((prev) => Math.max(prev, seq));
        },
        onEvent: () => {
          // Event channels are available for feature-specific UI hooks.
        },
        onPong: (latency) => {
          setLatencyMs(Math.round(latency));
        },
      });

      socket.connect();
      socketRef.current = socket;

      inputPump = window.setInterval(() => {
        void drainInputCommands().then((commands) => {
          const latest = commands[commands.length - 1];
          if (latest) {
            lastInputRef.current = {
              up: latest.up,
              down: latest.down,
              left: latest.left,
              right: latest.right,
            };
          }
          socket.sendInputCommands(commands);
        });
      }, 16);

      renderPump = window.setInterval(() => {
        const renderSnapshot = replicationRef.current.buildRenderSnapshot(clientPlayerId);
        if (!renderSnapshot) {
          return;
        }

        localPosRef.current = localPlayerPosition(renderSnapshot.players, clientPlayerId);
        void pushRenderSnapshot(renderSnapshot);
      }, 16);
    };

    void start().catch((error) => {
      console.error('Failed to start room session.', error);
      setConnectionStatus('Startup error');
    });

    return () => {
      disposed = true;
      if (inputPump !== null) {
        window.clearInterval(inputPump);
      }
      if (renderPump !== null) {
        window.clearInterval(renderPump);
      }
      socketRef.current?.disconnect();
      socketRef.current = null;
    };
  }, [clientPlayerId, roomCode]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.code !== 'Space') {
        return;
      }

      const socket = socketRef.current;
      if (!socket) {
        return;
      }

      event.preventDefault();

      const origin = localPosRef.current;
      const direction = directionFromInput(lastInputRef.current);

      socket.sendProjectileFire({
        x: origin.x,
        y: origin.y,
        vx: direction.x * PROJECTILE_SPEED,
        vy: direction.y * PROJECTILE_SPEED,
        clientProjectileId: `proj_${crypto.randomUUID()}`,
      });
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, []);

  if (!isLoaded) {
    return (
      <section className="grid h-full place-items-center bg-[#04070c] text-[#eaf1ff]">
        <p className="hud-pill">Loading player profile...</p>
      </section>
    );
  }

  if (!isSignedIn || !user) {
    return (
      <section className="grid h-full place-items-center bg-[#04070c] p-6 text-[#eaf1ff]">
        <div className="glass-panel max-w-lg rounded-3xl p-8">
          <p className="hud-pill w-fit">Authentication Required</p>
          <h1 className="mt-4 font-display text-3xl text-white">Sign in before joining rooms</h1>
          <Link
            to="/"
            className="mt-7 inline-flex h-11 items-center justify-center rounded-xl border border-[#39507a] px-5 font-semibold text-[#d5e3ff] transition hover:border-[#8eb1ff] hover:text-white"
          >
            Back To Lobby
          </Link>
        </div>
      </section>
    );
  }

  return (
    <section className="grid h-full min-h-0 grid-rows-[4rem_1fr] overflow-hidden bg-[#04070c] text-[#eaf1ff]">
      <header className="flex h-16 shrink-0 items-center justify-between border-b border-white/10 bg-[#0b1222]/90 px-4 backdrop-blur md:px-6">
        <div className="flex min-w-0 items-center gap-3">
          <Link
            to="/"
            className="inline-flex h-10 items-center rounded-lg border border-[#324565] px-3 text-xs font-semibold uppercase tracking-[0.2em] text-[#bcd2fb] transition hover:border-[#68e4bd] hover:text-white"
          >
            Exit
          </Link>
          <div className="min-w-0">
            <p className="truncate font-display text-lg text-white">Room {roomCode}</p>
            <p className="truncate text-xs text-[#9bb0d6]">{connectionStatus}</p>
          </div>
        </div>

        <div className="flex items-center gap-2 text-xs sm:gap-3">
          <button
            type="button"
            className="hud-pill transition hover:border-[#67f0c1]"
            onClick={() => {
              const socket = socketRef.current;
              if (!socket) {
                return;
              }

              const targetX = localPosRef.current.x + 48;
              const targetY = localPosRef.current.y;
              setOptimisticPlacements((count) => count + 1);
              socket.sendBuildPlace(targetX, targetY, 'beacon');
            }}
          >
            Place Beacon
          </button>
          <span className="hud-pill">Space = Shoot</span>
          <span className="hud-pill">Tick {serverTick}</span>
          <span className="hud-pill">Sim {simRateHz}Hz</span>
          <span className="hud-pill">Snap {snapshotRateHz}Hz</span>
          <span className="hud-pill">Ping {latencyMs}ms</span>
          <span className="hud-pill">Ack {lastAckSeq}</span>
          <span className="hud-pill">Online {activePlayers}</span>
          <span className="hud-pill">Structures {structureCount + optimisticPlacements}</span>
          <span className="hud-pill">Projectiles {projectileCount}</span>
          <span className="hidden rounded-md border border-white/15 bg-[#101b31] px-3 py-1.5 text-[#cfddf9] md:inline-flex">
            {playerLabel}
          </span>
          <span className="hidden rounded-md border border-white/15 bg-[#101b31] px-3 py-1.5 font-mono text-[#9dd9ff] lg:inline-flex">
            {serverPlayerId || clientPlayerId}
          </span>
        </div>
      </header>

      <div className="min-h-0 p-2 md:p-3">
        <div className="relative h-full w-full overflow-hidden rounded-2xl border border-white/10 bg-[#060c16] shadow-[0_20px_80px_rgba(9,14,24,0.6)]">
          <canvas id={CANVAS_ID} className="absolute inset-0 block h-full w-full" />
        </div>
      </div>
    </section>
  );
}

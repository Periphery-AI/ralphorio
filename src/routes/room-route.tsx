import { Link, useParams } from '@tanstack/react-router';
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useAuth, useUser } from '@clerk/clerk-react';
import {
  bootGame,
  drainFeatureCommands,
  drainInputCommands,
  pushRenderSnapshot,
  resetSessionState,
  setPlayerId,
} from '../game/bridge';
import { RoomSocket } from '../game/network-client';
import { ReplicationPipeline } from '../game/netcode/replication';
import type {
  ActiveCraftState,
  CraftQueueEntry,
  InventoryStack,
  PlayerState,
  RoomSnapshot,
} from '../game/types';

const CANVAS_ID = 'bevy-game-canvas';
const DEFAULT_INTERP_DELAY_MS = 110;
const CANVAS_STASH_ID = 'bevy-canvas-stash';

let persistentCanvas: HTMLCanvasElement | null = null;

function getCanvasStash() {
  let stash = document.getElementById(CANVAS_STASH_ID) as HTMLDivElement | null;
  if (stash) {
    return stash;
  }

  stash = document.createElement('div');
  stash.id = CANVAS_STASH_ID;
  stash.style.position = 'fixed';
  stash.style.left = '-10000px';
  stash.style.top = '-10000px';
  stash.style.width = '1px';
  stash.style.height = '1px';
  stash.style.overflow = 'hidden';
  stash.style.pointerEvents = 'none';
  stash.style.opacity = '0';
  document.body.appendChild(stash);
  return stash;
}

function getPersistentCanvas() {
  if (persistentCanvas) {
    return persistentCanvas;
  }

  const existing = document.getElementById(CANVAS_ID);
  if (existing instanceof HTMLCanvasElement) {
    persistentCanvas = existing;
  } else {
    persistentCanvas = document.createElement('canvas');
    persistentCanvas.id = CANVAS_ID;
  }

  persistentCanvas.className = 'block h-full w-full';
  return persistentCanvas;
}

function resumeTokenStorageKey(roomCode: string, playerId: string) {
  return `ralph-resume:${roomCode}:${playerId}`;
}

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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function stringField(payload: Record<string, unknown>, key: string) {
  const value = payload[key];
  return typeof value === 'string' ? value : null;
}

function numberField(payload: Record<string, unknown>, key: string) {
  const value = payload[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function formatCombatEvent(action: string, payload: unknown, localPlayerId: string) {
  const details = isRecord(payload) ? payload : {};
  const markLocal = (playerId: string | null) =>
    playerId && playerId === localPlayerId ? `${playerId} (you)` : playerId;

  switch (action) {
    case 'player_attacked': {
      const attacker = markLocal(stringField(details, 'attackerPlayerId'));
      const enemyId = stringField(details, 'targetEnemyId');
      if (attacker && enemyId) {
        return `${attacker} attacked ${enemyId}`;
      }
      return 'attack fired';
    }
    case 'enemy_damaged': {
      const attacker = markLocal(stringField(details, 'attackerPlayerId'));
      const enemyId = stringField(details, 'enemyId');
      const remaining = numberField(details, 'remainingHealth');
      if (attacker && enemyId && remaining !== null) {
        return `${attacker} hit ${enemyId} (${remaining} hp)`;
      }
      return 'enemy damaged';
    }
    case 'enemy_defeated': {
      const enemyKind = stringField(details, 'enemyKind') ?? 'enemy';
      const byPlayerId = markLocal(stringField(details, 'byPlayerId'));
      if (byPlayerId) {
        return `${enemyKind} defeated by ${byPlayerId}`;
      }
      return `${enemyKind} defeated`;
    }
    case 'player_damaged': {
      const playerId = markLocal(stringField(details, 'playerId'));
      const damage = numberField(details, 'damage');
      const remaining = numberField(details, 'remainingHealth');
      if (playerId && damage !== null && remaining !== null) {
        return `${playerId} took ${damage} damage (${remaining} hp)`;
      }
      return 'player damaged';
    }
    case 'player_defeated': {
      const playerId = markLocal(stringField(details, 'playerId'));
      if (playerId) {
        return `${playerId} was defeated`;
      }
      return 'player defeated';
    }
    default:
      return `combat.${action}`;
  }
}

function MetricPill({ label, value }: { label: string; value: string | number }) {
  return (
    <span className="hud-pill hud-metric">
      <span className="hud-metric-label">{label}</span>
      <span className="hud-metric-value">{value}</span>
    </span>
  );
}

type InventoryPanelState = {
  maxSlots: number;
  stacks: InventoryStack[];
};

type CraftingPanelState = {
  active: ActiveCraftState | null;
  pending: CraftQueueEntry[];
};

type BuildPanelState = {
  kind: string;
  x: number;
  y: number;
  canPlace: boolean;
  reason: string | null;
};

const EMPTY_INVENTORY_PANEL: InventoryPanelState = {
  maxSlots: 0,
  stacks: [],
};

const EMPTY_CRAFTING_PANEL: CraftingPanelState = {
  active: null,
  pending: [],
};

function titleCaseToken(token: string) {
  return token
    .split(/[_-]+/)
    .filter((segment) => segment.length > 0)
    .map((segment) => segment[0].toUpperCase() + segment.slice(1))
    .join(' ');
}

function OverlayPanel({
  eyebrow,
  title,
  children,
}: {
  eyebrow: string;
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="glass-panel pointer-events-auto min-w-[14.75rem] flex-1 rounded-xl border border-[#2e4f80]/80 bg-[#071120]/90 p-3 md:min-w-0">
      <p className="text-[10px] uppercase tracking-[0.16em] text-[#89a8df]">{eyebrow}</p>
      <p className="mt-1 font-display text-sm text-[#f2f7ff]">{title}</p>
      <div className="mt-2">{children}</div>
    </section>
  );
}

export function RoomRoute() {
  const { roomCode } = useParams({ from: '/room/$roomCode' });
  const { isLoaded, isSignedIn, user } = useUser();
  const { getToken } = useAuth();

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
  const [inventoryStackCount, setInventoryStackCount] = useState(0);
  const [inventoryItemCount, setInventoryItemCount] = useState(0);
  const [miningNodeCount, setMiningNodeCount] = useState(0);
  const [miningActiveCount, setMiningActiveCount] = useState(0);
  const [dropCount, setDropCount] = useState(0);
  const [showDevConsole, setShowDevConsole] = useState(false);
  const [devInput, setDevInput] = useState('');
  const [devLog, setDevLog] = useState<string[]>([]);
  const [combatFeed, setCombatFeed] = useState<string[]>([]);
  const [interpDelayMs, setInterpDelayMs] = useState(DEFAULT_INTERP_DELAY_MS);
  const [inventoryPanel, setInventoryPanel] = useState<InventoryPanelState>(EMPTY_INVENTORY_PANEL);
  const [craftingPanel, setCraftingPanel] = useState<CraftingPanelState>(EMPTY_CRAFTING_PANEL);
  const [selectedBuildPanel, setSelectedBuildPanel] = useState<BuildPanelState | null>(null);
  const [buildPreviewCount, setBuildPreviewCount] = useState(0);

  const socketRef = useRef<RoomSocket | null>(null);
  const replicationRef = useRef(new ReplicationPipeline());
  const localPosRef = useRef({ x: 0, y: 0 });
  const devInputRef = useRef<HTMLInputElement | null>(null);
  const canvasHostRef = useRef<HTMLDivElement | null>(null);
  const interpDelayRef = useRef(DEFAULT_INTERP_DELAY_MS);
  const authoritativePlayerIdRef = useRef(clientPlayerId);

  const pushDevLog = (line: string) => {
    setDevLog((existing) => [...existing.slice(-11), line]);
  };

  const pushCombatFeed = (line: string) => {
    setCombatFeed((existing) => [...existing.slice(-7), line]);
  };

  useEffect(() => {
    const host = canvasHostRef.current;
    if (!host) {
      return;
    }

    const canvas = getPersistentCanvas();
    host.appendChild(canvas);

    return () => {
      if (canvas.parentElement === host) {
        getCanvasStash().appendChild(canvas);
      }
    };
  }, [roomCode]);

  useEffect(() => {
    if (!clientPlayerId) {
      return;
    }

    authoritativePlayerIdRef.current = clientPlayerId;
    replicationRef.current = new ReplicationPipeline(interpDelayRef.current);
    setCombatFeed([]);
    setInventoryPanel(EMPTY_INVENTORY_PANEL);
    setCraftingPanel(EMPTY_CRAFTING_PANEL);
    setSelectedBuildPanel(null);
    setBuildPreviewCount(0);

    let inputPump: number | null = null;
    let renderPump: number | null = null;
    let disposed = false;

    const start = async () => {
      setConnectionStatus('Booting game...');
      await bootGame(CANVAS_ID);
      await resetSessionState();
      await setPlayerId(clientPlayerId);
      const clerkToken = await getToken();
      const storedResumeToken = window.localStorage.getItem(
        resumeTokenStorageKey(roomCode, clientPlayerId),
      );

      if (disposed) {
        return;
      }

      const socket = new RoomSocket(
        roomCode,
        clientPlayerId,
        {
          onStatus: (status) => {
            setConnectionStatus(status);
            if (status.startsWith('Error:')) {
              pushDevLog(status);
            }
          },
          onWelcome: (payload) => {
            setServerPlayerId(payload.playerId);
            authoritativePlayerIdRef.current = payload.playerId;
            setSimRateHz(payload.simRateHz);
            setSnapshotRateHz(payload.snapshotRateHz);
            setConnectionStatus(`Connected to ${payload.roomCode}`);
            void setPlayerId(payload.playerId);

            if (payload.resumeToken) {
              window.localStorage.setItem(
                resumeTokenStorageKey(roomCode, clientPlayerId),
                payload.resumeToken,
              );
            }
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
            const inventory = snapshot.features.inventory;
            const mining = snapshot.features.mining;
            const drops = snapshot.features.drops;
            const crafting = snapshot.features.crafting;

            if (movement) {
              localPosRef.current = localPlayerPosition(movement.players, clientPlayerId);
            }

            if (build) {
              setStructureCount(build.structureCount);
              setBuildPreviewCount(build.previewCount);

              const localBuildPreview =
                build.previews.find((preview) => preview.playerId === authoritativePlayerIdRef.current) ??
                null;
              setSelectedBuildPanel(
                localBuildPreview
                  ? {
                      kind: localBuildPreview.kind,
                      x: Math.round(localBuildPreview.x),
                      y: Math.round(localBuildPreview.y),
                      canPlace: localBuildPreview.canPlace,
                      reason: localBuildPreview.reason,
                    }
                  : null,
              );
            }

            if (projectile) {
              setProjectileCount(projectile.projectileCount);
            }

            if (inventory) {
              const localInventory =
                inventory.players.find(
                  (player) => player.playerId === authoritativePlayerIdRef.current,
                ) ?? null;

              if (!localInventory) {
                setInventoryStackCount(0);
                setInventoryItemCount(0);
                setInventoryPanel(EMPTY_INVENTORY_PANEL);
              } else {
                setInventoryStackCount(localInventory.stacks.length);
                setInventoryItemCount(
                  localInventory.stacks.reduce((total, stack) => total + stack.amount, 0),
                );
                setInventoryPanel({
                  maxSlots: localInventory.maxSlots,
                  stacks: localInventory.stacks.map((stack) => ({ ...stack })),
                });
              }
            }

            if (mining) {
              setMiningNodeCount(mining.nodeCount);
              setMiningActiveCount(mining.activeCount);
            }

            if (drops) {
              setDropCount(drops.dropCount);
            }

            if (crafting) {
              const localCraftQueue =
                crafting.queues.find((queue) => queue.playerId === authoritativePlayerIdRef.current) ??
                null;

              if (!localCraftQueue) {
                setCraftingPanel(EMPTY_CRAFTING_PANEL);
              } else {
                setCraftingPanel({
                  active: localCraftQueue.active ? { ...localCraftQueue.active } : null,
                  pending: localCraftQueue.pending.map((entry) => ({ ...entry })),
                });
              }
            }
          },
          onAck: (seq) => {
            setLastAckSeq((prev) => Math.max(prev, seq));
          },
          onEvent: (feature, action, payload) => {
            if (feature !== 'combat') {
              return;
            }

            pushCombatFeed(formatCombatEvent(action, payload, authoritativePlayerIdRef.current));
          },
          onPong: (latency) => {
            setLatencyMs(Math.round(latency));
          },
        },
        clerkToken ?? null,
        storedResumeToken,
      );

      await socket.connect();
      socketRef.current = socket;

      inputPump = window.setInterval(() => {
        void drainInputCommands().then((commands) => {
          socket.sendInputCommands(commands);
        });

        void drainFeatureCommands().then((featureCommands) => {
          for (const command of featureCommands) {
            socket.sendFeatureCommand(command.feature, command.action, command.payload);
          }
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
  }, [clientPlayerId, roomCode, getToken]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.code === 'Backquote') {
        event.preventDefault();
        setShowDevConsole((enabled) => !enabled);
        return;
      }

      if (event.code === 'Escape') {
        setShowDevConsole(false);
      }
    };

    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
    };
  }, []);

  useEffect(() => {
    if (!showDevConsole) {
      return;
    }

    window.setTimeout(() => {
      devInputRef.current?.focus();
    }, 0);
  }, [showDevConsole]);

  const runDevCommand = (commandRaw: string) => {
    const command = commandRaw.trim();
    if (!command) {
      return;
    }

    pushDevLog(`> ${command}`);
    const [name, arg] = command.split(/\s+/, 2);

    if (name === 'help') {
      pushDevLog('help | clear | net.stats | net.interp [ms]');
      return;
    }

    if (name === 'clear') {
      setDevLog([]);
      return;
    }

    if (name === 'net.stats') {
      const debug = replicationRef.current.getDebugInfo();
      pushDevLog(
        `tick=${serverTick} ping=${latencyMs}ms ack=${lastAckSeq} online=${activePlayers} proj=${projectileCount} interp=${Math.round(debug.interpolationDelayMs)}ms buf=${debug.bufferedSnapshots} offset=${Math.round(debug.clockOffsetMs)}ms`,
      );
      return;
    }

    if (name === 'net.interp') {
      if (!arg) {
        pushDevLog(`interp=${Math.round(replicationRef.current.getInterpolationDelayMs())}ms`);
        return;
      }

      const parsed = Number(arg);
      if (!Number.isFinite(parsed)) {
        pushDevLog('invalid value, expected number in ms');
        return;
      }

      replicationRef.current.setInterpolationDelayMs(parsed);
      const current = replicationRef.current.getInterpolationDelayMs();
      interpDelayRef.current = current;
      setInterpDelayMs(current);
      pushDevLog(`interp set to ${Math.round(current)}ms`);
      return;
    }

    pushDevLog('unknown command');
  };

  const sortedInventoryStacks = useMemo(() => {
    return [...inventoryPanel.stacks].sort((left, right) => left.slot - right.slot);
  }, [inventoryPanel.stacks]);

  const inventorySlotsLabel =
    inventoryPanel.maxSlots > 0
      ? `${inventoryPanel.stacks.length}/${inventoryPanel.maxSlots}`
      : `${inventoryPanel.stacks.length}`;

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

        <div className="flex min-w-0 items-center gap-2 overflow-x-auto py-1 text-xs sm:gap-3">
          <span className="hud-pill">Q = Build</span>
          <span className="hud-pill">Hold Click = Mine</span>
          <span className="hud-pill">Click = Place (Build Mode)</span>
          <span className="hud-pill">E = Pickup</span>
          <span className="hud-pill">Space = Shoot</span>
          <MetricPill label="Tick" value={serverTick} />
          <MetricPill label="Sim" value={`${simRateHz}Hz`} />
          <MetricPill label="Snap" value={`${snapshotRateHz}Hz`} />
          <MetricPill label="Ping" value={`${latencyMs}ms`} />
          <MetricPill label="Interp" value={`${Math.round(interpDelayMs)}ms`} />
          <MetricPill label="Ack" value={lastAckSeq} />
          <MetricPill label="Online" value={activePlayers} />
          <MetricPill label="Inv" value={inventoryItemCount} />
          <MetricPill label="Stacks" value={inventoryStackCount} />
          <MetricPill label="Nodes" value={miningNodeCount} />
          <MetricPill label="Mining" value={miningActiveCount} />
          <MetricPill label="Drops" value={dropCount} />
          <MetricPill label="Structures" value={structureCount} />
          <MetricPill label="Projectiles" value={projectileCount} />
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
          <div ref={canvasHostRef} className="absolute inset-0" />
          {combatFeed.length > 0 ? (
            <div className="absolute left-3 top-3 z-20 max-w-sm rounded-lg border border-[#37527f] bg-[#081325]/86 px-3 py-2 backdrop-blur">
              <p className="mb-1 text-[10px] uppercase tracking-[0.14em] text-[#93b4ee]">Combat Feed</p>
              <div className="space-y-1 text-xs text-[#d8e6ff]">
                {combatFeed.map((line, index) => (
                  <p key={`${line}-${index}`}>{line}</p>
                ))}
              </div>
            </div>
          ) : null}
          {!showDevConsole ? (
            <div className="pointer-events-none absolute inset-x-3 bottom-3 z-20">
              <div className="flex gap-2 overflow-x-auto pb-1 md:grid md:grid-cols-3 md:overflow-visible md:pb-0">
                <OverlayPanel
                  eyebrow="Authoritative"
                  title={`Inventory ${inventoryItemCount} items`}
                >
                  <div className="grid grid-cols-2 gap-2">
                    <div className="rounded-md border border-[#2f4976] bg-[#0a172b]/88 px-2 py-1.5">
                      <p className="text-[10px] uppercase tracking-[0.14em] text-[#88a7de]">Stacks</p>
                      <p className="text-sm font-semibold text-[#dce9ff]">{inventorySlotsLabel}</p>
                    </div>
                    <div className="rounded-md border border-[#2f4976] bg-[#0a172b]/88 px-2 py-1.5">
                      <p className="text-[10px] uppercase tracking-[0.14em] text-[#88a7de]">Total</p>
                      <p className="text-sm font-semibold text-[#dce9ff]">{inventoryItemCount}</p>
                    </div>
                  </div>
                  <div className="mt-2 max-h-32 overflow-y-auto rounded-md border border-[#273f69] bg-[#081326]/86 p-2 text-xs text-[#d4e3ff]">
                    {sortedInventoryStacks.length === 0 ? (
                      <p className="text-[#8ea8d6]">No resources in inventory.</p>
                    ) : (
                      <div className="space-y-1">
                        {sortedInventoryStacks.map((stack) => (
                          <div
                            key={`${stack.slot}-${stack.resource}`}
                            className="flex items-center justify-between rounded border border-[#2a3f66] bg-[#0b162a]/84 px-2 py-1"
                          >
                            <span>
                              {stack.slot + 1}. {titleCaseToken(stack.resource)}
                            </span>
                            <span className="font-mono text-[#9ce7cf]">{stack.amount}</span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </OverlayPanel>

                <OverlayPanel eyebrow="Authoritative" title="Crafting Queue">
                  <div className="rounded-md border border-[#2f4976] bg-[#0a172b]/88 px-2 py-1.5 text-xs">
                    <p className="text-[10px] uppercase tracking-[0.14em] text-[#88a7de]">Active Craft</p>
                    {craftingPanel.active ? (
                      <p className="mt-1 text-[#dce9ff]">
                        {titleCaseToken(craftingPanel.active.recipe)}{' '}
                        <span className="font-mono text-[#9ce7cf]">
                          ({craftingPanel.active.remainingTicks} ticks)
                        </span>
                      </p>
                    ) : (
                      <p className="mt-1 text-[#8ea8d6]">Idle</p>
                    )}
                  </div>
                  <div className="mt-2 max-h-32 overflow-y-auto rounded-md border border-[#273f69] bg-[#081326]/86 p-2 text-xs text-[#d4e3ff]">
                    {craftingPanel.pending.length === 0 ? (
                      <p className="text-[#8ea8d6]">No queued crafts.</p>
                    ) : (
                      <div className="space-y-1">
                        {craftingPanel.pending.map((entry, index) => (
                          <div
                            key={`${entry.recipe}-${index}`}
                            className="flex items-center justify-between rounded border border-[#2a3f66] bg-[#0b162a]/84 px-2 py-1"
                          >
                            <span>{titleCaseToken(entry.recipe)}</span>
                            <span className="font-mono text-[#9ce7cf]">x{entry.count}</span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </OverlayPanel>

                <OverlayPanel eyebrow="Authoritative" title="Selected Build">
                  <div className="grid grid-cols-2 gap-2">
                    <div className="rounded-md border border-[#2f4976] bg-[#0a172b]/88 px-2 py-1.5">
                      <p className="text-[10px] uppercase tracking-[0.14em] text-[#88a7de]">Placed</p>
                      <p className="text-sm font-semibold text-[#dce9ff]">{structureCount}</p>
                    </div>
                    <div className="rounded-md border border-[#2f4976] bg-[#0a172b]/88 px-2 py-1.5">
                      <p className="text-[10px] uppercase tracking-[0.14em] text-[#88a7de]">Previews</p>
                      <p className="text-sm font-semibold text-[#dce9ff]">{buildPreviewCount}</p>
                    </div>
                  </div>
                  <div className="mt-2 rounded-md border border-[#273f69] bg-[#081326]/86 px-2 py-2 text-xs">
                    {selectedBuildPanel ? (
                      <>
                        <p className="text-[#dce9ff]">{titleCaseToken(selectedBuildPanel.kind)}</p>
                        <p className="mt-1 font-mono text-[#9ce7cf]">
                          x={selectedBuildPanel.x} y={selectedBuildPanel.y}
                        </p>
                        <p
                          className={`mt-1 ${
                            selectedBuildPanel.canPlace ? 'text-[#8be3bf]' : 'text-[#ff9d90]'
                          }`}
                        >
                          {selectedBuildPanel.canPlace ? 'Placement valid' : 'Placement blocked'}
                        </p>
                        {!selectedBuildPanel.canPlace && selectedBuildPanel.reason ? (
                          <p className="mt-1 text-[#ffb7ad]">{selectedBuildPanel.reason}</p>
                        ) : null}
                      </>
                    ) : (
                      <p className="text-[#8ea8d6]">No active build preview. Press Q to enter build mode.</p>
                    )}
                  </div>
                </OverlayPanel>
              </div>
            </div>
          ) : null}
          {showDevConsole ? (
            <div className="absolute inset-x-3 bottom-3 z-20 rounded-xl border border-[#6de7c0]/60 bg-[#071520]/88 p-3 backdrop-blur">
              <div className="mb-2 flex items-center justify-between text-[11px] uppercase tracking-[0.16em] text-[#b8ffe8]">
                <span>Net Dev Console</span>
                <span className="text-[#8dcfb9]">` to toggle</span>
              </div>
              <div className="max-h-32 overflow-auto rounded-md border border-[#2a5a50] bg-[#041017] p-2 font-mono text-xs text-[#9fe6d4]">
                {devLog.length === 0 ? <p>type `help`</p> : null}
                {devLog.map((line, index) => (
                  <p key={`${line}-${index}`}>{line}</p>
                ))}
              </div>
              <form
                className="mt-2"
                onSubmit={(event) => {
                  event.preventDefault();
                  const command = devInput.trim();
                  if (!command) {
                    return;
                  }

                  runDevCommand(command);
                  setDevInput('');
                }}
              >
                <input
                  ref={devInputRef}
                  value={devInput}
                  onChange={(event) => setDevInput(event.target.value)}
                  className="w-full rounded-md border border-[#3a7b6e] bg-[#031018] px-3 py-2 font-mono text-xs text-[#c7fff0] outline-none focus:border-[#74ffd4]"
                  placeholder="help | net.stats | net.interp 80"
                />
              </form>
            </div>
          ) : null}
        </div>
      </div>
    </section>
  );
}

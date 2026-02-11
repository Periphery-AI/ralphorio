import { Link, useParams } from '@tanstack/react-router';
import { useEffect, useMemo, useState } from 'react';
import { bootGame, drainMoveEvents, pushSnapshot, setPlayerId } from '../game/bridge';
import { RoomSocket } from '../game/network-client';
import type { PlayerState } from '../game/types';
import { getOrCreatePlayerId } from '../lib/player-id';

const CANVAS_ID = 'bevy-game-canvas';

function connectedCount(players: PlayerState[]) {
  return players.filter((player) => player.connected).length;
}

export function RoomRoute() {
  const { roomCode } = useParams({ from: '/room/$roomCode' });
  const clientPlayerId = useMemo(() => getOrCreatePlayerId(), []);

  const [connectionStatus, setConnectionStatus] = useState('Booting game...');
  const [activePlayers, setActivePlayers] = useState(0);
  const [serverPlayerId, setServerPlayerId] = useState(clientPlayerId);

  useEffect(() => {
    let socket: RoomSocket | null = null;
    let movePump: number | null = null;
    let disposed = false;

    const start = async () => {
      await bootGame(CANVAS_ID);
      await setPlayerId(clientPlayerId);

      if (disposed) {
        return;
      }

      socket = new RoomSocket(roomCode, clientPlayerId, {
        onStatus: (status) => {
          setConnectionStatus(status);
        },
        onWelcome: (message) => {
          setServerPlayerId(message.playerId);
          setConnectionStatus(`Connected to ${message.roomCode}`);
          setActivePlayers(connectedCount(message.players));

          void setPlayerId(message.playerId);
          void pushSnapshot(message.players);
        },
        onSnapshot: (players) => {
          setActivePlayers(connectedCount(players));
          void pushSnapshot(players);
        },
      });

      socket.connect();

      movePump = window.setInterval(() => {
        void drainMoveEvents().then((moves) => {
          socket?.sendMoves(moves);
        });
      }, 33);
    };

    void start().catch((error) => {
      console.error('Failed to start room session.', error);
      setConnectionStatus('Startup error');
    });

    return () => {
      disposed = true;
      if (movePump !== null) {
        window.clearInterval(movePump);
      }
      socket?.disconnect();
    };
  }, [clientPlayerId, roomCode]);

  return (
    <section className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="font-mono text-2xl font-bold text-cyan-200">Room {roomCode}</h1>
          <p className="mt-1 text-sm text-slate-300">{connectionStatus}</p>
        </div>

        <div className="flex items-center gap-3 text-sm text-slate-300">
          <span className="rounded-lg border border-slate-700 bg-slate-900 px-3 py-2">
            Player: <strong className="text-cyan-300">{serverPlayerId}</strong>
          </span>
          <span className="rounded-lg border border-slate-700 bg-slate-900 px-3 py-2">
            Online: <strong className="text-cyan-300">{activePlayers}</strong>
          </span>
          <Link
            to="/"
            className="rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 font-semibold text-slate-200 transition hover:border-cyan-400"
          >
            Leave
          </Link>
        </div>
      </div>

      <div className="overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl shadow-cyan-950/20">
        <div className="relative h-[70vh] min-h-[420px] w-full">
          <canvas id={CANVAS_ID} className="absolute inset-0 h-full w-full" />
        </div>
      </div>

      <p className="text-sm text-slate-400">
        Controls: <span className="font-semibold text-slate-200">WASD</span> or{' '}
        <span className="font-semibold text-slate-200">Arrow Keys</span>.
      </p>
    </section>
  );
}

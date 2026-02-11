import type {
  MoveMessage,
  PlayerState,
  ServerMessage,
  WelcomeMessage,
} from './types';

type Handlers = {
  onWelcome: (message: WelcomeMessage) => void;
  onSnapshot: (players: PlayerState[]) => void;
  onStatus: (status: string) => void;
};

function buildWebSocketUrl(roomCode: string, playerId: string) {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const host = window.location.host;
  const encodedRoom = encodeURIComponent(roomCode);
  const encodedPlayer = encodeURIComponent(playerId);
  return `${protocol}//${host}/api/rooms/${encodedRoom}/ws?playerId=${encodedPlayer}`;
}

export class RoomSocket {
  private socket: WebSocket | null = null;
  private readonly roomCode: string;
  private readonly playerId: string;
  private readonly handlers: Handlers;

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
    });

    this.socket.addEventListener('message', (event) => {
      if (typeof event.data !== 'string') {
        return;
      }

      let message: ServerMessage;
      try {
        message = JSON.parse(event.data) as ServerMessage;
      } catch {
        return;
      }

      if (message.type === 'welcome') {
        this.handlers.onWelcome(message);
        return;
      }

      if (message.type === 'snapshot') {
        this.handlers.onSnapshot(message.players);
      }
    });

    this.socket.addEventListener('close', () => {
      this.handlers.onStatus('Disconnected');
    });

    this.socket.addEventListener('error', () => {
      this.handlers.onStatus('Connection error');
    });
  }

  sendMoves(moves: { x: number; y: number }[]) {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) {
      return;
    }

    const latestMove = moves[moves.length - 1];
    if (!latestMove) {
      return;
    }

    const message: MoveMessage = {
      type: 'move',
      x: latestMove.x,
      y: latestMove.y,
    };

    this.socket.send(JSON.stringify(message));
  }

  disconnect() {
    if (this.socket) {
      this.socket.close();
      this.socket = null;
    }
  }
}

export type PlayerState = {
  id: string;
  x: number;
  y: number;
  connected: boolean;
};

export type WelcomeMessage = {
  type: 'welcome';
  roomCode: string;
  playerId: string;
  players: PlayerState[];
};

export type SnapshotMessage = {
  type: 'snapshot';
  players: PlayerState[];
};

export type ServerMessage = WelcomeMessage | SnapshotMessage;

export type MoveEvent = {
  x: number;
  y: number;
};

export type MoveMessage = {
  type: 'move';
  x: number;
  y: number;
};

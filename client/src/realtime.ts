// The live channel for an open project: the server's fold after every
// append, who else is here and where they are. Writes do not go here —
// they stay on POST /ops (see opQueue.ts). Reconnects with backoff.

import type { Edit } from './types';

export interface PeerInfo {
  id: string;
  displayName: string;
  color: string;
}

export interface PresenceState {
  playhead: number;
  selection: [number, number] | null;
  caret: number | null;
  playing: boolean;
}

export interface Peer {
  connId: string;
  user: PeerInfo;
  state: PresenceState;
}

export type ServerMsg =
  | { t: 'hello'; headSeq: number; edits: Edit[]; speakerNames: string[]; peers: Peer[]; you: Peer }
  | {
      t: 'doc';
      seq: number;
      authorId: string;
      headSeq: number;
      edits: Edit[];
      speakerNames: string[];
    }
  | { t: 'presence'; connId: string; user: PeerInfo; state: PresenceState }
  | { t: 'left'; connId: string }
  | { t: 'resync'; headSeq: number; edits: Edit[]; speakerNames: string[] }
  | { t: 'pong' }
  | { t: 'error'; code: string; detail: string };

export type ConnectionStatus = 'connecting' | 'open' | 'reconnecting' | 'closed';

export interface RealtimeHandlers {
  onMessage(msg: ServerMsg): void;
  onStatus(status: ConnectionStatus): void;
}

export interface Realtime {
  sendPresence(state: PresenceState): void;
  close(): void;
}

interface Options {
  WebSocketImpl?: typeof WebSocket;
  baseDelayMs?: number;
  maxDelayMs?: number;
  random?: () => number;
}

/** Presence frames are coalesced to this interval. */
const PRESENCE_EVERY_MS = 100;

export function wsUrl(projectId: string, loc?: { protocol: string; host: string }): string {
  // `typeof` never throws on an undeclared identifier, unlike a bare
  // reference — this keeps the module usable outside a browser (tests, SSR).
  const here =
    loc ?? (typeof location === 'undefined' ? { protocol: 'http:', host: '' } : location);
  const scheme = here.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${here.host}/api/projects/${projectId}/ws`;
}

export function presenceEquals(a: PresenceState, b: PresenceState): boolean {
  return (
    a.playhead === b.playhead &&
    a.playing === b.playing &&
    a.caret === b.caret &&
    (a.selection === b.selection ||
      (a.selection !== null &&
        b.selection !== null &&
        a.selection[0] === b.selection[0] &&
        a.selection[1] === b.selection[1]))
  );
}

export function connectRealtime(
  projectId: string,
  handlers: RealtimeHandlers,
  opts: Options = {},
): Realtime {
  const Impl = opts.WebSocketImpl ?? WebSocket;
  const baseDelayMs = opts.baseDelayMs ?? 500;
  const maxDelayMs = opts.maxDelayMs ?? 10_000;
  const random = opts.random ?? Math.random;

  let socket: WebSocket | null = null;
  let attempt = 0;
  let closedByUs = false;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  // Presence: remember what we last sent, and what is waiting to go.
  let lastSent: PresenceState | null = null;
  let queued: PresenceState | null = null;
  let presenceTimer: ReturnType<typeof setTimeout> | null = null;

  const flushPresence = () => {
    presenceTimer = null;
    if (!queued || !socket || socket.readyState !== Impl.OPEN) return;
    if (lastSent && presenceEquals(lastSent, queued)) {
      queued = null;
      return;
    }
    socket.send(JSON.stringify({ t: 'presence', ...queued }));
    lastSent = queued;
    queued = null;
  };

  const scheduleFlush = () => {
    if (presenceTimer) return;
    presenceTimer = setTimeout(flushPresence, PRESENCE_EVERY_MS);
  };

  const open = () => {
    handlers.onStatus(attempt === 0 ? 'connecting' : 'reconnecting');
    const s = new Impl(wsUrl(projectId));
    socket = s;
    s.onopen = () => {
      attempt = 0;
      lastSent = null; // a fresh socket knows nothing about us yet
      handlers.onStatus('open');
      if (queued) flushPresence();
    };
    s.onmessage = (e: MessageEvent<string>) => {
      try {
        handlers.onMessage(JSON.parse(e.data) as ServerMsg);
      } catch {
        // A frame we cannot parse is not worth a reconnect.
      }
    };
    s.onerror = () => {
      // onclose follows; nothing to do here.
    };
    s.onclose = () => {
      if (socket !== s) return;
      socket = null;
      if (closedByUs) return;
      const delay = Math.min(maxDelayMs, baseDelayMs * 2 ** attempt) * (0.5 + random());
      attempt += 1;
      handlers.onStatus('reconnecting');
      reconnectTimer = setTimeout(open, delay);
    };
  };

  open();

  return {
    sendPresence(state) {
      queued = state;
      if (!socket || socket.readyState !== Impl.OPEN) return;
      scheduleFlush();
    },
    close() {
      closedByUs = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      if (presenceTimer) clearTimeout(presenceTimer);
      presenceTimer = null;
      const s = socket;
      socket = null;
      s?.close();
      handlers.onStatus('closed');
    },
  };
}

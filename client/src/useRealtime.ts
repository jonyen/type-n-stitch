// Ties the realtime socket to React: peers, connection status, and the
// server's fold whenever someone (including us) appends.

import { useCallback, useEffect, useRef, useState } from 'react';

import {
  connectRealtime,
  type ConnectionStatus,
  type Peer,
  type PresenceState,
  type Realtime,
  type ServerMsg,
} from './realtime';
import type { Edit } from './types';

export interface RemoteDoc {
  headSeq: number;
  edits: Edit[];
  speakerNames: string[];
}

/**
 * Decide what to do with a fold that arrived over the socket while our own
 * edits may still be in flight.
 *
 * `remote` replaces the whole edit list, so a peer's fold landing between our
 * optimistic cut and the reply to our POST would put the word we just cut
 * back on screen, only for the reply to cut it again — a visible flicker.
 * While the queue has anything pending we therefore hold the fold (keeping
 * only the newest by `headSeq`) and apply it once the queue drains; the
 * reducer's `headSeq >` rule drops it if our own reply already covered it.
 */
export function holdOrApply(
  incoming: RemoteDoc,
  held: RemoteDoc | null,
  pending: number,
): { apply: RemoteDoc | null; held: RemoteDoc | null } {
  if (pending === 0) return { apply: incoming, held: null };
  if (held && held.headSeq >= incoming.headSeq) return { apply: null, held };
  return { apply: null, held: incoming };
}

export function useRealtime(
  projectId: string | null,
  onDoc: (doc: RemoteDoc) => void,
  onOpen: () => void,
) {
  const [peers, setPeers] = useState<Peer[]>([]);
  const [me, setMe] = useState<Peer | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('closed');
  const [lastError, setLastError] = useState<string | null>(null);
  const rt = useRef<Realtime | null>(null);

  // Latest callbacks without reconnecting when they change identity. Written
  // in an effect, since a render-phase ref write is not concurrent-safe.
  const onDocRef = useRef(onDoc);
  const onOpenRef = useRef(onOpen);
  useEffect(() => {
    onDocRef.current = onDoc;
    onOpenRef.current = onOpen;
  }, [onDoc, onOpen]);

  useEffect(() => {
    if (!projectId) return;
    let myConn: string | null = null;
    const handle = (msg: ServerMsg) => {
      switch (msg.t) {
        case 'hello':
          myConn = msg.you.connId;
          setLastError(null);
          setMe(msg.you);
          setPeers(msg.peers.filter((p) => p.connId !== msg.you.connId));
          onDocRef.current(msg);
          break;
        case 'doc':
        case 'resync':
          onDocRef.current(msg);
          break;
        case 'presence':
          if (msg.connId === myConn) break;
          setPeers((list) => {
            const peer: Peer = { connId: msg.connId, user: msg.user, state: msg.state };
            const i = list.findIndex((p) => p.connId === msg.connId);
            if (i === -1) return [...list, peer];
            const next = [...list];
            next[i] = peer;
            return next;
          });
          break;
        case 'left':
          setPeers((list) => list.filter((p) => p.connId !== msg.connId));
          break;
        case 'error':
          console.error('realtime:', msg.code, msg.detail);
          setLastError(`${msg.code}: ${msg.detail}`);
          break;
        case 'pong':
          break;
      }
    };
    const conn = connectRealtime(projectId, {
      onMessage: handle,
      onStatus: (s) => {
        setStatus(s);
        if (s === 'open') onOpenRef.current();
        if (s !== 'open') setPeers([]);
      },
    });
    rt.current = conn;
    return () => {
      conn.close();
      rt.current = null;
      setPeers([]);
      setMe(null);
      setLastError(null);
    };
  }, [projectId]);

  const sendPresence = useCallback((state: PresenceState) => {
    rt.current?.sendPresence(state);
  }, []);

  return { peers, me, status, lastError, sendPresence };
}

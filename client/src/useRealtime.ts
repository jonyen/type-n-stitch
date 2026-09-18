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

export function useRealtime(
  projectId: string | null,
  onDoc: (doc: RemoteDoc) => void,
  onOpen: () => void,
) {
  const [peers, setPeers] = useState<Peer[]>([]);
  const [me, setMe] = useState<Peer | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('closed');
  const rt = useRef<Realtime | null>(null);

  // Latest callbacks without reconnecting when they change identity.
  const onDocRef = useRef(onDoc);
  const onOpenRef = useRef(onOpen);
  onDocRef.current = onDoc;
  onOpenRef.current = onOpen;

  useEffect(() => {
    if (!projectId) return;
    let myConn: string | null = null;
    const handle = (msg: ServerMsg) => {
      switch (msg.t) {
        case 'hello':
          myConn = msg.you.connId;
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
    };
  }, [projectId]);

  const sendPresence = useCallback((state: PresenceState) => {
    rt.current?.sendPresence(state);
  }, []);

  return { peers, me, status, sendPresence };
}

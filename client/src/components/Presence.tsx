import type { ConnectionStatus, Peer } from '../realtime';
import { Avatar } from './Avatar';

interface Props {
  peers: Peer[];
  status: ConnectionStatus;
  /** The last `error` frame the server sent, if any; cleared on reconnect. */
  lastError?: string | null;
}

const LABEL: Record<ConnectionStatus, string> = {
  open: 'live',
  connecting: 'connecting…',
  reconnecting: 'reconnecting…',
  closed: 'offline',
};

/** Who else has this project open, and whether we are hearing from the server. */
export function Presence({ peers, status, lastError }: Props) {
  const shown = peers.slice(0, 5);
  const extra = peers.length - shown.length;
  return (
    <span
      className="presence"
      aria-label={`${peers.length} other ${peers.length === 1 ? 'person' : 'people'} here`}
    >
      <span className={`status ${status}`}>
        <span className="dot" aria-hidden />
        {LABEL[status]}
      </span>
      {lastError && (
        <span className="status error" title={lastError}>
          <span className="dot" aria-hidden />
          error
        </span>
      )}
      <span className="peer-avatars">
        {shown.map((p) => (
          <Avatar key={p.connId} user={p.user} size="sm" />
        ))}
        {extra > 0 && <span className="avatar-more muted">+{extra}</span>}
      </span>
    </span>
  );
}

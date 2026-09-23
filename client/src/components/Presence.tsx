import type { ConnectionStatus, Peer } from '../realtime';
import { cx } from '../cx';
import { Avatar } from './Avatar';
import styles from './Presence.module.css';

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
      className={styles.presence}
      aria-label={`${peers.length} other ${peers.length === 1 ? 'person' : 'people'} here`}
    >
      <span
        className={cx(
          styles.status,
          status === 'open' && styles.open,
          (status === 'connecting' || status === 'reconnecting') && styles.waiting,
        )}
      >
        <span className={styles.dot} aria-hidden />
        {LABEL[status]}
      </span>
      {lastError && (
        <span className={cx(styles.status, styles.error)} title={lastError}>
          <span className={styles.dot} aria-hidden />
          error
        </span>
      )}
      <span className={styles.peers}>
        {shown.map((p) => (
          <Avatar key={p.connId} user={p.user} size="sm" ring />
        ))}
        {extra > 0 && <span className={styles.more}>+{extra}</span>}
      </span>
    </span>
  );
}

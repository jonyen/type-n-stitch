import { initials } from '../format';
import type { User } from '../types';
import { cx } from '../cx';
import styles from './Avatar.module.css';

interface Props {
  user: Pick<User, 'displayName' | 'color' | 'bot'>;
  /** Show the name beside the circle. */
  withName?: boolean;
  size?: 'sm' | 'md';
  /** A gap in the surface colour, for overlapping chips. */
  ring?: boolean;
}

/** A colored initial circle; the same chip the presence row will use later. */
export function Avatar({ user, withName = false, size = 'md', ring = false }: Props) {
  return (
    <span
      className={cx(styles.avatar, size === 'sm' && styles.sm, ring && styles.ring)}
      title={user.bot ? `${user.displayName} (agent)` : user.displayName}
    >
      <span className={styles.dot} style={{ background: user.color }} aria-hidden>
        {initials(user.displayName)}
        {user.bot && (
          <span className={styles.bot} aria-hidden>
            AI
          </span>
        )}
      </span>
      {withName && <span>{user.displayName}</span>}
    </span>
  );
}

import { initials } from '../format';
import type { User } from '../types';

interface Props {
  user: Pick<User, 'displayName' | 'color' | 'bot'>;
  /** Show the name beside the circle. */
  withName?: boolean;
  size?: 'sm' | 'md';
}

/** A colored initial circle; the same chip the presence row will use later. */
export function Avatar({ user, withName = false, size = 'md' }: Props) {
  return (
    <span
      className={`avatar ${size}`}
      title={user.bot ? `${user.displayName} (agent)` : user.displayName}
    >
      <span className="avatar-dot" style={{ background: user.color }} aria-hidden>
        {initials(user.displayName)}
        {user.bot && (
          <span className="avatar-bot" aria-hidden>
            AI
          </span>
        )}
      </span>
      {withName && <span className="avatar-name">{user.displayName}</span>}
    </span>
  );
}

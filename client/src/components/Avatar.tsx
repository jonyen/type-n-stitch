import { initials } from '../format';
import type { User } from '../types';

interface Props {
  user: Pick<User, 'displayName' | 'color'>;
  /** Show the name beside the circle. */
  withName?: boolean;
  size?: 'sm' | 'md';
}

/** A colored initial circle; the same chip the presence row will use later. */
export function Avatar({ user, withName = false, size = 'md' }: Props) {
  return (
    <span className={`avatar ${size}`} title={user.displayName}>
      <span className="avatar-dot" style={{ background: user.color }} aria-hidden>
        {initials(user.displayName)}
      </span>
      {withName && <span className="avatar-name">{user.displayName}</span>}
    </span>
  );
}

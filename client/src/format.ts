// Small display helpers for people and times.

/** Up to two initials from a display name; "?" when there is nothing to show. */
export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.slice(0, 2).map((w) => w[0]?.toUpperCase() ?? '');
  return letters.join('') || '?';
}

/** A clip length as m:ss, for cards and badges. */
export function clipLength(seconds: number): string {
  const s = Math.round(seconds);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** "just now", "3 hours ago", or a date once it is more than a month old. */
export function relativeTime(unixSeconds: number, now = Date.now() / 1000): string {
  const delta = Math.max(0, now - unixSeconds);
  if (delta < MINUTE) return 'just now';
  if (delta < HOUR) return `${Math.floor(delta / MINUTE)} min ago`;
  if (delta < DAY) return plural(Math.floor(delta / HOUR), 'hour');
  if (delta < 30 * DAY) return plural(Math.floor(delta / DAY), 'day');
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

function plural(n: number, unit: string): string {
  return `${n} ${unit}${n === 1 ? '' : 's'} ago`;
}

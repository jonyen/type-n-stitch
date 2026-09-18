import { describe, expect, it } from 'vitest';

import { clipLength, initials, relativeTime } from './format';

describe('initials', () => {
  it('takes the first letter of the first two words, upper-cased', () => {
    expect(initials('ada lovelace')).toBe('AL');
    expect(initials('Grace Brewster Murray Hopper')).toBe('GB');
  });

  it('uses one letter for a single word and falls back to ? when empty', () => {
    expect(initials('Demo')).toBe('D');
    expect(initials('  ')).toBe('?');
  });

  it('treats an email as one word', () => {
    expect(initials('jon@example.com')).toBe('J');
  });
});

describe('relativeTime', () => {
  const now = 1_700_000_000;

  it('says just now under a minute', () => {
    expect(relativeTime(now - 5, now)).toBe('just now');
  });

  it('counts minutes, hours and days', () => {
    expect(relativeTime(now - 90, now)).toBe('1 min ago');
    expect(relativeTime(now - 60 * 45, now)).toBe('45 min ago');
    expect(relativeTime(now - 3600 * 3, now)).toBe('3 hours ago');
    expect(relativeTime(now - 3600, now)).toBe('1 hour ago');
    expect(relativeTime(now - 86400 * 2, now)).toBe('2 days ago');
  });

  it('shows a date after a month', () => {
    expect(relativeTime(now - 86400 * 40, now)).toMatch(/\d{4}/);
  });
});

describe('clipLength', () => {
  it('formats m:ss and rounds to whole seconds', () => {
    expect(clipLength(20)).toBe('0:20');
    expect(clipLength(59.6)).toBe('1:00');
    expect(clipLength(3725)).toBe('62:05');
  });
});

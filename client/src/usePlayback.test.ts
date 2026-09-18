import { describe, expect, it } from 'vitest';

import type { TitleEdit } from './types';
import { titleCrossed } from './usePlayback';

const t = (at: number): TitleEdit => ({
  kind: 'title',
  at,
  duration: 1,
  text: 'T',
  subtitle: null,
  style: 'dark',
});

describe('titleCrossed', () => {
  it('returns the first title whose instant was crossed since the previous tick', () => {
    expect(titleCrossed([t(5)], 4.9, 5.0)).toEqual(t(5));
    expect(titleCrossed([t(5)], 5.0, 5.1)).toBeNull();
    expect(titleCrossed([t(7), t(5)], 4.9, 8)).toEqual(t(5));
    expect(titleCrossed([t(5)], 6, 4)).toBeNull(); // seeking backwards never triggers
  });

  it('returns null when there are no titles or none in range', () => {
    expect(titleCrossed([], 0, 10)).toBeNull();
    expect(titleCrossed([t(5)], 0, 1)).toBeNull();
    expect(titleCrossed([t(5)], 3, 3)).toBeNull();
  });

  it('keeps edit order between titles sharing an instant', () => {
    const first = { ...t(5), text: 'first' };
    const second = { ...t(5), text: 'second' };
    expect(titleCrossed([first, second], 4, 6)).toEqual(first);
  });
});

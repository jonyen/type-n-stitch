import { describe, expect, it } from 'vitest';

import type { TitleEdit } from './types';
import { nextTitleAt, tickSpan, titleCrossed } from './usePlayback';

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

describe('tickSpan', () => {
  it('reaches the far side of a cut skip or an overdub', () => {
    expect(tickSpan(1.9, null, null)).toBe(1.9);
    expect(tickSpan(2.0, 4.0, null)).toBe(4.0);
    expect(tickSpan(2.0, null, 5.0)).toBe(5.0);
    // A jump never pulls the span backwards.
    expect(tickSpan(6.0, 4.0, null)).toBe(6.0);
  });
});

describe('titleCrossed over a tick span', () => {
  it('sees a title buried inside a cut the tick skips over', () => {
    // The playhead is at 1.9, a cut covers [2, 4) and a title sits at 3.0:
    // the tick jumps to 4, so the crossing has to be checked over (1.9, 4].
    const buried = t(3);
    expect(titleCrossed([buried], 1.9, tickSpan(1.9, null, null))).toBeNull();
    expect(titleCrossed([buried], 1.9, tickSpan(2.0, 4.0, null))).toEqual(buried);
  });

  it('sees a title inside an overdubbed range', () => {
    const inside = t(5.5);
    expect(titleCrossed([inside], 4.9, tickSpan(5.0, null, 6.0))).toEqual(inside);
  });
});

describe('nextTitleAt', () => {
  it('walks the titles sharing an instant in edit order, then stops', () => {
    const first = { ...t(5), text: 'first' };
    const second = { ...t(5), text: 'second' };
    const later = t(9);
    const list = [first, second, later];
    expect(nextTitleAt(list, 5, [])).toEqual(first);
    expect(nextTitleAt(list, 5, [first])).toEqual(second);
    expect(nextTitleAt(list, 5, [first, second])).toBeNull();
  });

  it('ignores titles at other instants and titles already shown', () => {
    const only = t(5);
    expect(nextTitleAt([t(9)], 5, [])).toBeNull();
    expect(nextTitleAt([only], 5, [only])).toBeNull();
    expect(nextTitleAt([], 5, [])).toBeNull();
  });

  it('still offers a surviving card when the one that was shown is undone', () => {
    const shown = { ...t(5), text: 'undone' };
    const survivor = { ...t(5), text: 'survivor' };
    // `shown` was removed from the edit list while its card was up.
    expect(nextTitleAt([survivor], 5, [shown])).toEqual(survivor);
  });
});

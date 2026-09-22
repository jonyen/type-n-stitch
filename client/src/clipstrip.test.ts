import { describe, expect, it } from 'vitest';
import { dropSlot, firstWords, moveFor } from './clipstrip';

const ordered = [
  { start: 0, end: 2 },
  { start: 2, end: 5 },
  { start: 5, end: 10 },
];

describe('clip strip', () => {
  it('dropSlot picks the gap nearest the pointer', () => {
    expect(dropSlot(0, [50, 150, 250])).toBe(0);
    expect(dropSlot(120, [50, 150, 250])).toBe(1);
    expect(dropSlot(400, [50, 150, 250])).toBe(3);
  });
  it('moveFor turns a slot into a move op, or null for a no-op', () => {
    expect(moveFor(ordered, 2, 0)).toEqual({ piece: 5, before: 0 });
    expect(moveFor(ordered, 0, 3)).toEqual({ piece: 0, before: null });
    expect(moveFor(ordered, 0, 2)).toEqual({ piece: 0, before: 5 });
    expect(moveFor(ordered, 1, 1)).toBeNull();
    expect(moveFor(ordered, 1, 2)).toBeNull();
  });
  it('firstWords takes the first few words a piece owns', () => {
    const words = [0, 0.9, 1.2, 2.1, 3].map((s, i) => ({
      id: `w${i}`,
      text: `w${i}`,
      start: s,
      end: s + 0.3,
    }));
    expect(firstWords(words, { start: 0, end: 2 }, 2)).toBe('w0 w1…');
    expect(firstWords(words, { start: 2, end: 5 })).toBe('w3 w4');
  });
});

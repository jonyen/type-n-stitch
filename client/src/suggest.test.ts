import { describe, expect, it } from 'vitest';

import { defaultSuggestOptions, fillerCuts, normalizeWord, pauseCuts, pending } from './suggest';
import type { CutEdit, Word } from './types';

const w = (i: number, text: string, start: number, end: number): Word => ({
  id: `w${i}`,
  text,
  start,
  end,
});
const cut = (start: number, end: number): CutEdit => ({ kind: 'cut', start, end });

// Same fixture as engine/src/suggest.rs so the two stay in lockstep.
const sentence: Word[] = [
  w(0, 'So,', 0.0, 0.3),
  w(1, 'um,', 0.4, 0.6),
  w(2, 'I', 0.7, 0.8),
  w(3, 'think', 0.8, 1.1),
  w(4, 'UH', 1.2, 1.4),
  w(5, 'you', 1.5, 1.6),
  w(6, 'know', 1.6, 1.8),
  w(7, "it's", 1.9, 2.0),
  w(8, 'fine.', 2.0, 2.4),
  w(9, 'Hmm.', 3.0, 3.3),
];

describe('normalizeWord', () => {
  it('strips case and surrounding punctuation only', () => {
    expect(normalizeWord('Um,')).toBe('um');
    expect(normalizeWord('"Uh..."')).toBe('uh');
    expect(normalizeWord("it's")).toBe("it's");
    expect(normalizeWord('...')).toBe('');
  });
});

describe('fillerCuts', () => {
  it('matches case-insensitively and owns the gap after each filler', () => {
    expect(fillerCuts(sentence, 10)).toEqual([cut(0.4, 0.7), cut(1.2, 1.5), cut(3.0, 10)]);
  });

  it('cuts two-word fillers as one only when the option is on', () => {
    const opts = { ...defaultSuggestOptions, twoWordFillers: true };
    expect(fillerCuts(sentence, 10, opts)).toEqual([
      cut(0.4, 0.7),
      cut(1.2, 1.5),
      cut(1.5, 1.9),
      cut(3.0, 10),
    ]);
  });

  it('needs both words of a two-word filler in order', () => {
    const words = [w(0, 'I', 0, 0.1), w(1, 'know', 0.1, 0.3), w(2, 'mean', 0.3, 0.5)];
    expect(fillerCuts(words, 1, { ...defaultSuggestOptions, twoWordFillers: true })).toEqual([]);
  });

  it('is empty for clean speech', () => {
    expect(fillerCuts([], 5)).toEqual([]);
    expect(fillerCuts([w(0, 'hello', 0, 0.5), w(1, 'there', 0.5, 1)], 5)).toEqual([]);
  });
});

describe('pauseCuts', () => {
  it('tightens gaps over 0.6 s to 0.25 s and leaves the rest alone', () => {
    const words = [
      w(0, 'one', 0.2, 0.5),
      w(1, 'two', 0.9, 1.2), // 0.4 s gap: kept
      w(2, 'three', 2.0, 2.3), // 0.8 s gap: tightened
      w(3, 'four', 2.9, 3.1), // exactly 0.6: kept
      w(4, 'five', 5.0, 5.4), // 1.9 s gap: tightened
    ];
    expect(pauseCuts(words)).toEqual([cut(1.45, 2.0), cut(3.35, 5.0)]);
  });

  it('trims leading silence beyond 0.5 s, keeping a quarter second', () => {
    expect(pauseCuts([w(0, 'hi', 1.5, 1.8), w(1, 'there', 1.9, 2.2)])).toEqual([cut(0, 1.25)]);
    expect(pauseCuts([w(0, 'hi', 0.5, 0.8)])).toEqual([]);
    expect(pauseCuts([])).toEqual([]);
  });
});

describe('pending', () => {
  it('drops suggestions already inside an existing cut', () => {
    const suggested = [cut(0.4, 0.7), cut(1.2, 1.5), cut(3.0, 10)];
    expect(pending(suggested, [])).toEqual(suggested);
    expect(pending(suggested, [cut(1.0, 2.0)])).toEqual([cut(0.4, 0.7), cut(3.0, 10)]);
    expect(pending(suggested, suggested)).toEqual([]);
  });

  it('keeps a suggestion that only partly overlaps a cut', () => {
    expect(pending([cut(1.2, 1.5)], [cut(1.3, 5)])).toEqual([cut(1.2, 1.5)]);
  });
});

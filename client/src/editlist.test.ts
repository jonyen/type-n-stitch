import { describe, expect, it } from 'vitest';

import {
  formatTime,
  keptSegments,
  normalizeCuts,
  outputDuration,
  overdubAt,
  rangeForWords,
  skipTarget,
  wordIndexAt,
  wordStatus,
} from './editlist';
import type { Edit, Word } from './types';

const words: Word[] = [
  { id: 'w0', text: 'thankful', start: 0, end: 0.91 },
  { id: 'w1', text: 'for', start: 0.91, end: 1.25 },
  { id: 'w2', text: 'you', start: 1.25, end: 1.59 },
  { id: 'w3', text: 'as', start: 2.0, end: 2.3 },
];

const cut = (start: number, end: number): Edit => ({ kind: 'cut', start, end });
const overdub = (start: number, end: number, audioDuration: number): Edit => ({
  kind: 'overdub',
  start,
  end,
  text: 'new words',
  audioUrl: '/data/x/overdub-0.wav',
  audioDuration,
});

describe('normalizeCuts', () => {
  it('sorts, merges overlapping and touching ranges, drops empties', () => {
    expect(
      normalizeCuts([
        { start: 5, end: 6 },
        { start: 1, end: 2 },
        { start: 1.5, end: 3 },
        { start: 3, end: 4 },
        { start: 7, end: 7 },
      ]),
    ).toEqual([
      { start: 1, end: 4 },
      { start: 5, end: 6 },
    ]);
  });
});

describe('keptSegments / outputDuration', () => {
  it('is the whole media with no edits', () => {
    expect(keptSegments(10, [])).toEqual([{ start: 0, end: 10 }]);
    expect(outputDuration(10, [])).toBe(10);
  });

  it('removes cuts and stretches overdubs', () => {
    const edits = [cut(1, 3), overdub(5, 6, 2.5)];
    expect(keptSegments(10, edits)).toEqual([
      { start: 0, end: 1 },
      { start: 3, end: 10 },
    ]);
    expect(outputDuration(10, edits)).toBeCloseTo(10 - 2 - 1 + 2.5);
  });
});

describe('rangeForWords', () => {
  it('runs to the next word start so the trailing gap goes too', () => {
    expect(rangeForWords(words, 1, 2, 20)).toEqual({ start: 0.91, end: 2.0 });
  });

  it('runs to the end of the media for the last word', () => {
    expect(rangeForWords(words, 3, 3, 20)).toEqual({ start: 2.0, end: 20 });
  });
});

describe('wordStatus', () => {
  it('marks covered words and lets overdub win over cut', () => {
    const edits = [cut(0.91, 2.0), overdub(1.25, 2.0, 1)];
    expect(words.map((w) => wordStatus(w, edits))).toEqual(['kept', 'cut', 'overdub', 'kept']);
  });

  it('does not mark a partially covered word', () => {
    expect(wordStatus(words[0]!, [cut(0.5, 2)])).toBe('kept');
  });
});

describe('skipTarget / overdubAt', () => {
  it('jumps to the end of the cut containing t', () => {
    const cuts = normalizeCuts([
      { start: 2, end: 4 },
      { start: 6, end: 7 },
    ]);
    expect(skipTarget(1, cuts)).toBeNull();
    expect(skipTarget(2, cuts)).toBe(4);
    expect(skipTarget(3.99, cuts)).toBe(4);
    expect(skipTarget(4, cuts)).toBeNull();
    expect(skipTarget(6.5, cuts)).toBe(7);
  });

  it('finds the overdub containing t', () => {
    const edits = [cut(0, 1), overdub(5, 6, 2)];
    expect(overdubAt(5.5, edits)?.audioDuration).toBe(2);
    expect(overdubAt(6, edits)).toBeUndefined();
  });
});

describe('wordIndexAt', () => {
  it('returns the word whose span owns t', () => {
    expect(wordIndexAt(-1, words)).toBe(-1);
    expect(wordIndexAt(0, words)).toBe(0);
    expect(wordIndexAt(1.0, words)).toBe(1);
    expect(wordIndexAt(1.8, words)).toBe(2); // in the gap after "you"
    expect(wordIndexAt(99, words)).toBe(3);
  });
});

describe('formatTime', () => {
  it('formats m:ss.t', () => {
    expect(formatTime(0)).toBe('0:00.0');
    expect(formatTime(75.26)).toBe('1:15.3');
  });
});

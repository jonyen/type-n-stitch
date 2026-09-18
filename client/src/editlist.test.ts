import { describe, expect, it } from 'vitest';

import {
  captionsAt,
  formatTime,
  joins,
  keptSegments,
  nearDipJoin,
  normalizeCuts,
  outputDuration,
  overdubAt,
  pieces,
  rangeForWords,
  skipTarget,
  wordIndexAt,
  wordStatus,
} from './editlist';
import type { CaptionEdit, Edit, TitleEdit, Word } from './types';

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
    const [thankful] = words;
    if (!thankful) throw new Error('fixture');
    expect(wordStatus(thankful, [cut(0.5, 2)])).toBe('kept');
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

describe('titles and joins', () => {
  const title = (at: number, duration: number): TitleEdit => ({
    kind: 'title',
    at,
    duration,
    text: 'T',
    subtitle: null,
    style: 'dark',
  });
  it('outputDuration adds title durations', () => {
    expect(outputDuration(10, [title(4, 2), { kind: 'cut', start: 1, end: 2 }])).toBe(11);
  });
  it('pieces mirror the engine: split at a title, title precedes an overdub at the same instant', () => {
    const p = pieces(10, [
      title(5, 1),
      { kind: 'overdub', start: 5, end: 6, text: 'x', audioUrl: '/a', audioDuration: 0.5 },
    ]);
    expect(p.map((x) => x.kind)).toEqual(['source', 'title', 'overdub', 'source']);
    expect(p[0]?.source).toEqual({ start: 0, end: 5 });
    expect(p[3]?.source).toEqual({ start: 6, end: 10 });
  });
  it('joins: override, then project default, always dip around titles, none around overdubs', () => {
    const edits: Edit[] = [
      { kind: 'cut', start: 2, end: 3, transition: 'none' },
      { kind: 'cut', start: 5, end: 6 },
      title(8, 1),
      { kind: 'overdub', start: 9, end: 9.5, text: 'x', audioUrl: '/a', audioDuration: 1 },
    ];
    const p = pieces(10, edits);
    expect(joins(p, edits, 'dip').map((j) => j.transition)).toEqual([
      'none',
      'dip',
      'dip',
      'dip',
      'none',
      'none',
    ]);
  });
  it('takes an override only from the cut starting at the boundary', () => {
    // Two adjacent cuts fill one gap; only the first one's start is the
    // boundary, so the second's override must not leak into the join.
    const edits: Edit[] = [
      { kind: 'cut', start: 2, end: 3 },
      { kind: 'cut', start: 3, end: 5, transition: 'none' },
    ];
    const p = pieces(10, edits);
    expect(joins(p, edits, 'dip').map((j) => j.transition)).toEqual(['dip']);
  });
  it('nearDipJoin is true within 0.25 s of a dipping boundary in source time', () => {
    const edits: Edit[] = [{ kind: 'cut', start: 3, end: 5 }];
    const p = pieces(10, edits);
    const j = joins(p, edits, 'dip');
    expect(nearDipJoin(2.9, p, j)).toBe(true);
    expect(nearDipJoin(5.2, p, j)).toBe(true);
    expect(nearDipJoin(4, p, j)).toBe(false);
    expect(nearDipJoin(2.5, p, j)).toBe(false);
  });
  it('captionsAt returns captions whose range contains t', () => {
    const c: CaptionEdit = { kind: 'caption', start: 1, end: 2, text: 'c', position: 'topLeft' };
    expect(captionsAt(1.5, [c])).toEqual([c]);
    expect(captionsAt(2, [c])).toEqual([]);
  });
});

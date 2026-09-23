import { describe, expect, it } from 'vitest';

import {
  captionsAt,
  cutTransitionAt,
  EPS,
  formatTime,
  isJoin,
  joins,
  jumpTarget,
  keptSegments,
  locate,
  nearDipJoin,
  nextCutTransition,
  normalizeCuts,
  orderedPieces,
  orderStarts,
  outputDuration,
  overdubAt,
  pieces,
  pieceStarts,
  rangeForWords,
  skipTarget,
  sourceJoins,
  stitchedDuration,
  wordIndexAt,
  wordStatus,
} from './editlist';
import type { CaptionEdit, Edit, Source, TitleEdit, Word } from './types';

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

  // Two files: video 1 is [0, 3), video 2 is [3, 23). The server clamps an MCP cut the same way.
  const joined: Source[] = [
    { media: 'm0', offset: 0, duration: 3 },
    { media: 'm1', offset: 3, duration: 20 },
  ];
  const across: Word[] = [...words, { id: '1:w0', text: 'next', start: 3.4, end: 3.8 }];

  it("stops the last word of a file at that file's end, not at the next file's first word", () => {
    expect(rangeForWords(across, 3, 3, 23, joined)).toEqual({ start: 2.0, end: 3 });
    // Selected across the join: the last word's own file bounds it.
    expect(rangeForWords(across, 2, 4, 23, joined)).toEqual({ start: 1.25, end: 23 });
  });

  it('never runs into a later file that has no words yet', () => {
    expect(rangeForWords(words, 3, 3, 23, joined)).toEqual({ start: 2.0, end: 3 });
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

describe('cut transition overrides', () => {
  it('cycles none set → dip → jump cut → none set', () => {
    expect(nextCutTransition(null)).toBe('dip');
    expect(nextCutTransition('dip')).toBe('none');
    expect(nextCutTransition('none')).toBe(null);
  });

  it('reads the override off the cut starting at an instant', () => {
    const edits: Edit[] = [
      { kind: 'cut', start: 1, end: 2, transition: 'dip' },
      { kind: 'cut', start: 3, end: 4 },
    ];
    expect(cutTransitionAt(1, edits)).toBe('dip');
    expect(cutTransitionAt(3, edits)).toBe(null);
    expect(cutTransitionAt(9, edits)).toBe(null);
  });
});

describe('pieceStarts', () => {
  it('mirrors the engine: cut ends and splits outside cuts', () => {
    expect(pieceStarts([], [])).toEqual([0]);
    expect(pieceStarts([cut(2, 3)], [])).toEqual([0, 3]);
    expect(pieceStarts([cut(2, 3)], [2.5, 5, 3, 0])).toEqual([0, 3, 5]);
    expect(pieceStarts([cut(0, 1)], [])).toEqual([1]);
  });
});

describe('orderedPieces and jumps', () => {
  it('orders live entries first, then the rest in source order', () => {
    expect(orderedPieces(10, [cut(4, 5)], [2], [5, 4, 0])).toEqual([
      { start: 5, end: 10 },
      { start: 0, end: 2 },
      { start: 2, end: 4 },
    ]);
  });
  // Three ten-second videos moved to C, A, B: order [20, 0, 10].
  const startsOf = (list: { start: number }[]) => list.map((p) => p.start);
  it('a cut or split inside a reordered piece keeps the remainder with it', () => {
    const order = [20, 0, 10];
    expect(startsOf(orderedPieces(30, [cut(3, 5)], [10, 20], order))).toEqual([20, 0, 5, 10]);
    expect(startsOf(orderedPieces(30, [], [10, 20, 5], order))).toEqual([20, 0, 5, 10]);
  });
  it('orderStarts groups each start under its parent, like the engine', () => {
    expect(orderStarts([10, 0, 5], [])).toEqual([0, 5, 10]);
    expect(orderStarts([2, 10, 20], [20, 0, 10])).toEqual([20, 2, 10]);
    expect(orderStarts([1, 5, 8], [8, 5])).toEqual([8, 1, 5]);
  });
  it('a head cut of a reordered piece keeps its place', () => {
    expect(startsOf(orderedPieces(30, [cut(0, 2)], [10, 20], [20, 0, 10]))).toEqual([20, 2, 10]);
  });
  it('pieces lays out sub-pieces per ordered piece', () => {
    const list = pieces(10, [overdub(6, 7, 2)], [5], [5, 0]);
    expect(list.map((p) => [p.kind, p.source.start])).toEqual([
      ['source', 5],
      ['overdub', 6],
      ['source', 7],
      ['source', 0],
    ]);
    expect(pieces(10, [cut(2, 4)])).toEqual(pieces(10, [cut(2, 4)], [], []));
  });
  it('jumpTarget follows output order and matches skipTarget for plain cuts', () => {
    const ordered = orderedPieces(10, [cut(2, 3)], [], []);
    expect(jumpTarget(1, ordered)).toBeNull();
    expect(jumpTarget(2.5, ordered)).toBe(3);
    const swapped = orderedPieces(10, [], [5], [5, 0]);
    expect(jumpTarget(10, swapped)).toBe(0);
    expect(jumpTarget(5, swapped)).toBeNull();
    expect(jumpTarget(5, orderedPieces(10, [], [5], [0, 5]))).toBeNull();
    expect(jumpTarget(10, orderedPieces(10, [], [5], [0, 5]))).toBe(Infinity);
    // Before the first piece (cut from zero): the first output piece.
    expect(jumpTarget(0.5, orderedPieces(10, [cut(0, 1)], [], []))).toBe(1);
  });
  it('a reorder join takes the project transition', () => {
    const list = pieces(10, [], [5], [5, 0]);
    expect(joins(list, [], 'dip')).toEqual([{ after: 0, transition: 'dip' }]);
  });
});

describe('stitched sources (mirror engine/src/editlist.rs)', () => {
  // The engine's `three()`: 10 s, 5 s and 2.5 s files end to end.
  const three: Source[] = [
    { media: 'm1', offset: 0, duration: 10 },
    { media: 'm2', offset: 10, duration: 5 },
    { media: 'm3', offset: 15, duration: 2.5 },
  ];

  it('ends where the last source ends', () => {
    expect(stitchedDuration(three)).toBe(17.5);
    expect(stitchedDuration([])).toBe(0);
  });

  it('locates at every boundary', () => {
    expect(locate(three, 0)).toEqual({ index: 0, local: 0 }); // the very start
    expect(locate(three, 4.25)).toEqual({ index: 0, local: 4.25 }); // inside the first
    expect(locate(three, 9.5)).toEqual({ index: 0, local: 9.5 }); // just before a join
    expect(locate(three, 10)).toEqual({ index: 1, local: 0 }); // a join belongs to the later source
    expect(locate(three, 12)).toEqual({ index: 1, local: 2 }); // inside the second
    expect(locate(three, 15)).toEqual({ index: 2, local: 0 }); // the second join
    expect(locate(three, 17.5)).toEqual({ index: 2, local: 2.5 }); // the exact end: last source at its duration
    expect(locate(three, 17.6)).toBeNull(); // past the end
    expect(locate(three, -0.1)).toBeNull(); // before the start
    expect(locate([], 0)).toBeNull(); // no sources
  });

  it('snaps within EPS of a join or the end', () => {
    expect(locate(three, 10 - EPS / 2)).toEqual({ index: 1, local: 0 });
    expect(locate(three, 17.5 + EPS / 2)).toEqual({ index: 2, local: 2.5 });
    expect(locate(three, -EPS / 2)).toEqual({ index: 0, local: 0 });
  });

  it('is the identity on one source', () => {
    const one: Source[] = [{ media: 'm1', offset: 0, duration: 10 }];
    expect(locate(one, 3.5)).toEqual({ index: 0, local: 3.5 });
    expect(locate(one, 10)).toEqual({ index: 0, local: 10 });
    expect(locate(one, 10.5)).toBeNull();
  });

  it('lists the joins, and knows an instant on one', () => {
    expect(sourceJoins(three)).toEqual([10, 15]);
    expect(sourceJoins(three.slice(0, 1))).toEqual([]);
    expect(isJoin(15, three)).toBe(true);
    expect(isJoin(15 + EPS / 2, three)).toBe(true);
    expect(isJoin(12, three)).toBe(false);
    expect(isJoin(0, three)).toBe(false);
  });
});

import { describe, expect, it } from 'vitest';

import { editorReducer, initialEditor } from './editor';
import {
  canSplitAt,
  clipSpans,
  nearestStop,
  outputToSource,
  overlaySpans,
  pxToOutput,
  razorAt,
  rangeCuts,
  rulerTicks,
  snappedBand,
  sourceToOutput,
  timelineLength,
  timelineSegments,
  wordStops,
} from './timeline';
import type { Edit, Word } from './types';

const cut = (start: number, end: number): Edit => ({ kind: 'cut', start, end });
const overdub = (start: number, end: number, audioDuration: number): Edit => ({
  kind: 'overdub',
  start,
  end,
  text: 'x',
  audioUrl: '/data/m/od.wav',
  audioDuration,
});
const title = (at: number, duration: number): Edit => ({
  kind: 'title',
  at,
  duration,
  text: 'T',
  subtitle: null,
  style: 'dark',
});
/** One word per second: word i spans [i, i + 0.5). */
const words = (n: number): Word[] =>
  Array.from({ length: n }, (_, i) => ({ id: `w${i}`, text: `w${i}`, start: i, end: i + 0.5 }));
const r = (start: number, end: number) => ({ start, end });

describe('timelineSegments', () => {
  it('closes cut gaps and gives holds their rendered length', () => {
    const segs = timelineSegments(10, [cut(2, 4), overdub(6, 7, 3), title(8, 2)], [], []);
    expect(segs.map((s) => [s.kind, s.source, s.output])).toEqual([
      ['source', r(0, 2), r(0, 2)],
      ['source', r(4, 6), r(2, 4)],
      ['overdub', r(6, 7), r(4, 7)],
      ['source', r(7, 8), r(7, 8)],
      ['title', r(8, 8), r(8, 10)],
      ['source', r(8, 10), r(10, 12)],
    ]);
    expect(timelineLength(segs)).toBe(12);
  });

  it('lays pieces out in order and keeps a boundary title with its piece (engine case)', () => {
    const segs = timelineSegments(10, [title(5, 1)], [5], [5, 0]);
    expect(segs.map((s) => [s.kind, s.source, s.output])).toEqual([
      ['title', r(5, 5), r(0, 1)],
      ['source', r(5, 10), r(1, 6)],
      ['source', r(0, 5), r(6, 11)],
    ]);
  });

  it('moves an overdub with its reordered piece (engine case)', () => {
    const segs = timelineSegments(10, [overdub(6, 7, 2)], [5], [5, 0]);
    expect(segs[0]?.source).toEqual(r(5, 6));
    expect(segs[1]?.kind).toBe('overdub');
    expect(segs[1]?.output).toEqual(r(1, 3));
    expect(segs[3]?.source).toEqual(r(0, 5));
    expect(timelineLength(segs)).toBe(11);
  });
});

describe('sourceToOutput and outputToSource (engine cases)', () => {
  it('skips a cut', () => {
    const segs = timelineSegments(10, [cut(2, 4)], [], []);
    expect(sourceToOutput(1, segs)).toBe(1);
    expect(sourceToOutput(3, segs)).toBe(2);
    expect(sourceToOutput(5, segs)).toBe(3);
    expect(sourceToOutput(10, segs)).toBe(8);
    expect(outputToSource(1, segs)).toBe(1);
    expect(outputToSource(2, segs)).toBe(4);
    expect(outputToSource(7.9, segs)).toBeCloseTo(9.9);
    expect(outputToSource(8, segs)).toBe(10);
  });

  it('freezes on an overdub and on a title', () => {
    const od = timelineSegments(10, [overdub(2, 3, 4)], [], []);
    expect(sourceToOutput(2.5, od)).toBe(2);
    expect(sourceToOutput(3, od)).toBe(6);
    expect(outputToSource(4, od)).toBe(2);
    expect(outputToSource(6.5, od)).toBe(3.5);
    const ti = timelineSegments(10, [title(4, 2)], [], []);
    expect(sourceToOutput(4, ti)).toBe(4);
    expect(sourceToOutput(4.5, ti)).toBe(6.5);
    expect(outputToSource(5, ti)).toBe(4);
    expect(outputToSource(6.5, ti)).toBe(4.5);
  });

  it('follows the output order', () => {
    const segs = timelineSegments(10, [], [5], [5, 0]);
    expect(sourceToOutput(6, segs)).toBe(1);
    expect(sourceToOutput(1, segs)).toBe(6);
    expect(outputToSource(1, segs)).toBe(6);
    expect(outputToSource(6, segs)).toBe(1);
    const reordered = timelineSegments(10, [cut(2, 3)], [], [3, 0]);
    expect(sourceToOutput(2.5, reordered)).toBe(0);
  });

  it('maps inside a cut to the title that follows it', () => {
    const segs = timelineSegments(10, [cut(2, 3), title(3, 1)], [], []);
    expect(sourceToOutput(2.5, segs)).toBe(2);
    expect(sourceToOutput(3, segs)).toBe(2);
  });
});

describe('word stops', () => {
  it('places kept word starts in output time and ends with the media end', () => {
    const segs = timelineSegments(5, [cut(1, 3)], [], []);
    expect(wordStops(words(5), segs)).toEqual([
      { at: 0, word: 0 },
      { at: 1, word: 3 },
      { at: 2, word: 4 },
      { at: 3, word: 5 },
    ]);
  });

  it('keeps an overdubbed run as one stop at the start of its hold', () => {
    const segs = timelineSegments(5, [overdub(1, 3, 4)], [], []);
    expect(wordStops(words(5), segs).map((s) => s.word)).toEqual([0, 1, 3, 4, 5]);
    expect(wordStops(words(5), segs)[2]).toEqual({ at: 5, word: 3 });
  });

  it('finds the nearest stop', () => {
    const stops = [
      { at: 0, word: 0 },
      { at: 1, word: 1 },
      { at: 3, word: 2 },
    ];
    expect(nearestStop(1.4, stops)).toEqual({ at: 1, word: 1 });
    expect(nearestStop(2.1, stops)).toEqual({ at: 3, word: 2 });
    expect(nearestStop(0, [])).toBeNull();
  });
});

describe('split legality', () => {
  it('follows the server rules and refuses a split that would do nothing', () => {
    const edits = [cut(2, 4), overdub(6, 8, 1)];
    expect(canSplitAt(0, edits, [], 10)).toBe(false); // start of media
    expect(canSplitAt(10, edits, [], 10)).toBe(false); // end of media
    expect(canSplitAt(3, edits, [], 10)).toBe(false); // inside a cut
    expect(canSplitAt(7, edits, [], 10)).toBe(false); // inside an overdub
    expect(canSplitAt(4, edits, [], 10)).toBe(false); // a cut boundary is already a piece start
    expect(canSplitAt(5, edits, [5], 10)).toBe(false); // an existing split
    expect(canSplitAt(5, edits, [], 10)).toBe(true);
  });

  it('treats touching cuts as one', () => {
    expect(canSplitAt(3, [cut(2, 3), cut(3, 4)], [], 10)).toBe(false);
  });

  it('refuses when the split count reaches the server cap', () => {
    expect(
      canSplitAt(
        5,
        [],
        Array.from({ length: 64 }, (_, i) => 0.05 + i * 0.1),
        10,
      ),
    ).toBe(false);
  });
});

describe('razor', () => {
  const ws = words(10);

  it('snaps to the nearest kept word start and splits at its source time', () => {
    const segs = timelineSegments(10, [cut(2, 4)], [], []);
    expect(razorAt(2.9, ws, segs, [cut(2, 4)], [], 10)).toEqual({ ok: true, at: 5, x: 3 });
  });

  it('refuses on a title hold, at the edges and on an existing piece start', () => {
    const edits = [title(5, 2)];
    const segs = timelineSegments(10, edits, [], []);
    expect(razorAt(5.5, ws, segs, edits, [], 10).ok).toBe(false); // on the card
    expect(razorAt(0.1, ws, segs, edits, [], 10)).toEqual({ ok: false, x: 0 });
    expect(razorAt(11.9, ws, segs, edits, [], 10)).toEqual({ ok: false, x: 12 });
    const plain = timelineSegments(10, [], [3], []);
    expect(razorAt(3.2, ws, plain, [], [3], 10)).toEqual({ ok: false, x: 3 });
  });
});

describe('range', () => {
  const ws = words(10);

  it('snaps the band to word starts in either drag direction', () => {
    const segs = timelineSegments(10, [], [], []);
    expect(snappedBand(1.2, 3.7, ws, segs)).toEqual(r(1, 4));
    expect(snappedBand(3.7, 1.2, ws, segs)).toEqual(r(1, 4));
    expect(snappedBand(1.2, 1.3, ws, segs)).toBeNull();
  });

  it('turns a band into source cuts, one per stretch of source it covers', () => {
    const segs = timelineSegments(10, [], [5], [5, 0]);
    // Output [4, 7) covers source [9, 10) (end of the first clip) then [0, 2).
    expect(rangeCuts(4, 7, ws, segs)).toEqual([r(0, 2), r(9, 10)]);
  });

  it('takes an overdub only when the band covers its whole hold, and skips titles', () => {
    const edits = [overdub(2, 3, 2), title(6, 1)];
    const segs = timelineSegments(10, edits, [], []);
    // Layout: [0,2) src, [2,4) overdub hold, [4,7) src 3..6, [7,8) title, [8,12) src 6..10
    expect(rangeCuts(1, 5, ws, segs)).toEqual([r(1, 4)]);
    // 3.2 snaps to the stop at 4, so the band starts after the hold and leaves it alone.
    expect(rangeCuts(3.2, 5, ws, segs)).toEqual([r(3, 4)]);
    expect(rangeCuts(6, 9, ws, segs)).toEqual([r(5, 7)]);
  });

  it('a band over a whole overdub hold removes it once the cuts are applied', () => {
    const edits = [overdub(2, 3, 2)];
    let state = editorReducer(initialEditor, { type: 'load', words: ws, duration: 10 });
    state = editorReducer(state, {
      type: 'sync',
      doc: { headSeq: 1, edits, speakerNames: [], undoable: null, redoable: null },
    });
    const segs = timelineSegments(10, edits, [], []);
    const cuts = rangeCuts(1, 5, ws, segs);
    state = editorReducer(state, {
      type: 'applyCuts',
      cuts: cuts.map((r) => ({ kind: 'cut' as const, ...r })),
    });
    const after = timelineSegments(10, state.edits, [], []);
    expect(after.some((s) => s.kind === 'overdub')).toBe(false);
    // 11 s of output, minus the 4 s the band covered.
    expect(timelineLength(after)).toBe(7);
  });
});

describe('lane geometry', () => {
  it('gives each clip its span in output time, holds included', () => {
    const edits = [title(5, 1)];
    const segs = timelineSegments(10, edits, [5], [5, 0]);
    expect(clipSpans([r(5, 10), r(0, 5)], segs)).toEqual([r(0, 6), r(6, 11)]);
  });

  it('maps an overlay range onto output windows, across a reorder', () => {
    const segs = timelineSegments(10, [], [5], [5, 0]);
    expect(overlaySpans(r(4, 6), segs, false)).toEqual([r(0, 1), r(9, 10)]);
  });

  it('covers a title hold only for music', () => {
    const edits = [title(4, 2)];
    const segs = timelineSegments(10, edits, [], []);
    expect(overlaySpans(r(0, 10), segs, true)).toEqual([r(0, 12)]);
    expect(overlaySpans(r(0, 10), segs, false)).toEqual([r(0, 4), r(6, 12)]);
  });

  it('converts a pointer x to output time, clamped', () => {
    expect(pxToOutput(150, 100, 200, 60)).toBe(15);
    expect(pxToOutput(50, 100, 200, 60)).toBe(0);
    expect(pxToOutput(500, 100, 200, 60)).toBe(60);
  });
});

describe('rulerTicks', () => {
  it('picks a round step that keeps the labels sparse', () => {
    expect(rulerTicks(60)).toEqual([0, 10, 20, 30, 40, 50, 60]);
    expect(rulerTicks(7)).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
    expect(rulerTicks(125)).toEqual([0, 30, 60, 90, 120]);
    expect(rulerTicks(0)).toEqual([0]);
  });
});

// @vitest-environment jsdom
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { formatTime, orderedPieces } from './editlist';
import { timelineLength, timelineSegments } from './timeline';
import type { Range, TitleEdit } from './types';
import {
  nextTitleAt,
  pieceIndexAt,
  playStep,
  restartAt,
  tickSpan,
  titleCrossed,
  usePlayback,
} from './usePlayback';

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

describe('restartAt', () => {
  it('starts at the first output piece when at the beginning or the end', () => {
    const ordered = [
      { start: 5, end: 10 },
      { start: 0, end: 5 },
    ];
    expect(restartAt(0, false, 10, ordered)).toBe(5);
    expect(restartAt(9.995, false, 10, ordered)).toBe(5);
    expect(restartAt(3, false, 10, ordered)).toBeNull();
    expect(restartAt(3, true, 10, ordered)).toBe(5);
  });
});

/** Drives `playStep` until it stops, returning the sequence of piece indices visited. */
function runToStop(ordered: Range[], startIndex: number, stepSize: number): number[] {
  let index = startIndex;
  let t = ordered[index]?.start ?? 0;
  const visited = [index];
  for (let guard = 0; guard < 1000; guard++) {
    const step = playStep(t, index, ordered);
    if (step.kind === 'stop') return visited;
    if (step.kind === 'seek') {
      index = step.index;
      t = step.to;
      visited.push(index);
      continue;
    }
    t += stepSize;
  }
  throw new Error('runToStop did not stop');
}

describe('playStep', () => {
  it('skips the gap of a plain cut, then stops at the end', () => {
    // A cut over [3, 5) of a 10 s source: the kept pieces are [0,3) and [5,10).
    const ordered: Range[] = [
      { start: 0, end: 3 },
      { start: 5, end: 10 },
    ];
    expect(playStep(1, 0, ordered)).toEqual({ kind: 'play' });
    expect(playStep(3, 0, ordered)).toEqual({ kind: 'seek', to: 5, index: 1 });
    expect(playStep(8, 1, ordered)).toEqual({ kind: 'play' });
    expect(playStep(10, 1, ordered)).toEqual({ kind: 'stop' });
    expect(runToStop(ordered, 0, 1)).toEqual([0, 1]);
  });

  it('stops in place at a trailing cut instead of looping', () => {
    // A cut over [7.5, 10) of a 10 s source: only [0, 7.5) survives.
    const ordered: Range[] = [{ start: 0, end: 7.5 }];
    expect(playStep(7, 0, ordered)).toEqual({ kind: 'play' });
    expect(playStep(7.5, 0, ordered)).toEqual({ kind: 'stop' });
    // A second play restarts at the first (only) piece rather than staying stuck.
    expect(restartAt(10, false, 10, ordered)).toBe(0);
  });

  it('plays a reversed two-piece order once through, then stops (not a loop)', () => {
    const ordered: Range[] = [
      { start: 5, end: 10 },
      { start: 0, end: 5 },
    ];
    // Leaving piece 0 at its own end advances to piece 1, never back to
    // whichever piece happens to contain the current source time (piece 1
    // also covers time 5, the tail of piece 0, so a source-time-only lookup
    // would wrongly re-enter piece 0 here).
    expect(playStep(10, 0, ordered)).toEqual({ kind: 'seek', to: 0, index: 1 });
    expect(playStep(5, 1, ordered)).toEqual({ kind: 'stop' });
    expect(runToStop(ordered, 0, 1)).toEqual([0, 1]);
  });
});

describe('pieceIndexAt', () => {
  it('finds the piece containing t after seeking into the middle of the second output piece', () => {
    const ordered: Range[] = [
      { start: 5, end: 10 },
      { start: 0, end: 5 },
    ];
    // t = 2 sits inside the second output piece ([0, 5)), not the first.
    expect(pieceIndexAt(2, ordered)).toBe(1);
    expect(pieceIndexAt(7, ordered)).toBe(0);
  });

  it('falls back to the next piece at or after t, else the last piece', () => {
    const ordered: Range[] = [
      { start: 0, end: 3 },
      { start: 5, end: 10 },
    ];
    // 4 is in the gap between pieces: the next piece starts at 5.
    expect(pieceIndexAt(4, ordered)).toBe(1);
    // Past every piece: the last one.
    expect(pieceIndexAt(20, ordered)).toBe(1);
    expect(pieceIndexAt(0, [])).toBe(0);
  });

  it('resyncs onto the piece that now covers t after ordered changes mid-playback', () => {
    // Playing piece index 1 ([5,10)) at t=7; an edit splits it into [3,4) and
    // [5,10), shifting what index 1 means. Resyncing from `t` (not from the
    // stale index) must land back on the piece that now contains 7.
    const ordered: Range[] = [
      { start: 0, end: 3 },
      { start: 3, end: 4 },
      { start: 5, end: 10 },
    ];
    expect(pieceIndexAt(7, ordered)).toBe(2);
    // A time inside no piece (the gap [4, 5)) resyncs to the next piece.
    expect(pieceIndexAt(4.5, ordered)).toBe(2);
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

/** Just enough of a media element for the hook: a settable clock, play/pause and their events. */
class FakeMedia extends EventTarget {
  currentTime = 0;
  paused = true;
  ended = false;
  play = vi.fn(() => {
    this.paused = false;
    this.dispatchEvent(new Event('play'));
    return Promise.resolve();
  });
  pause = vi.fn(() => {
    if (this.paused) return;
    this.paused = true;
    this.dispatchEvent(new Event('pause'));
  });
}

describe('usePlayback in output time (a reordered edit)', () => {
  // The reported repro: 60.04 s of media, split at 47.9, the tail played first.
  const duration = 60.04;
  const ordered = orderedPieces(duration, [], [47.9], [47.9, 0]);
  const segments = timelineSegments(duration, [], [47.9], [47.9, 0]);
  const length = timelineLength(segments);

  let frames: FrameRequestCallback[] = [];
  beforeEach(() => {
    frames = [];
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal('cancelAnimationFrame', () => undefined);
  });
  afterEach(() => vi.unstubAllGlobals());

  /** Run the playhead watchdog once, with the media clock at `t`. */
  function frameAt(media: FakeMedia, t: number) {
    media.currentTime = t;
    const pending = frames;
    frames = [];
    act(() => pending.forEach((cb) => cb(0)));
  }

  function setup() {
    const media = new FakeMedia();
    const ref = { current: media as unknown as HTMLMediaElement };
    const hook = renderHook(() => usePlayback(ref, [], [], duration, '/m.mp4', ordered, segments));
    return { media, hook };
  }

  it('shows the output length, not 0:12.1, once playback stops at the end', () => {
    const { media, hook } = setup();
    act(() => hook.result.current.toggle());
    // Starts at the first output piece, the tail [47.9, 60.04).
    expect(media.currentTime).toBeCloseTo(47.9);
    expect(hook.result.current.outputTime).toBeCloseTo(0);
    // The tail ends; the head [0, 47.9) follows.
    frameAt(media, 60.04);
    expect(media.currentTime).toBe(0);
    frameAt(media, 20);
    expect(hook.result.current.outputTime).toBeCloseTo(32.1);
    // The head ends: that is the end of the output.
    frameAt(media, 47.9);
    expect(media.paused).toBe(true);
    expect(hook.result.current.atEnd).toBe(true);
    expect(hook.result.current.outputTime).toBeCloseTo(length);
    expect(formatTime(hook.result.current.outputTime)).toBe(formatTime(length));
  });

  it('parks at the end when seeking to the output length, and plays again from the start', () => {
    const { media, hook } = setup();
    act(() => hook.result.current.seekOutput(length));
    expect(hook.result.current.atEnd).toBe(true);
    expect(hook.result.current.outputTime).toBeCloseTo(length);
    // Not piece 0, which starts at the source instant the output ends on.
    expect(media.currentTime).not.toBeCloseTo(47.9);
    act(() => hook.result.current.toggle());
    expect(hook.result.current.atEnd).toBe(false);
    expect(media.currentTime).toBeCloseTo(47.9);
    expect(hook.result.current.outputTime).toBeCloseTo(0);
  });

  it('seeks by output time into the piece that plays there', () => {
    const { media, hook } = setup();
    act(() => hook.result.current.seekOutput(20));
    expect(media.currentTime).toBeCloseTo(7.86);
    expect(hook.result.current.outputTime).toBeCloseTo(20);
    act(() => hook.result.current.seekOutput(2));
    expect(media.currentTime).toBeCloseTo(49.9);
    expect(hook.result.current.outputTime).toBeCloseTo(2);
  });

  it('keeps source-order edits as they were: output equals source', () => {
    const plain = orderedPieces(10, [], [], []);
    const segs = timelineSegments(10, [], [], []);
    const media = new FakeMedia();
    const ref = { current: media as unknown as HTMLMediaElement };
    const hook = renderHook(() => usePlayback(ref, [], [], 10, '/m.mp4', plain, segs));
    act(() => hook.result.current.seek(4.25));
    expect(hook.result.current.currentTime).toBeCloseTo(4.3);
    expect(hook.result.current.outputTime).toBeCloseTo(4.3);
    act(() => hook.result.current.toggle());
    frameAt(media, 10);
    expect(hook.result.current.atEnd).toBe(true);
    expect(hook.result.current.outputTime).toBe(10);
  });
});

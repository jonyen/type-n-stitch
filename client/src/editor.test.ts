import { describe, expect, it } from 'vitest';

import { editorReducer, initialEditor, selectedRange, type EditorState } from './editor';
import type { DocState } from './ops';
import type { Word } from './types';

const words: Word[] = [
  { id: 'w0', text: 'thankful', start: 0, end: 0.91 },
  { id: 'w1', text: 'for', start: 0.91, end: 1.25 },
  { id: 'w2', text: 'you', start: 1.25, end: 1.59 },
  { id: 'w3', text: 'as', start: 2.0, end: 2.3 },
];

const loaded = editorReducer(initialEditor, { type: 'load', words, duration: 20 });

function select(state: EditorState, from: number, to = from): EditorState {
  const first = editorReducer(state, { type: 'select', index: from, extend: false });
  return to === from ? first : editorReducer(first, { type: 'select', index: to, extend: true });
}

describe('selection', () => {
  it('click selects one word, shift-click extends from the anchor either way', () => {
    expect(selectedRange(select(loaded, 2).selection)).toEqual([2, 2]);
    expect(selectedRange(select(loaded, 1, 3).selection)).toEqual([1, 3]);
    expect(selectedRange(select(loaded, 3, 1).selection)).toEqual([1, 3]);
  });

  it('ignores out-of-range indices', () => {
    expect(editorReducer(loaded, { type: 'select', index: 9, extend: false }).selection).toBeNull();
  });
});

describe('move', () => {
  const move = (state: EditorState, delta: -1 | 1, extend = false, skipCut = false) =>
    editorReducer(state, { type: 'move', delta, extend, skipCut });

  it('steps a single-word selection left and right, clamped to the words', () => {
    expect(selectedRange(move(select(loaded, 1), 1).selection)).toEqual([2, 2]);
    expect(selectedRange(move(select(loaded, 1), -1).selection)).toEqual([0, 0]);
    const first = select(loaded, 0);
    const last = select(loaded, 3);
    expect(move(first, -1)).toBe(first);
    expect(move(last, 1)).toBe(last);
  });

  it('collapses a run to the edge it moves from', () => {
    expect(selectedRange(move(select(loaded, 1, 2), 1).selection)).toEqual([3, 3]);
    expect(selectedRange(move(select(loaded, 2, 1), -1).selection)).toEqual([0, 0]);
  });

  it('extends from the focus with shift, keeping the anchor', () => {
    const extended = move(select(loaded, 1), 1, true);
    expect(extended.selection).toEqual({ anchor: 1, focus: 2 });
    expect(selectedRange(move(extended, -1, true).selection)).toEqual([1, 1]);
    expect(selectedRange(move(move(extended, -1, true), -1, true).selection)).toEqual([0, 1]);
  });

  it('starts at the first or last word when nothing is selected', () => {
    expect(selectedRange(move(loaded, 1).selection)).toEqual([0, 0]);
    expect(selectedRange(move(loaded, -1).selection)).toEqual([3, 3]);
    expect(move(initialEditor, 1)).toBe(initialEditor);
  });

  it('skips cut words when asked, as when cuts are hidden', () => {
    const cutMiddle = editorReducer(select(loaded, 1, 2), { type: 'deleteSelection' });
    expect(selectedRange(move(select(cutMiddle, 0), 1, false, true).selection)).toEqual([3, 3]);
    expect(selectedRange(move(select(cutMiddle, 0), 1).selection)).toEqual([1, 1]);
    const beforeCutTail = select(editorReducer(select(loaded, 3), { type: 'deleteSelection' }), 2);
    expect(move(beforeCutTail, 1, false, true)).toBe(beforeCutTail);
  });
});

describe('deleteSelection', () => {
  it('cuts from the first word to the next word start and clears the selection', () => {
    const state = editorReducer(select(loaded, 1, 2), { type: 'deleteSelection' });
    expect(state.edits).toEqual([{ kind: 'cut', start: 0.91, end: 2.0 }]);
    expect(state.selection).toBeNull();
  });

  it('cuts to the end of the media for the last word', () => {
    const state = editorReducer(select(loaded, 3), { type: 'deleteSelection' });
    expect(state.edits).toEqual([{ kind: 'cut', start: 2.0, end: 20 }]);
  });

  it('does nothing without a selection', () => {
    expect(editorReducer(loaded, { type: 'deleteSelection' })).toBe(loaded);
  });

  it('drops an overdub that the deleted passage swallows', () => {
    const dubbed = editorReducer(select(loaded, 1), {
      type: 'overdub',
      text: 'to',
      audioUrl: '/data/x/overdub-0.wav',
      audioDuration: 0.4,
    });
    const state = editorReducer(select(dubbed, 0, 2), { type: 'deleteSelection' });
    expect(state.edits).toEqual([{ kind: 'cut', start: 0, end: 2.0 }]);
  });
});

describe('applyCuts', () => {
  it('appends the batch as one edit list and clears the selection', () => {
    const cuts = [
      { kind: 'cut' as const, start: 0.91, end: 1.25 },
      { kind: 'cut' as const, start: 2.0, end: 20 },
    ];
    const state = editorReducer(select(loaded, 2), { type: 'applyCuts', cuts });
    expect(state.edits).toEqual(cuts);
    expect(state.selection).toBeNull();
  });

  it('is a no-op for an empty batch', () => {
    expect(editorReducer(loaded, { type: 'applyCuts', cuts: [] })).toBe(loaded);
  });

  it('drops an overdub wholly covered by a cut, like a single cut', () => {
    // Start with an overdub
    let state = editorReducer(loaded, {
      type: 'sync',
      doc: {
        headSeq: 1,
        edits: [
          {
            kind: 'overdub' as const,
            start: 2,
            end: 3,
            text: 'x',
            audioUrl: '/data/m/od.wav',
            audioDuration: 1,
          },
        ],
        speakerNames: [],
        undoable: null,
        redoable: null,
      },
    });
    expect(state.edits).toHaveLength(1);
    // Apply a cut that wholly covers the overdub
    state = editorReducer(state, {
      type: 'applyCuts',
      cuts: [{ kind: 'cut' as const, start: 1, end: 4 }],
    });
    expect(state.edits).toEqual([{ kind: 'cut' as const, start: 1, end: 4 }]);
    // Apply a cut that only partly covers an overdub
    state = editorReducer(loaded, {
      type: 'sync',
      doc: {
        headSeq: 1,
        edits: [
          {
            kind: 'overdub' as const,
            start: 2,
            end: 3,
            text: 'x',
            audioUrl: '/data/m/od.wav',
            audioDuration: 1,
          },
        ],
        speakerNames: [],
        undoable: null,
        redoable: null,
      },
    });
    state = editorReducer(state, {
      type: 'applyCuts',
      cuts: [{ kind: 'cut' as const, start: 2.5, end: 4 }],
    });
    expect(state.edits.some((e) => e.kind === 'overdub')).toBe(true);
  });
});

describe('overdub', () => {
  it('adds an overdub over the selected words and replaces overlapping ones', () => {
    const first = editorReducer(select(loaded, 0, 1), {
      type: 'overdub',
      text: 'grateful to',
      audioUrl: '/data/x/overdub-0.wav',
      audioDuration: 1.1,
    });
    expect(first.edits).toEqual([
      {
        kind: 'overdub',
        start: 0,
        end: 1.25,
        text: 'grateful to',
        audioUrl: '/data/x/overdub-0.wav',
        audioDuration: 1.1,
      },
    ]);

    const second = editorReducer(select(first, 1, 2), {
      type: 'overdub',
      text: 'with you',
      audioUrl: '/data/x/overdub-1.wav',
      audioDuration: 0.9,
    });
    expect(second.edits.map((e) => (e.kind === 'overdub' ? e.text : e.kind))).toEqual(['with you']);
  });
});

describe('sync', () => {
  it('replaces edits and metadata with the server document and clears the selection', () => {
    const state = editorReducer(select(loaded, 1), {
      type: 'sync',
      doc: {
        headSeq: 4,
        edits: [{ kind: 'cut', start: 1, end: 2 }],
        speakerNames: ['Ada'],
        undoable: 4,
        redoable: null,
      },
    });
    expect(state.edits).toEqual([{ kind: 'cut', start: 1, end: 2 }]);
    expect(state.headSeq).toBe(4);
    expect(state.speakerNames).toEqual(['Ada']);
    expect(state.undoable).toBe(4);
    expect(state.selection).toBeNull();
  });

  it('keeps a newer remote fold when a slower reply arrives, but takes its undo targets', () => {
    const remote8 = editorReducer(loaded, {
      type: 'remote',
      headSeq: 8,
      edits: [{ kind: 'cut', start: 5, end: 6 }],
      speakerNames: ['Ada'],
    });
    const state = editorReducer(remote8, {
      type: 'sync',
      doc: {
        headSeq: 7,
        edits: [],
        speakerNames: [],
        undoable: 7,
        redoable: 6,
      },
    });
    expect(state.headSeq).toBe(8);
    expect(state.edits).toEqual([{ kind: 'cut', start: 5, end: 6 }]);
    expect(state.speakerNames).toEqual(['Ada']);
    expect(state.undoable).toBe(7);
    expect(state.redoable).toBe(6);
  });

  it('applies a reply whose fold is level with what it has', () => {
    const remote8 = editorReducer(select(loaded, 1), {
      type: 'remote',
      headSeq: 8,
      edits: [{ kind: 'cut', start: 5, end: 6 }],
      speakerNames: ['Ada'],
    });
    const state = editorReducer(remote8, {
      type: 'sync',
      doc: { headSeq: 8, edits: [], speakerNames: [], undoable: 8, redoable: null },
    });
    expect(state.headSeq).toBe(8);
    expect(state.edits).toEqual([]);
    expect(state.speakerNames).toEqual([]);
    expect(state.selection).toBeNull();
  });

  it('renameSpeaker updates names optimistically', () => {
    const state = editorReducer(loaded, { type: 'renameSpeaker', speaker: 2, name: 'Bob' });
    expect(state.speakerNames).toEqual(['', '', 'Bob']);
  });
});

describe('remote', () => {
  // Selection is applied after the sync, because `sync` clears it: what this
  // block is about is that a *remote* fold leaves the local selection alone.
  const synced = editorReducer(loaded, {
    type: 'sync',
    doc: { headSeq: 3, edits: [], speakerNames: [], undoable: 3, redoable: null },
  });
  const remote = (headSeq: number) =>
    editorReducer(select(synced, 1), {
      type: 'remote',
      headSeq,
      edits: [{ kind: 'cut', start: 1, end: 2 }],
      speakerNames: ['Ada'],
    });

  it('applies a newer fold, keeps the selection and the undo targets', () => {
    const state = remote(4);
    expect(state.headSeq).toBe(4);
    expect(state.edits).toHaveLength(1);
    expect(state.speakerNames).toEqual(['Ada']);
    expect(state.undoable).toBe(3);
    expect(state.selection).toEqual({ anchor: 1, focus: 1 });
  });

  it('ignores a fold that is not newer than what it has', () => {
    expect(remote(3).edits).toEqual([]);
    expect(remote(2).headSeq).toBe(3);
  });
});

describe('titles, captions and transitions', () => {
  const t = { at: 1.25, duration: 3, text: 'Intro', subtitle: null, style: 'dark' as const };
  it('adds, edits and removes a title', () => {
    let s = editorReducer(loaded, { type: 'addTitle', ...t });
    expect(s.edits).toEqual([{ kind: 'title', ...t }]);
    s = editorReducer(s, { type: 'editTitle', ...t, text: 'Part 2', style: 'accent' });
    expect(s.edits[0]).toMatchObject({ text: 'Part 2', style: 'accent' });
    s = editorReducer(s, { type: 'removeTitle', at: 1.25 });
    expect(s.edits).toEqual([]);
  });
  it('edits only the first title at an instant, as the engine fold does', () => {
    let s = editorReducer(loaded, { type: 'addTitle', ...t });
    s = editorReducer(s, { type: 'addTitle', ...t, text: 'Second' });
    s = editorReducer(s, { type: 'editTitle', ...t, text: 'Only me' });
    expect(s.edits.map((e) => (e.kind === 'title' ? e.text : e.kind))).toEqual([
      'Only me',
      'Second',
    ]);
  });
  it('captions cover the selection and replace overlapping ones', () => {
    let s = editorReducer(select(loaded, 1, 2), {
      type: 'addCaption',
      text: 'A',
      position: 'bottomLeft',
    });
    s = editorReducer(select(s, 2, 3), { type: 'addCaption', text: 'B', position: 'bottomLeft' });
    expect(s.edits).toEqual([
      { kind: 'caption', start: 1.25, end: 20, text: 'B', position: 'bottomLeft' },
    ]);
    s = editorReducer(s, { type: 'removeCaption', start: 1.25 });
    expect(s.edits).toEqual([]);
  });
  it('captions an explicit range even with no selection left', () => {
    const s = editorReducer(loaded, {
      type: 'addCaption',
      text: 'A',
      position: 'bottomLeft',
      range: [1, 2],
    });
    expect(s.edits).toEqual([
      { kind: 'caption', start: 0.91, end: 2.0, text: 'A', position: 'bottomLeft' },
    ]);
  });
  it('sets the project transition and a per-cut override, and sync/remote carry it', () => {
    let s = editorReducer(loaded, { type: 'setTransition', transition: 'dip' });
    expect(s.transition).toBe('dip');
    s = editorReducer(select(s, 0), { type: 'deleteSelection' });
    s = editorReducer(s, { type: 'setCutTransition', start: 0, transition: 'none' });
    expect(s.edits[0]).toMatchObject({ kind: 'cut', transition: 'none' });
    s = editorReducer(s, {
      type: 'remote',
      headSeq: 9,
      edits: [],
      speakerNames: [],
      transition: 'none',
    });
    expect(s.transition).toBe('none');
  });
});

describe('clips', () => {
  it('split keeps splits sorted and unique; unsplit removes', () => {
    let s = editorReducer(loaded, { type: 'split', at: 5 });
    s = editorReducer(s, { type: 'split', at: 2 });
    s = editorReducer(s, { type: 'split', at: 5 });
    expect(s.splits).toEqual([2, 5]);
    expect(editorReducer(s, { type: 'unsplit', at: 5 }).splits).toEqual([2]);
  });

  it('moveClip materialises the order like the engine', () => {
    let s = editorReducer(loaded, { type: 'split', at: 2 });
    s = editorReducer(s, { type: 'split', at: 5 });
    expect(editorReducer(s, { type: 'moveClip', piece: 5, before: 0 }).order).toEqual([5, 0, 2]);
    expect(editorReducer(s, { type: 'moveClip', piece: 0, before: null }).order).toEqual([2, 5, 0]);
    expect(editorReducer(s, { type: 'moveClip', piece: 9, before: null }).order).toEqual([]);
  });

  it('sync and remote carry splits and order, defaulting to empty', () => {
    const doc = {
      headSeq: 3,
      edits: [],
      speakerNames: [],
      undoable: null,
      redoable: null,
      splits: [2],
      order: [2, 0],
    };
    const s = editorReducer(loaded, { type: 'sync', doc });
    expect(s.splits).toEqual([2]);
    expect(s.order).toEqual([2, 0]);
    const r = editorReducer(s, { type: 'remote', headSeq: 4, edits: [], speakerNames: [] });
    expect(r.splits).toEqual([]);
  });

  it('audio edits by start; whole-edit range covers every word', () => {
    let s = editorReducer(loaded, {
      type: 'addAudio',
      media: 'm',
      gain: -6,
      duck: true,
      range: null,
    });
    expect(s.edits[0]).toEqual({
      kind: 'audio',
      start: 0,
      end: 20,
      media: 'm',
      offset: 0,
      gain: -6,
      duck: true,
    });
    s = editorReducer(s, { type: 'editAudio', start: 0, gain: 3, duck: false });
    expect(s.edits[0]).toMatchObject({ gain: 3, duck: false });
    s = editorReducer(s, { type: 'removeAudio', start: 0 });
    expect(s.edits).toEqual([]);
  });
});

describe('layers', () => {
  it('addLayer replaces the layers it overlaps on its own track only', () => {
    let s = editorReducer(select(loaded, 0, 1), {
      type: 'addLayer',
      track: 2,
      media: 'b',
      offset: 1,
      frame: 'full',
      audio: null,
    });
    s = editorReducer(select(s, 1, 2), {
      type: 'addLayer',
      track: 3,
      media: 'c',
      offset: 0,
      frame: 'pipTopRight',
      audio: -6,
    });
    s = editorReducer(select(s, 1, 2), {
      type: 'addLayer',
      track: 2,
      media: 'b',
      offset: 0,
      frame: 'full',
      audio: null,
    });
    expect(s.edits).toEqual([
      {
        kind: 'layer',
        track: 3,
        start: 0.91,
        end: 2,
        media: 'c',
        offset: 0,
        frame: 'pipTopRight',
        audio: -6,
      },
      {
        kind: 'layer',
        track: 2,
        start: 0.91,
        end: 2,
        media: 'b',
        offset: 0,
        frame: 'full',
        audio: null,
      },
    ]);
  });

  it('setLayer edits in place and drops what it now overlaps on the new track; removeLayer by track and start', () => {
    let s = editorReducer(select(loaded, 1, 2), {
      type: 'addLayer',
      track: 3,
      media: 'c',
      offset: 0,
      frame: 'pipTopRight',
      audio: -6,
    });
    s = editorReducer(select(s, 1, 2), {
      type: 'addLayer',
      track: 2,
      media: 'b',
      offset: 0,
      frame: 'full',
      audio: null,
    });
    s = editorReducer(s, {
      type: 'setLayer',
      track: 3,
      start: 0.91,
      toTrack: 2,
      frame: 'pipBottomLeft',
      audio: 0,
    });
    expect(s.edits).toEqual([
      {
        kind: 'layer',
        track: 2,
        start: 0.91,
        end: 2,
        media: 'c',
        offset: 0,
        frame: 'pipBottomLeft',
        audio: 0,
      },
    ]);
    // Wrong track: nothing matches.
    expect(editorReducer(s, { type: 'removeLayer', track: 3, start: 0.91 }).edits).toHaveLength(1);
    expect(editorReducer(s, { type: 'removeLayer', track: 2, start: 0.91 }).edits).toEqual([]);
  });
});

describe('sources', () => {
  const first = editorReducer(initialEditor, { type: 'load', words, duration: 20, media: 'm0' });

  it("starts with the project's own media as the only source", () => {
    expect(first.sources).toEqual([{ media: 'm0', offset: 0, duration: 20 }]);
    expect(first.duration).toBe(20);
    expect(editorReducer(initialEditor, { type: 'load', words: [], duration: 0 }).sources).toEqual(
      [],
    );
  });

  it('addSource appends at the end with its join, and the duration is stitched', () => {
    const s = editorReducer(first, { type: 'addSource', media: 'm1', offset: 20, duration: 12.5 });
    expect(s.sources).toEqual([
      { media: 'm0', offset: 0, duration: 20 },
      { media: 'm1', offset: 20, duration: 12.5 },
    ]);
    expect(s.duration).toBe(32.5);
    expect(s.splits).toEqual([20]);
    // The server's broadcast of the same append can beat the upload's reply.
    expect(editorReducer(s, { type: 'addSource', media: 'm1', offset: 20, duration: 12.5 })).toBe(
      s,
    );
  });

  it("takes the fold's appended sources on sync, and drops them on its undo", () => {
    const doc: DocState = {
      headSeq: 2,
      edits: [],
      speakerNames: [],
      undoable: 2,
      redoable: null,
      splits: [3, 20],
      sources: [{ media: 'm1', offset: 20, duration: 10 }],
    };
    const s = editorReducer(first, { type: 'sync', doc });
    expect(s.sources.map((x) => x.media)).toEqual(['m0', 'm1']);
    expect(s.duration).toBe(30);
    expect(s.splits).toEqual([3, 20]);
    const undone = editorReducer(s, {
      type: 'sync',
      doc: { ...doc, headSeq: 3, sources: [], splits: [3] },
    });
    expect(undone.sources).toEqual([{ media: 'm0', offset: 0, duration: 20 }]);
    expect(undone.duration).toBe(20);
  });

  it("reads a peer's fold the same way, and an older server's fold as one source", () => {
    const r = editorReducer(first, {
      type: 'remote',
      headSeq: 5,
      edits: [],
      speakerNames: [],
      splits: [20],
      sources: [{ media: 'm1', offset: 20, duration: 4 }],
    });
    expect(r.duration).toBe(24);
    const old = editorReducer(first, { type: 'remote', headSeq: 6, edits: [], speakerNames: [] });
    expect(old.sources).toHaveLength(1);
    expect(old.duration).toBe(20);
  });

  it('never unsplits a join', () => {
    const s = editorReducer(first, { type: 'addSource', media: 'm1', offset: 20, duration: 5 });
    expect(editorReducer(s, { type: 'unsplit', at: 20 })).toBe(s);
  });

  it('setWords swaps in the stitched words and drops the selection', () => {
    const more = [...words, { id: '1:w0', text: 'later', start: 20.5, end: 21 }];
    const s = editorReducer(select(first, 1), { type: 'setWords', words: more });
    expect(s.words).toBe(more);
    expect(s.selection).toBeNull();
    expect(s.sources).toBe(first.sources);
  });
});

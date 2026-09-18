import { describe, expect, it } from 'vitest';

import { editorReducer, initialEditor, selectedRange, type EditorState } from './editor';
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

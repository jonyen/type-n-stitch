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
  it('appends the batch as a single undo step and clears the selection', () => {
    const cuts = [
      { kind: 'cut' as const, start: 0.91, end: 1.25 },
      { kind: 'cut' as const, start: 2.0, end: 20 },
    ];
    const state = editorReducer(select(loaded, 2), { type: 'applyCuts', cuts });
    expect(state.edits).toEqual(cuts);
    expect(state.selection).toBeNull();
    expect(state.past).toHaveLength(1);
    expect(editorReducer(state, { type: 'undo' }).edits).toEqual([]);
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

describe('undo', () => {
  it('restores the previous edit list step by step', () => {
    let state = editorReducer(select(loaded, 0), { type: 'deleteSelection' });
    state = editorReducer(select(state, 3), { type: 'deleteSelection' });
    expect(state.edits).toHaveLength(2);

    state = editorReducer(state, { type: 'undo' });
    expect(state.edits).toEqual([{ kind: 'cut', start: 0, end: 0.91 }]);
    state = editorReducer(state, { type: 'undo' });
    expect(state.edits).toEqual([]);
    expect(editorReducer(state, { type: 'undo' })).toBe(state);
  });
});

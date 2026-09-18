import { describe, expect, it } from 'vitest';

import { editorReducer, initialEditor, type EditorState } from './editor';
import { opForAction } from './ops';
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

describe('opForAction', () => {
  it('maps deleteSelection to a cut owning the trailing gap', () => {
    expect(opForAction(select(loaded, 1, 2), { type: 'deleteSelection' })).toEqual({
      kind: 'cut',
      start: 0.91,
      end: 2.0,
    });
  });

  it('returns null with no selection or for selection-only actions', () => {
    expect(opForAction(loaded, { type: 'deleteSelection' })).toBeNull();
    expect(opForAction(loaded, { type: 'select', index: 0, extend: false })).toBeNull();
    expect(opForAction(loaded, { type: 'clearSelection' })).toBeNull();
  });

  it('maps overdub to an overdub op over the selection', () => {
    const op = opForAction(select(loaded, 3), {
      type: 'overdub',
      text: 'hi',
      audioUrl: '/data/m/overdub-0.wav',
      audioDuration: 0.4,
    });
    expect(op).toEqual({
      kind: 'overdub',
      start: 2.0,
      end: 20,
      text: 'hi',
      audioUrl: '/data/m/overdub-0.wav',
      audioDuration: 0.4,
    });
  });

  it('maps applyCuts to applycuts with bare ranges', () => {
    expect(
      opForAction(loaded, {
        type: 'applyCuts',
        cuts: [
          { kind: 'cut', start: 1, end: 2 },
          { kind: 'cut', start: 3, end: 4 },
        ],
      }),
    ).toEqual({
      kind: 'applycuts',
      cuts: [
        { start: 1, end: 2 },
        { start: 3, end: 4 },
      ],
    });
    expect(opForAction(loaded, { type: 'applyCuts', cuts: [] })).toBeNull();
  });

  it('maps renameSpeaker', () => {
    expect(opForAction(loaded, { type: 'renameSpeaker', speaker: 1, name: ' Ada ' })).toEqual({
      kind: 'renamespeaker',
      speaker: 1,
      name: 'Ada',
    });
  });
});

describe('titles, captions and transitions', () => {
  it('maps the title, caption and transition actions', () => {
    const t = { at: 1.25, duration: 3, text: 'Intro', subtitle: null, style: 'dark' as const };
    expect(opForAction(loaded, { type: 'addTitle', ...t })).toEqual({ kind: 'addtitle', ...t });
    expect(opForAction(loaded, { type: 'editTitle', ...t, text: 'X' })).toEqual({
      kind: 'edittitle',
      ...t,
      text: 'X',
    });
    expect(opForAction(loaded, { type: 'removeTitle', at: 1.25 })).toEqual({
      kind: 'removetitle',
      at: 1.25,
    });
    expect(
      opForAction(select(loaded, 1, 2), { type: 'addCaption', text: 'Ada', position: 'topLeft' }),
    ).toEqual({
      kind: 'addcaption',
      start: 0.91,
      end: 2.0,
      text: 'Ada',
      position: 'topLeft',
    });
    expect(
      opForAction(loaded, { type: 'addCaption', text: 'Ada', position: 'topLeft' }),
    ).toBeNull();
    // A range captured when the dialog opened survives a cleared selection.
    expect(
      opForAction(loaded, {
        type: 'addCaption',
        text: 'Ada',
        position: 'topLeft',
        range: [1, 2],
      }),
    ).toEqual({
      kind: 'addcaption',
      start: 0.91,
      end: 2.0,
      text: 'Ada',
      position: 'topLeft',
    });
    expect(opForAction(loaded, { type: 'removeCaption', start: 0.91 })).toEqual({
      kind: 'removecaption',
      start: 0.91,
    });
    expect(opForAction(loaded, { type: 'setTransition', transition: 'dip' })).toEqual({
      kind: 'settransition',
      transition: 'dip',
    });
    expect(opForAction(loaded, { type: 'setCutTransition', start: 1, transition: null })).toEqual({
      kind: 'setcuttransition',
      start: 1,
      transition: null,
    });
  });
});

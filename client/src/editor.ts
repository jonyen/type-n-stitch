// The editor's state machine: words, the edit list, an undo stack and the
// current word selection. Pure, so it is easy to test.

import { rangeForWords, wordStatus } from './editlist';
import type { CutEdit, Edit, OverdubEdit, Word } from './types';

export interface Selection {
  /** Where the selection started (click). */
  anchor: number;
  /** Where it currently ends (shift-click). */
  focus: number;
}

export interface EditorState {
  words: Word[];
  duration: number;
  edits: Edit[];
  /** Previous edit lists, newest last. */
  past: Edit[][];
  selection: Selection | null;
}

export type EditorAction =
  | { type: 'load'; words: Word[]; duration: number }
  | { type: 'select'; index: number; extend: boolean }
  /** Arrow keys: step the selection one word; `extend` keeps the anchor (Shift). */
  | { type: 'move'; delta: -1 | 1; extend: boolean; skipCut?: boolean }
  | { type: 'clearSelection' }
  | { type: 'deleteSelection' }
  | { type: 'overdub'; text: string; audioUrl: string; audioDuration: number }
  /** Append a batch of cuts (filler removal, pause tightening) as one undo step. */
  | { type: 'applyCuts'; cuts: CutEdit[] }
  | { type: 'undo' };

export const initialEditor: EditorState = {
  words: [],
  duration: 0,
  edits: [],
  past: [],
  selection: null,
};

/** The selection as an inclusive, ascending index pair. */
export function selectedRange(selection: Selection | null): [number, number] | null {
  if (!selection) return null;
  return [Math.min(selection.anchor, selection.focus), Math.max(selection.anchor, selection.focus)];
}

function withEdits(state: EditorState, edits: Edit[]): EditorState {
  return { ...state, edits, past: [...state.past, state.edits], selection: null };
}

function inside(inner: OverdubEdit, outer: { start: number; end: number }): boolean {
  return inner.start >= outer.start && inner.end <= outer.end;
}

export function editorReducer(state: EditorState, action: EditorAction): EditorState {
  switch (action.type) {
    case 'load':
      return { ...initialEditor, words: action.words, duration: action.duration };

    case 'select': {
      if (action.index < 0 || action.index >= state.words.length) return state;
      const anchor = action.extend && state.selection ? state.selection.anchor : action.index;
      return { ...state, selection: { anchor, focus: action.index } };
    }

    case 'move': {
      const last = state.words.length - 1;
      if (last < 0) return state;
      const isCut = (i: number) => {
        const word = state.words[i];
        return (
          action.skipCut === true && word !== undefined && wordStatus(word, state.edits) === 'cut'
        );
      };
      const sel = state.selection;
      let index: number;
      if (!sel) {
        index = action.delta > 0 ? 0 : last;
        while (index >= 0 && index <= last && isCut(index)) index += action.delta;
        if (index < 0 || index > last) return state;
        return { ...state, selection: { anchor: index, focus: index } };
      }
      // Collapse to the edge the caret moves from, or extend from the focus.
      const from = action.extend
        ? sel.focus
        : action.delta > 0
          ? Math.max(sel.anchor, sel.focus)
          : Math.min(sel.anchor, sel.focus);
      index = from + action.delta;
      while (index >= 0 && index <= last && isCut(index)) index += action.delta;
      if (index < 0 || index > last) return state;
      return {
        ...state,
        selection: { anchor: action.extend ? sel.anchor : index, focus: index },
      };
    }

    case 'clearSelection':
      return state.selection ? { ...state, selection: null } : state;

    case 'deleteSelection': {
      const range = selectedRange(state.selection);
      if (!range) return state;
      const cut = rangeForWords(state.words, range[0], range[1], state.duration);
      // Deleting an overdubbed passage removes the overdub with it.
      const kept = state.edits.filter((e) => !(e.kind === 'overdub' && inside(e, cut)));
      return withEdits(state, [...kept, { kind: 'cut', ...cut }]);
    }

    case 'overdub': {
      const range = selectedRange(state.selection);
      if (!range) return state;
      const span = rangeForWords(state.words, range[0], range[1], state.duration);
      // A new overdub replaces any it overlaps.
      const kept = state.edits.filter(
        (e) => !(e.kind === 'overdub' && e.start < span.end && e.end > span.start),
      );
      const edit: OverdubEdit = {
        kind: 'overdub',
        ...span,
        text: action.text,
        audioUrl: action.audioUrl,
        audioDuration: action.audioDuration,
      };
      return withEdits(state, [...kept, edit]);
    }

    case 'applyCuts': {
      if (action.cuts.length === 0) return state;
      return withEdits(state, [...state.edits, ...action.cuts]);
    }

    case 'undo': {
      const previous = state.past[state.past.length - 1];
      if (!previous) return state;
      return { ...state, edits: previous, past: state.past.slice(0, -1), selection: null };
    }
  }
}

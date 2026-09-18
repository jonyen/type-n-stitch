// The editor's state machine: words, the edit list, server-fold metadata and
// the current word selection. Pure, so it is easy to test.

import { EPS, rangeForWords, wordStatus } from './editlist';
import type { DocState } from './ops';
import type {
  CaptionEdit,
  CaptionPos,
  CutEdit,
  Edit,
  OverdubEdit,
  TitleEdit,
  TitleStyle,
  Transition,
  Word,
} from './types';

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
  /** Display names by speaker index; '' means unnamed. */
  speakerNames: string[];
  /** How pieces of the output meet, unless a cut overrides it. */
  transition: Transition;
  /** Sequence number of the last server operation folded into `edits`. */
  headSeq: number;
  /** This user's next undo / redo target on the server, if any. */
  undoable: number | null;
  redoable: number | null;
  selection: Selection | null;
}

export type EditorAction =
  | { type: 'load'; words: Word[]; duration: number }
  /** The server's authoritative document replaces local edits. */
  | { type: 'sync'; doc: DocState }
  | { type: 'select'; index: number; extend: boolean }
  /** Arrow keys: step the selection one word; `extend` keeps the anchor (Shift). */
  | { type: 'move'; delta: -1 | 1; extend: boolean; skipCut?: boolean }
  | { type: 'clearSelection' }
  | { type: 'deleteSelection' }
  | { type: 'overdub'; text: string; audioUrl: string; audioDuration: number }
  /** Append a batch of cuts (filler removal, pause tightening) as one edit list. */
  | { type: 'applyCuts'; cuts: CutEdit[] }
  | { type: 'renameSpeaker'; speaker: number; name: string }
  | {
      type: 'addTitle';
      at: number;
      duration: number;
      text: string;
      subtitle: string | null;
      style: TitleStyle;
    }
  /** Replace the title at `at`; a no-op when there is none. */
  | {
      type: 'editTitle';
      at: number;
      duration: number;
      text: string;
      subtitle: string | null;
      style: TitleStyle;
    }
  | { type: 'removeTitle'; at: number }
  /** `range` is the word range captured when the dialog opened, if any. */
  | { type: 'addCaption'; text: string; position: CaptionPos; range?: [number, number] }
  | { type: 'removeCaption'; start: number }
  | { type: 'setTransition'; transition: Transition }
  | { type: 'setCutTransition'; start: number; transition: Transition | null }
  /** Another collaborator's append, as the server's fold. Per-user fields stay. */
  | {
      type: 'remote';
      headSeq: number;
      edits: Edit[];
      speakerNames: string[];
      transition?: Transition;
    };

export const initialEditor: EditorState = {
  words: [],
  duration: 0,
  edits: [],
  speakerNames: [],
  transition: 'none',
  headSeq: 0,
  undoable: null,
  redoable: null,
  selection: null,
};

/** The selection as an inclusive, ascending index pair. */
export function selectedRange(selection: Selection | null): [number, number] | null {
  if (!selection) return null;
  return [Math.min(selection.anchor, selection.focus), Math.max(selection.anchor, selection.focus)];
}

function withEdits(state: EditorState, edits: Edit[]): EditorState {
  return { ...state, edits, selection: null };
}

function inside(inner: OverdubEdit, outer: { start: number; end: number }): boolean {
  return inner.start >= outer.start && inner.end <= outer.end;
}

function sameInstant(a: number, b: number): boolean {
  return Math.abs(a - b) < EPS;
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

    case 'sync': {
      // The undo targets are ours alone, so a reply always brings them. Its
      // fold, though, can be older than one a peer's broadcast already
      // applied — a slower reply must not undo newer server truth.
      const stale = action.doc.headSeq < state.headSeq;
      return {
        ...state,
        edits: stale ? state.edits : action.doc.edits,
        speakerNames: stale ? state.speakerNames : action.doc.speakerNames,
        headSeq: stale ? state.headSeq : action.doc.headSeq,
        transition: stale ? state.transition : (action.doc.transition ?? 'none'),
        undoable: action.doc.undoable,
        redoable: action.doc.redoable,
        selection: stale ? state.selection : null,
      };
    }

    case 'remote':
      // Out-of-order broadcasts, and our own append arriving before its POST
      // reply, are both harmless as long as only newer folds win.
      if (action.headSeq <= state.headSeq) return state;
      return {
        ...state,
        edits: action.edits,
        speakerNames: action.speakerNames,
        transition: action.transition ?? 'none',
        headSeq: action.headSeq,
      };

    case 'renameSpeaker': {
      const speakerNames = [...state.speakerNames];
      while (speakerNames.length <= action.speaker) speakerNames.push('');
      speakerNames[action.speaker] = action.name.trim();
      return { ...state, speakerNames };
    }

    case 'addTitle': {
      const title: TitleEdit = {
        kind: 'title',
        at: action.at,
        duration: action.duration,
        text: action.text,
        subtitle: action.subtitle,
        style: action.style,
      };
      return { ...state, edits: [...state.edits, title] };
    }

    case 'editTitle': {
      // The engine's fold edits the first title at the instant and no others.
      const at = state.edits.findIndex((e) => e.kind === 'title' && sameInstant(e.at, action.at));
      const found = state.edits[at];
      if (at === -1 || found?.kind !== 'title') return state;
      const edits = [...state.edits];
      edits[at] = {
        ...found,
        duration: action.duration,
        text: action.text,
        subtitle: action.subtitle,
        style: action.style,
      };
      return { ...state, edits };
    }

    case 'removeTitle':
      return {
        ...state,
        edits: state.edits.filter((e) => !(e.kind === 'title' && sameInstant(e.at, action.at))),
      };

    case 'addCaption': {
      const range = action.range ?? selectedRange(state.selection);
      if (!range) return state;
      const span = rangeForWords(state.words, range[0], range[1], state.duration);
      // A new caption replaces any it overlaps, like an overdub.
      const kept = state.edits.filter(
        (e) => !(e.kind === 'caption' && e.start < span.end && e.end > span.start),
      );
      const caption: CaptionEdit = {
        kind: 'caption',
        ...span,
        text: action.text,
        position: action.position,
      };
      return withEdits(state, [...kept, caption]);
    }

    case 'removeCaption':
      return {
        ...state,
        edits: state.edits.filter(
          (e) => !(e.kind === 'caption' && sameInstant(e.start, action.start)),
        ),
      };

    case 'setTransition':
      return { ...state, transition: action.transition };

    case 'setCutTransition': {
      const edits = state.edits.map((e) => {
        if (e.kind !== 'cut' || !sameInstant(e.start, action.start)) return e;
        // A null override drops the key, as the engine's `None` does on the wire.
        const cut: CutEdit = { kind: 'cut', start: e.start, end: e.end };
        return action.transition === null ? cut : { ...cut, transition: action.transition };
      });
      return { ...state, edits };
    }
  }
}

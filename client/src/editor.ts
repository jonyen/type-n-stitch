// The editor's state machine: words, the edit list, server-fold metadata and
// the current word selection. Pure, so it is easy to test.

import { EPS, isJoin, pieceStarts, rangeForWords, stitchedDuration, wordStatus } from './editlist';
import type { DocState } from './ops';
import type {
  AudioEdit,
  CaptionEdit,
  CaptionPos,
  CutEdit,
  Edit,
  Frame,
  LayerEdit,
  LayerTrack,
  OverdubEdit,
  Source,
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
  /** Stitched length of every source on the main track, in seconds. */
  duration: number;
  /**
   * The main track's files in stitched order: the project's own media, then
   * the fold's appended sources. Empty before a project loads.
   */
  sources: Source[];
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
  /** Extra piece boundaries outside any cut, sorted ascending. */
  splits: number[];
  /** Explicit output order of piece starts; empty means source order. */
  order: number[];
}

export type EditorAction =
  /** `media` is the project's own media id: source 0. */
  | { type: 'load'; words: Word[]; duration: number; media?: string }
  /** The stitched words again, once another source's transcript is ready. */
  | { type: 'setWords'; words: Word[] }
  /** The server's authoritative document replaces local edits. */
  | { type: 'sync'; doc: DocState }
  | { type: 'select'; index: number; extend: boolean }
  /** Arrow keys: step the selection one word; `extend` keeps the anchor (Shift). */
  | { type: 'move'; delta: -1 | 1; extend: boolean; skipCut?: boolean }
  | { type: 'clearSelection' }
  | { type: 'deleteSelection' }
  /** `range` is the word range captured when the dialog opened, if any. */
  | {
      type: 'overdub';
      text: string;
      audioUrl: string;
      audioDuration: number;
      range?: [number, number];
    }
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
  | { type: 'split'; at: number }
  | { type: 'unsplit'; at: number }
  | { type: 'moveClip'; piece: number; before: number | null }
  /** The optimistic echo of the AddSource the upload route appended. Never sent. */
  | { type: 'addSource'; media: string; offset: number; duration: number }
  /** `range` is the word range captured when the dialog opened; else the selection. */
  | {
      type: 'addLayer';
      track: LayerTrack;
      media: string;
      offset: number;
      frame: Frame;
      audio: number | null;
      range?: [number, number];
    }
  | {
      type: 'setLayer';
      track: LayerTrack;
      start: number;
      toTrack: LayerTrack;
      frame: Frame;
      audio: number | null;
    }
  | { type: 'removeLayer'; track: LayerTrack; start: number }
  /** `range` `null` means the whole edit; `undefined` means the current selection. */
  | {
      type: 'addAudio';
      media: string;
      gain: number;
      duck: boolean;
      range?: [number, number] | null;
    }
  | { type: 'editAudio'; start: number; gain: number; duck: boolean }
  | { type: 'removeAudio'; start: number }
  /** Another collaborator's append, as the server's fold. Per-user fields stay. */
  | {
      type: 'remote';
      headSeq: number;
      edits: Edit[];
      speakerNames: string[];
      transition?: Transition;
      splits?: number[];
      order?: number[];
      sources?: Source[];
    };

export const initialEditor: EditorState = {
  words: [],
  duration: 0,
  sources: [],
  edits: [],
  speakerNames: [],
  transition: 'none',
  headSeq: 0,
  undoable: null,
  redoable: null,
  selection: null,
  splits: [],
  order: [],
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

/** The whole edit as a word range, or null when there are no words yet. */
export function wholeRange(state: EditorState): [number, number] | null {
  return state.words.length ? [0, state.words.length - 1] : null;
}

/** The first source (the project's media) followed by a fold's appended sources. */
function withAppended(state: EditorState, appended: Source[] | undefined): Source[] {
  const first = state.sources[0];
  return first ? [first, ...(appended ?? [])] : [...(appended ?? [])];
}

/** The stitched length of `sources`, or `fallback` when there are none yet. */
function lengthOf(sources: Source[], fallback: number): number {
  return sources.length > 0 ? stitchedDuration(sources) : fallback;
}

export function editorReducer(state: EditorState, action: EditorAction): EditorState {
  switch (action.type) {
    case 'load':
      return {
        ...initialEditor,
        words: action.words,
        duration: action.duration,
        // A named source seeds source 0 even at duration 0 (an unprobed or
        // audio-only file); with no media there is nothing to seed.
        sources:
          action.media !== undefined || action.duration > 0
            ? [{ media: action.media ?? '', offset: 0, duration: action.duration }]
            : [],
      };

    case 'setWords':
      // Indices shift when another source's words arrive, so the selection goes.
      return { ...state, words: action.words, selection: null };

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
      const range = action.range ?? selectedRange(state.selection);
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
      // The same rule as a single cut: an overdub wholly inside a cut goes with it.
      const kept = state.edits.filter(
        (e) => !(e.kind === 'overdub' && action.cuts.some((c) => inside(e, c))),
      );
      return withEdits(state, [...kept, ...action.cuts]);
    }

    case 'sync': {
      // The undo targets are ours alone, so a reply always brings them. Its
      // fold, though, can be older than one a peer's broadcast already
      // applied — a slower reply must not undo newer server truth.
      const stale = action.doc.headSeq < state.headSeq;
      const sources = stale ? state.sources : withAppended(state, action.doc.sources);
      return {
        ...state,
        edits: stale ? state.edits : action.doc.edits,
        speakerNames: stale ? state.speakerNames : action.doc.speakerNames,
        headSeq: stale ? state.headSeq : action.doc.headSeq,
        transition: stale ? state.transition : (action.doc.transition ?? 'none'),
        undoable: action.doc.undoable,
        redoable: action.doc.redoable,
        selection: stale ? state.selection : null,
        splits: stale ? state.splits : (action.doc.splits ?? []),
        order: stale ? state.order : (action.doc.order ?? []),
        sources,
        duration: lengthOf(sources, state.duration),
      };
    }

    case 'remote': {
      // Out-of-order broadcasts, and our own append arriving before its POST
      // reply, are both harmless as long as only newer folds win.
      if (action.headSeq <= state.headSeq) return state;
      const sources = withAppended(state, action.sources);
      return {
        ...state,
        edits: action.edits,
        speakerNames: action.speakerNames,
        transition: action.transition ?? 'none',
        headSeq: action.headSeq,
        splits: action.splits ?? [],
        order: action.order ?? [],
        sources,
        duration: lengthOf(sources, state.duration),
      };
    }

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

    case 'split': {
      if (state.splits.some((s) => sameInstant(s, action.at))) return state;
      return { ...state, splits: [...state.splits, action.at].sort((a, b) => a - b) };
    }

    case 'unsplit':
      // A join between two files is permanent, as in the engine's fold.
      if (isJoin(action.at, state.sources)) return state;
      return { ...state, splits: state.splits.filter((s) => !sameInstant(s, action.at)) };

    case 'moveClip': {
      const current = pieceStarts(state.edits, state.splits);
      const has = (list: number[], x: number) => list.some((s) => sameInstant(s, x));
      if (!has(current, action.piece)) return state;
      const effective: number[] = [];
      for (const s of [...state.order, ...current])
        if (has(current, s) && !has(effective, s)) effective.push(s);
      const rest = effective.filter((s) => !sameInstant(s, action.piece));
      const at =
        action.before === null
          ? -1
          : rest.findIndex((s) => sameInstant(s, action.before as number));
      rest.splice(at === -1 ? rest.length : at, 0, action.piece);
      return { ...state, order: rest };
    }

    case 'addSource': {
      if (state.sources.some((s) => sameInstant(s.offset, action.offset))) return state;
      const sources = [
        ...state.sources,
        { media: action.media, offset: action.offset, duration: action.duration },
      ];
      // Each file starts as its own clip, as the fold's permanent split does.
      const splits = state.splits.some((s) => sameInstant(s, action.offset))
        ? state.splits
        : [...state.splits, action.offset].sort((a, b) => a - b);
      return { ...state, sources, splits, duration: stitchedDuration(sources) };
    }

    case 'addLayer': {
      const range = action.range ?? selectedRange(state.selection);
      if (!range) return state;
      const span = rangeForWords(state.words, range[0], range[1], state.duration);
      // A new layer replaces those it overlaps on its own track, as B-roll did.
      const kept = state.edits.filter(
        (e) =>
          !(
            e.kind === 'layer' &&
            e.track === action.track &&
            e.start < span.end &&
            e.end > span.start
          ),
      );
      const layer: LayerEdit = {
        kind: 'layer',
        track: action.track,
        ...span,
        media: action.media,
        offset: action.offset,
        frame: action.frame,
        audio: action.audio,
      };
      return withEdits(state, [...kept, layer]);
    }

    case 'setLayer': {
      const at = state.edits.findIndex(
        (e) => e.kind === 'layer' && e.track === action.track && sameInstant(e.start, action.start),
      );
      const found = state.edits[at];
      if (at === -1 || found?.kind !== 'layer') return state;
      const moved: LayerEdit = {
        ...found,
        track: action.toTrack,
        frame: action.frame,
        audio: action.audio,
      };
      // In place, as the fold does; a layer it now overlaps on the new track goes.
      const edits = state.edits.flatMap((e, i): Edit[] => {
        if (i === at) return [moved];
        const covered =
          e.kind === 'layer' &&
          e.track === action.toTrack &&
          e.start < moved.end &&
          e.end > moved.start;
        return covered ? [] : [e];
      });
      return { ...state, edits };
    }

    case 'removeLayer':
      return {
        ...state,
        edits: state.edits.filter(
          (e) =>
            !(e.kind === 'layer' && e.track === action.track && sameInstant(e.start, action.start)),
        ),
      };

    case 'addAudio': {
      const range =
        action.range === null
          ? wholeRange(state)
          : (action.range ?? selectedRange(state.selection));
      if (!range) return state;
      const span = rangeForWords(state.words, range[0], range[1], state.duration);
      const audio: AudioEdit = {
        kind: 'audio',
        ...span,
        media: action.media,
        offset: 0,
        gain: action.gain,
        duck: action.duck,
      };
      return withEdits(state, [...state.edits, audio]);
    }

    case 'editAudio':
      return {
        ...state,
        edits: state.edits.map((e) =>
          e.kind === 'audio' && sameInstant(e.start, action.start)
            ? { ...e, gain: action.gain, duck: action.duck }
            : e,
        ),
      };

    case 'removeAudio':
      return {
        ...state,
        edits: state.edits.filter(
          (e) => !(e.kind === 'audio' && sameInstant(e.start, action.start)),
        ),
      };
  }
}

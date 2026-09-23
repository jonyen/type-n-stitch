// Operations: what the client tells the server it did. Mirrors
// engine/src/ops.rs. The reducer applies the same change optimistically;
// the server's fold (a `sync` action) is authoritative.

import { isJoin, rangeForWords } from './editlist';
import type { EditorAction, EditorState } from './editor';
import { selectedRange, wholeRange } from './editor';
import type {
  CaptionPos,
  Edit,
  Frame,
  LayerTrack,
  Range,
  Source,
  TitleStyle,
  Transition,
} from './types';

export type Op =
  | { kind: 'cut'; start: number; end: number }
  | {
      kind: 'overdub';
      start: number;
      end: number;
      text: string;
      audioUrl: string;
      audioDuration: number;
    }
  | { kind: 'applycuts'; cuts: Range[] }
  | { kind: 'renamespeaker'; speaker: number; name: string }
  | { kind: 'undo'; targetSeq: number }
  | { kind: 'redo'; targetSeq: number }
  | {
      kind: 'addtitle';
      at: number;
      duration: number;
      text: string;
      subtitle: string | null;
      style: TitleStyle;
    }
  | {
      kind: 'edittitle';
      at: number;
      duration: number;
      text: string;
      subtitle: string | null;
      style: TitleStyle;
    }
  | { kind: 'removetitle'; at: number }
  | { kind: 'addcaption'; start: number; end: number; text: string; position: CaptionPos }
  | { kind: 'removecaption'; start: number }
  | { kind: 'settransition'; transition: Transition }
  | { kind: 'setcuttransition'; start: number; transition: Transition | null }
  | { kind: 'split'; at: number }
  | { kind: 'unsplit'; at: number }
  | { kind: 'move'; piece: number; before: number | null }
  /** Appended by POST /api/projects/:id/sources; listed for completeness, never sent by the client. */
  | { kind: 'addsource'; media: string; offset: number; duration: number }
  | {
      kind: 'addlayer';
      track: LayerTrack;
      start: number;
      end: number;
      media: string;
      offset: number;
      frame: Frame;
      audio: number | null;
    }
  | {
      kind: 'setlayer';
      track: LayerTrack;
      start: number;
      toTrack: LayerTrack;
      frame: Frame;
      audio: number | null;
    }
  | { kind: 'removelayer'; track: LayerTrack; start: number }
  | {
      kind: 'addaudio';
      start: number;
      end: number;
      media: string;
      offset: number;
      gain: number;
      duck: boolean;
    }
  | { kind: 'editaudio'; start: number; gain: number; duck: boolean }
  | { kind: 'removeaudio'; start: number };

export type ClientOp = Op & { opId: string };

export interface DocState {
  headSeq: number;
  edits: Edit[];
  speakerNames: string[];
  undoable: number | null;
  redoable: number | null;
  /** The project-wide transition. Absent on folds from an older server. */
  transition?: Transition;
  /** Extra piece boundaries outside any cut. Absent on folds from an older server. */
  splits?: number[];
  /** Explicit output order of piece starts. Absent on folds from an older server. */
  order?: number[];
  /** Sources appended after the project's own media. Absent on folds from an older server. */
  sources?: Source[];
}

/**
 * A fresh operation id. The server dedupes replays by it.
 * `crypto.randomUUID()` yields a v4 UUID, not v7 — a v7 would sort by
 * creation time, which is a nicety for logs, not a correctness requirement
 * here, so v4 is fine for the server's unique index.
 */
export function newOpId(): string {
  return crypto.randomUUID();
}

/** The operation an editing action sends, or null if it changes nothing shared. */
export function opForAction(state: EditorState, action: EditorAction): Op | null {
  switch (action.type) {
    case 'deleteSelection': {
      const range = selectedRange(state.selection);
      if (!range) return null;
      return { kind: 'cut', ...rangeForWords(state.words, range[0], range[1], state.duration) };
    }
    case 'overdub': {
      const range = action.range ?? selectedRange(state.selection);
      if (!range) return null;
      return {
        kind: 'overdub',
        ...rangeForWords(state.words, range[0], range[1], state.duration),
        text: action.text,
        audioUrl: action.audioUrl,
        audioDuration: action.audioDuration,
      };
    }
    case 'applyCuts':
      if (action.cuts.length === 0) return null;
      return { kind: 'applycuts', cuts: action.cuts.map(({ start, end }) => ({ start, end })) };
    case 'renameSpeaker':
      return { kind: 'renamespeaker', speaker: action.speaker, name: action.name.trim() };
    case 'addTitle':
      return {
        kind: 'addtitle',
        at: action.at,
        duration: action.duration,
        text: action.text,
        subtitle: action.subtitle,
        style: action.style,
      };
    case 'editTitle':
      return {
        kind: 'edittitle',
        at: action.at,
        duration: action.duration,
        text: action.text,
        subtitle: action.subtitle,
        style: action.style,
      };
    case 'removeTitle':
      return { kind: 'removetitle', at: action.at };
    case 'addCaption': {
      const range = action.range ?? selectedRange(state.selection);
      if (!range) return null;
      return {
        kind: 'addcaption',
        ...rangeForWords(state.words, range[0], range[1], state.duration),
        text: action.text,
        position: action.position,
      };
    }
    case 'removeCaption':
      return { kind: 'removecaption', start: action.start };
    case 'setTransition':
      return { kind: 'settransition', transition: action.transition };
    case 'setCutTransition':
      return { kind: 'setcuttransition', start: action.start, transition: action.transition };
    case 'split':
      return { kind: 'split', at: action.at };
    case 'unsplit':
      // The fold ignores an unsplit at a join, so there is nothing to send.
      return isJoin(action.at, state.sources) ? null : { kind: 'unsplit', at: action.at };
    case 'moveClip':
      return { kind: 'move', piece: action.piece, before: action.before };
    // `addSource` is deliberately absent: the upload route appends that op
    // itself, and the reducer's copy is only its optimistic echo.
    case 'addLayer': {
      const range = action.range ?? selectedRange(state.selection);
      if (!range) return null;
      return {
        kind: 'addlayer',
        track: action.track,
        ...rangeForWords(state.words, range[0], range[1], state.duration),
        media: action.media,
        offset: action.offset,
        frame: action.frame,
        audio: action.audio,
      };
    }
    case 'setLayer':
      return {
        kind: 'setlayer',
        track: action.track,
        start: action.start,
        toTrack: action.toTrack,
        frame: action.frame,
        audio: action.audio,
      };
    case 'removeLayer':
      return { kind: 'removelayer', track: action.track, start: action.start };
    case 'addAudio': {
      const range =
        action.range === null
          ? wholeRange(state)
          : (action.range ?? selectedRange(state.selection));
      if (!range) return null;
      return {
        kind: 'addaudio',
        ...rangeForWords(state.words, range[0], range[1], state.duration),
        media: action.media,
        offset: 0,
        gain: action.gain,
        duck: action.duck,
      };
    }
    case 'editAudio':
      return { kind: 'editaudio', start: action.start, gain: action.gain, duck: action.duck };
    case 'removeAudio':
      return { kind: 'removeaudio', start: action.start };
    default:
      return null;
  }
}

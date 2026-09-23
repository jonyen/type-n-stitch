// Operations: what the client tells the server it did. Mirrors
// engine/src/ops.rs. The reducer applies the same change optimistically;
// the server's fold (a `sync` action) is authoritative.

import { rangeForWords } from './editlist';
import type { EditorAction, EditorState } from './editor';
import { selectedRange, wholeRange } from './editor';
import type { CaptionPos, Edit, Range, TitleStyle, Transition } from './types';

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
  | { kind: 'addbroll'; start: number; end: number; media: string; offset: number }
  | { kind: 'removebroll'; start: number }
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
      return { kind: 'unsplit', at: action.at };
    case 'moveClip':
      return { kind: 'move', piece: action.piece, before: action.before };
    case 'addBroll': {
      const range = action.range ?? selectedRange(state.selection);
      if (!range) return null;
      return {
        kind: 'addbroll',
        ...rangeForWords(state.words, range[0], range[1], state.duration),
        media: action.media,
        offset: action.offset,
      };
    }
    case 'removeBroll':
      return { kind: 'removebroll', start: action.start };
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

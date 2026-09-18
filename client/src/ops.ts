// Operations: what the client tells the server it did. Mirrors
// engine/src/ops.rs. The reducer applies the same change optimistically;
// the server's fold (a `sync` action) is authoritative.

import { rangeForWords } from './editlist';
import type { EditorAction, EditorState } from './editor';
import { selectedRange } from './editor';
import type { Edit, Range } from './types';

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
  | { kind: 'redo'; targetSeq: number };

export type ClientOp = Op & { opId: string };

export interface DocState {
  headSeq: number;
  edits: Edit[];
  speakerNames: string[];
  undoable: number | null;
  redoable: number | null;
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
      const range = selectedRange(state.selection);
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
    default:
      return null;
  }
}

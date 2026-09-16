// Suggested edits: filler words and long pauses. A local mirror of engine/src/suggest.rs, used
// only if POST /api/media/:id/suggest fails. The server's pause list comes from measured silence.

import { cutRanges } from './editlist';
import type { CutEdit, Edit, Word } from './types';

/** Tolerance for comparing gaps against thresholds (timestamps are ms). */
const EPS = 1e-6;

export const FILLERS = ['um', 'uh', 'umm', 'uhh', 'hmm', 'mm', 'er', 'ah', 'erm'];
export const TWO_WORD_FILLERS: [string, string][] = [
  ['you', 'know'],
  ['i', 'mean'],
];

export interface SuggestOptions {
  /** Also cut "you know" and "I mean". Off by default: they are often real speech. */
  twoWordFillers: boolean;
  /** A gap between consecutive words longer than this is a pause worth tightening. */
  pauseThreshold: number;
  /** How much of a tightened pause is left in the output. */
  pauseKeep: number;
  /** Silence before the first word longer than this is trimmed. */
  leadingThreshold: number;
}

export const defaultSuggestOptions: SuggestOptions = {
  twoWordFillers: false,
  pauseThreshold: 0.6,
  pauseKeep: 0.25,
  leadingThreshold: 0.5,
};

/** Lower-case and strip surrounding punctuation so "Um," and "um" compare equal. */
export function normalizeWord(text: string): string {
  return text.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, '').toLowerCase();
}

function ownedEnd(words: Word[], index: number, duration: number): number {
  return words[index + 1]?.start ?? duration;
}

/** One cut per filler occurrence, in word order; a two-word filler is one cut. */
export function fillerCuts(
  words: Word[],
  duration: number,
  opts: SuggestOptions = defaultSuggestOptions,
): CutEdit[] {
  const norm = words.map((w) => normalizeWord(w.text));
  const cuts: CutEdit[] = [];
  let i = 0;
  while (i < words.length) {
    const word = words[i];
    if (!word) break;
    const two =
      opts.twoWordFillers &&
      i + 1 < words.length &&
      TWO_WORD_FILLERS.some(([a, b]) => norm[i] === a && norm[i + 1] === b);
    if (two) {
      cuts.push({ kind: 'cut', start: word.start, end: ownedEnd(words, i + 1, duration) });
      i += 2;
      continue;
    }
    if (FILLERS.includes(norm[i] ?? '')) {
      cuts.push({ kind: 'cut', start: word.start, end: ownedEnd(words, i, duration) });
    }
    i++;
  }
  return cuts;
}

/**
 * Word-gap pauses: a fallback only. whisper.cpp timestamps rarely leave gaps, so the server's
 * silence-based suggestions are what the editor actually uses.
 */
export function pauseCuts(words: Word[], opts: SuggestOptions = defaultSuggestOptions): CutEdit[] {
  const cuts: CutEdit[] = [];
  const first = words[0];
  if (first && first.start > opts.leadingThreshold + EPS) {
    cuts.push({ kind: 'cut', start: 0, end: first.start - opts.pauseKeep });
  }
  for (let i = 1; i < words.length; i++) {
    const prev = words[i - 1];
    const next = words[i];
    if (prev && next && next.start - prev.end > opts.pauseThreshold + EPS) {
      cuts.push({ kind: 'cut', start: prev.end + opts.pauseKeep, end: next.start });
    }
  }
  return cuts;
}

/** Suggestions not already inside an existing cut, so counts fall to 0 once applied. */
export function pending(suggested: CutEdit[], edits: Edit[]): CutEdit[] {
  const cuts = cutRanges(edits);
  return suggested.filter(
    (s) => !cuts.some((c) => c.start <= s.start + EPS && c.end >= s.end - EPS),
  );
}

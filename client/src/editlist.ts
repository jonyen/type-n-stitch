// Edit-list helpers the browser needs for live playback and rendering the
// transcript. The Rust engine (engine/src/editlist.rs) is the source of
// truth for export; these mirror the same rules for the preview.

import type { Edit, OverdubEdit, Range, Word } from './types';

const EPS = 1e-6;

/** Sort cuts and merge any that overlap or touch. Empty ranges are dropped. */
export function normalizeCuts(cuts: Range[]): Range[] {
  const sorted = cuts
    .filter((r) => r.end > r.start)
    .map((r) => ({ start: r.start, end: r.end }))
    .sort((a, b) => a.start - b.start);
  const merged: Range[] = [];
  for (const r of sorted) {
    const last = merged[merged.length - 1];
    if (last && r.start <= last.end + EPS) {
      last.end = Math.max(last.end, r.end);
    } else {
      merged.push(r);
    }
  }
  return merged;
}

export function cutRanges(edits: Edit[]): Range[] {
  return normalizeCuts(edits.filter((e) => e.kind === 'cut'));
}

export function overdubs(edits: Edit[]): OverdubEdit[] {
  return edits.filter((e): e is OverdubEdit => e.kind === 'overdub');
}

/** Source ranges that survive the cuts. */
export function keptSegments(duration: number, edits: Edit[]): Range[] {
  const kept: Range[] = [];
  let cursor = 0;
  for (const cut of cutRanges(edits)) {
    const start = Math.min(Math.max(cut.start, 0), duration);
    const end = Math.min(Math.max(cut.end, 0), duration);
    if (start > cursor + EPS) kept.push({ start: cursor, end: start });
    cursor = Math.max(cursor, end);
  }
  if (duration > cursor + EPS) kept.push({ start: cursor, end: duration });
  return kept;
}

function overlap(a: Range, b: Range): number {
  return Math.max(0, Math.min(a.end, b.end) - Math.max(a.start, b.start));
}

/** Length of the rendered output: kept source minus overdubbed ranges, plus overdub audio. */
export function outputDuration(duration: number, edits: Edit[]): number {
  const kept = keptSegments(duration, edits);
  let total = kept.reduce((sum, r) => sum + (r.end - r.start), 0);
  for (const od of overdubs(edits)) {
    total -= kept.reduce((sum, r) => sum + overlap(r, od), 0);
    total += od.audioDuration;
  }
  return total;
}

/**
 * The time a run of words "owns": from the first word's start to the next
 * word's start (or the end of the media), so the pause after the last word
 * goes with it and no half-gaps are left behind.
 */
export function rangeForWords(words: Word[], from: number, to: number, duration: number): Range {
  const first = words[from];
  if (!first) throw new Error(`no word at index ${from}`);
  const next = words[to + 1];
  return { start: first.start, end: next ? next.start : duration };
}

export type WordStatus = 'kept' | 'cut' | 'overdub';

function covers(r: Range, word: Word): boolean {
  return r.start <= word.start + EPS && r.end >= word.end - EPS;
}

/** How a word renders. An overdub takes precedence over a cut, as in the engine. */
export function wordStatus(word: Word, edits: Edit[]): WordStatus {
  if (edits.some((e) => e.kind === 'overdub' && covers(e, word))) return 'overdub';
  if (edits.some((e) => e.kind === 'cut' && covers(e, word))) return 'cut';
  return 'kept';
}

/** If `t` sits inside a cut, where playback should jump to; otherwise null. */
export function skipTarget(t: number, cuts: Range[]): number | null {
  for (const cut of cuts) {
    if (t >= cut.start && t < cut.end) return cut.end;
    if (t < cut.start) break;
  }
  return null;
}

/** The overdub whose range contains `t`, if any. */
export function overdubAt(t: number, edits: Edit[]): OverdubEdit | undefined {
  return overdubs(edits).find((od) => t >= od.start && t < od.end);
}

/** Index of the word whose owned span contains `t`, or -1 before the first word. */
export function wordIndexAt(t: number, words: Word[]): number {
  let lo = 0;
  let hi = words.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const w = words[mid];
    if (w && w.start <= t) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

export function formatTime(seconds: number): string {
  const s = Math.max(0, seconds);
  const m = Math.floor(s / 60);
  const rest = (s - m * 60).toFixed(1).padStart(4, '0');
  return `${m}:${rest}`;
}

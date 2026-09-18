// Edit-list helpers the browser needs for live playback and rendering the
// transcript. The Rust engine (engine/src/editlist.rs) is the source of
// truth for export; these mirror the same rules for the preview.

import type {
  CaptionEdit,
  CutEdit,
  Edit,
  OverdubEdit,
  Range,
  TitleEdit,
  Transition,
  Word,
} from './types';

const EPS = 1e-6;

/** How long a dip at a join lasts, in seconds. Mirrors the engine's fade. */
export const FADE = 0.25;

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

export function titles(edits: Edit[]): TitleEdit[] {
  return edits.filter((e): e is TitleEdit => e.kind === 'title');
}

export function captions(edits: Edit[]): CaptionEdit[] {
  return edits.filter((e): e is CaptionEdit => e.kind === 'caption');
}

/** The captions drawn at source time `t`, in edit-list order. */
export function captionsAt(t: number, edits: Edit[]): CaptionEdit[] {
  return captions(edits).filter((c) => t >= c.start && t < c.end);
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
  // Titles insert time; they never consume source.
  for (const title of titles(edits)) total += title.duration;
  return total;
}

/** One piece of the rendered output, in order. Mirrors the engine's `Segment`. */
export interface Piece {
  /** Where this piece comes from in the source. A title is an instant. */
  source: Range;
  kind: 'source' | 'overdub' | 'title';
  /** Index into `edits` for an overdub or title piece. */
  index?: number;
}

/** Where two neighbouring pieces meet: `after` is the index of the earlier one. */
export interface Join {
  after: number;
  transition: Transition;
}

/** Subtract `holes` (sorted, non-overlapping) from `[from, to)`. */
function complement(from: number, to: number, holes: Range[]): Range[] {
  const kept: Range[] = [];
  let cursor = from;
  for (const hole of holes) {
    const start = Math.min(Math.max(hole.start, from), to);
    const end = Math.min(Math.max(hole.end, from), to);
    if (start > cursor + EPS) kept.push({ start: cursor, end: start });
    cursor = Math.max(cursor, end);
  }
  if (to > cursor + EPS) kept.push({ start: cursor, end: to });
  return kept;
}

const RANK: Record<Piece['kind'], number> = { title: 0, overdub: 1, source: 2 };

/**
 * The rendered output, piece by piece: kept source ranges split around
 * overdubs and at every title instant, plus one piece per overdub and per
 * title. A port of the engine's `timeline` without the output ranges.
 */
export function pieces(duration: number, edits: Edit[]): Piece[] {
  const dubs = edits
    .map((e, index) => ({ e, index }))
    .filter((x): x is { e: OverdubEdit; index: number } => x.e.kind === 'overdub')
    .filter(({ e }) => e.end > e.start);
  const holes = normalizeCuts(dubs.map(({ e }) => e));
  const instants = titles(edits)
    .map((t) => t.at)
    .sort((a, b) => a - b);

  const out: Piece[] = [];
  for (const kept of keptSegments(duration, edits)) {
    for (const run of complement(kept.start, kept.end, holes)) {
      // Split the run at every title instant strictly inside it.
      let cursor = run.start;
      for (const at of instants) {
        if (at > cursor + EPS && at < run.end - EPS) {
          out.push({ source: { start: cursor, end: at }, kind: 'source' });
          cursor = at;
        }
      }
      out.push({ source: { start: cursor, end: run.end }, kind: 'source' });
    }
  }
  for (const { e, index } of dubs) {
    out.push({ source: { start: e.start, end: e.end }, kind: 'overdub', index });
  }
  edits.forEach((e, index) => {
    if (e.kind === 'title') out.push({ source: { start: e.at, end: e.at }, kind: 'title', index });
  });

  // Stable sort by start, then title before overdub before source, so a title
  // card plays ahead of anything else beginning at the same instant.
  return out
    .map((piece, i) => ({ piece, i }))
    .sort(
      (a, b) =>
        a.piece.source.start - b.piece.source.start ||
        RANK[a.piece.kind] - RANK[b.piece.kind] ||
        a.i - b.i,
    )
    .map(({ piece }) => piece);
}

/**
 * How each pair of neighbouring pieces meets. A title always dips in and out.
 * Where a cut removed source between two pieces, the cut's own transition
 * wins over the project default; anywhere else the pieces simply butt up.
 */
export function joins(list: Piece[], edits: Edit[], project: Transition): Join[] {
  const cuts = edits.filter((e): e is CutEdit => e.kind === 'cut');
  const out: Join[] = [];
  for (let i = 0; i + 1 < list.length; i++) {
    const prev = list[i];
    const next = list[i + 1];
    if (!prev || !next) continue;
    let transition: Transition = 'none';
    if (prev.kind === 'title' || next.kind === 'title') {
      transition = 'dip';
    } else if (next.source.start > prev.source.end + EPS) {
      // A cut boundary: the gap was removed. Merged cuts can fill one gap, so
      // the first cut inside it carrying an override decides.
      const override = cuts.find(
        (c) =>
          c.transition !== undefined &&
          c.start >= prev.source.end - EPS &&
          c.end <= next.source.start + EPS,
      );
      transition = override?.transition ?? project;
    }
    out.push({ after: i, transition });
  }
  return out;
}

/** Whether `t` in source time sits within a fade of a dipping join. */
export function nearDipJoin(t: number, list: Piece[], joinList: Join[]): boolean {
  return joinList.some((join) => {
    if (join.transition !== 'dip') return false;
    const prev = list[join.after];
    const next = list[join.after + 1];
    return (
      (prev !== undefined && Math.abs(t - prev.source.end) < FADE) ||
      (next !== undefined && Math.abs(t - next.source.start) < FADE)
    );
  });
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

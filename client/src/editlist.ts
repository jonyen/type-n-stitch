// Edit-list helpers the browser needs for live playback and rendering the
// transcript. The Rust engine (engine/src/editlist.rs) is the source of
// truth for export; these mirror the same rules for the preview.

import type {
  CaptionEdit,
  CutEdit,
  Edit,
  OverdubEdit,
  Range,
  Source,
  TitleEdit,
  Transition,
  Word,
} from './types';

/** Two times closer than this are the same instant, as in the engine. */
export const EPS = 1e-6;

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
 * Lay piece starts out in output order. With no `order`, source order.
 * Otherwise each start joins a parent: the greatest `order` entry at or
 * before it (entries whose piece has since gone still count), else the
 * smallest entry. Groups follow `order`; each group is in source order. So a
 * cut, split or head trim made after a move keeps what remains of a piece
 * where the piece was. Mirrors the engine's `order_starts`.
 */
export function orderStarts(starts: number[], order: number[]): number[] {
  const sorted = [...starts].sort((a, b) => a - b);
  if (order.length === 0) return sorted;
  const parent = (s: number): number => {
    let best = -1;
    order.forEach((o, i) => {
      if (o <= s + EPS && (best === -1 || o > (order[best] as number))) best = i;
    });
    if (best !== -1) return best;
    let least = 0;
    order.forEach((o, i) => {
      if (o < (order[least] as number)) least = i;
    });
    return least;
  };
  const parents = sorted.map(parent);
  const out: number[] = [];
  order.forEach((_, i) => sorted.forEach((s, j) => parents[j] === i && out.push(s)));
  return out;
}

/**
 * The kept source split at every reorder split point, then laid out in
 * output order by `orderStarts`. Mirrors the engine's `ordered_pieces`.
 */
export function orderedPieces(
  duration: number,
  edits: Edit[],
  splits: number[],
  order: number[],
): Range[] {
  const points = [...splits].sort((a, b) => a - b);
  const source: Range[] = [];
  for (const kept of keptSegments(duration, edits)) {
    let cursor = kept.start;
    for (const p of points) {
      if (p > cursor + EPS && p < kept.end - EPS) {
        source.push({ start: cursor, end: p });
        cursor = p;
      }
    }
    source.push({ start: cursor, end: kept.end });
  }
  return orderStarts(
    source.map((r) => r.start),
    order,
  ).map((s) => source.find((r) => r.start === s) as Range);
}

/** Index of the ordered piece that owns source time `t`: containing it, or the next one after it. */
export function owner(list: Range[], t: number): number {
  const inside = list.findIndex((p) => (t >= p.start && t < p.end) || Math.abs(t - p.start) < EPS);
  if (inside !== -1) return inside;
  let best = -1;
  list.forEach((p, i) => {
    if (p.start >= t && (best === -1 || p.start < (list[best] as Range).start)) best = i;
  });
  return best === -1 ? list.length : best;
}

/**
 * The rendered output, piece by piece: kept source ranges split around
 * overdubs and at every title instant, plus one piece per overdub and per
 * title. With `splits`/`order`, the pieces are laid out per ordered piece
 * (reorder-aware); with neither, this is exactly the source-order layout. A
 * port of the engine's `timeline_with`.
 */
export function pieces(
  duration: number,
  edits: Edit[],
  splits: number[] = [],
  order: number[] = [],
): Piece[] {
  const ordered = orderedPieces(duration, edits, splits, order);
  const dubs = edits
    .map((e, index) => ({ e, index }))
    .filter((x): x is { e: OverdubEdit; index: number } => x.e.kind === 'overdub')
    .filter(({ e }) => e.end > e.start)
    // A hold past the stitched end (say, a video undone under it) plays nothing.
    .filter(({ e }) => e.start < duration - EPS);
  const holes = normalizeCuts(dubs.map(({ e }) => e));
  const cards = edits
    .map((e, index) => ({ e, index }))
    .filter((x): x is { e: TitleEdit; index: number } => x.e.kind === 'title')
    .filter(({ e }) => e.at <= duration + EPS);

  const owned: Piece[][] = Array.from({ length: ordered.length + 1 }, () => []);
  for (const { e, index } of dubs) {
    owned[owner(ordered, e.start)]?.push({
      source: { start: e.start, end: e.end },
      kind: 'overdub',
      index,
    });
  }
  for (const { e, index } of cards) {
    owned[owner(ordered, e.at)]?.push({ source: { start: e.at, end: e.at }, kind: 'title', index });
  }

  const byStart = (a: { piece: Piece; i: number }, b: { piece: Piece; i: number }) =>
    a.piece.source.start - b.piece.source.start ||
    RANK[a.piece.kind] - RANK[b.piece.kind] ||
    a.i - b.i;

  const out: Piece[] = [];
  ordered.forEach((piece, k) => {
    const instants = cards
      .map((c) => c.e.at)
      .filter((at) => at > piece.start + EPS && at < piece.end - EPS)
      .sort((a, b) => a - b);
    const bucket: Piece[] = [];
    for (const run of complement(piece.start, piece.end, holes)) {
      let cursor = run.start;
      for (const at of instants) {
        if (at > cursor + EPS && at < run.end - EPS) {
          bucket.push({ source: { start: cursor, end: at }, kind: 'source' });
          cursor = at;
        }
      }
      bucket.push({ source: { start: cursor, end: run.end }, kind: 'source' });
    }
    bucket.push(...(owned[k] ?? []));
    out.push(
      ...bucket
        .map((p, i) => ({ piece: p, i }))
        .sort(byStart)
        .map(({ piece: p }) => p),
    );
  });
  const tail = owned[ordered.length] ?? [];
  out.push(
    ...tail
      .map((p, i) => ({ piece: p, i }))
      .sort(byStart)
      .map(({ piece: p }) => p),
  );
  return out;
}

/**
 * Where playback goes when source time `t` is not inside a piece: the next
 * piece's start in output order, `Infinity` past the last, or `null` while
 * inside a piece.
 */
export function jumpTarget(t: number, ordered: Range[]): number | null {
  if (ordered.some((p) => t >= p.start && t < p.end)) return null;
  let prev = -1;
  ordered.forEach((p, i) => {
    if (p.end <= t + EPS && (prev === -1 || p.end > (ordered[prev] as Range).end)) prev = i;
  });
  if (prev === -1) return ordered[0]?.start ?? Infinity;
  const next = ordered[prev + 1];
  return next ? next.start : Infinity;
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
    } else if (Math.abs(next.source.start - prev.source.end) > EPS) {
      // A cut boundary: the gap was removed. Only a cut that *starts* at the
      // boundary can override it, so where several cuts merged into one gap
      // the later ones' overrides do not apply.
      const override = cuts.find(
        (c) => c.transition !== undefined && Math.abs(c.start - prev.source.end) < EPS,
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
export function rangeForWords(
  words: Word[],
  from: number,
  to: number,
  duration: number,
  sources: readonly Placed[] = [],
): Range {
  const first = words[from];
  if (!first) throw new Error(`no word at index ${from}`);
  const next = words[to + 1];
  const end = next ? next.start : duration;
  // A word never owns time in another file: the last word before a join stops
  // at its own file's end, transcribed next file or not. The server's MCP
  // `word_range` clamps the same way.
  const last = words[to];
  const own = last ? locate(sources, last.start) : null;
  const src = own ? sources[own.index] : undefined;
  return { start: first.start, end: src ? Math.min(end, src.offset + src.duration) : end };
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

/**
 * The next override in the cycle a cut's transition button walks through:
 * nothing set → dip to black → jump cut → nothing set.
 */
export function nextCutTransition(current: Transition | null): Transition | null {
  if (current === 'dip') return 'none';
  if (current === 'none') return null;
  return 'dip';
}

/** The transition override on the cut starting at `start`, if it has one. */
export function cutTransitionAt(start: number, edits: Edit[]): Transition | null {
  const cut = edits.find((e): e is CutEdit => e.kind === 'cut' && Math.abs(e.start - start) < EPS);
  return cut?.transition ?? null;
}

/** Source starts of every piece, ascending; mirrors the engine's `piece_starts`. */
export function pieceStarts(edits: Edit[], splits: number[]): number[] {
  const cuts = cutRanges(edits);
  const starts: number[] = [];
  let cursor = 0;
  for (const cut of cuts) {
    if (cut.start > cursor + EPS) starts.push(cursor);
    cursor = Math.max(cursor, cut.end);
  }
  starts.push(cursor);
  for (const s of splits) {
    const inCut = cuts.some((c) => s > c.start - EPS && s < c.end + EPS);
    if (!inCut && !starts.some((x) => Math.abs(x - s) < EPS)) starts.push(s);
  }
  return starts.sort((a, b) => a - b);
}

type Placed = Pick<Source, 'offset' | 'duration'>;

/** Where the stitched sources end: the last one's offset plus its duration. 0 with none. */
export function stitchedDuration(sources: readonly Placed[]): number {
  const last = sources[sources.length - 1];
  return last ? last.offset + last.duration : 0;
}

/**
 * The source holding stitched instant `t` and the time inside it. An instant
 * within EPS of a join belongs to the later source; the stitched end is the
 * last source at its full duration. Null before 0, past the end, or with no
 * sources. Mirrors the engine's `locate`.
 */
export function locate(
  sources: readonly Placed[],
  t: number,
): { index: number; local: number } | null {
  const last = sources.length - 1;
  const end = stitchedDuration(sources);
  if (last < 0 || t < -EPS || t > end + EPS) return null;
  if (t >= end - EPS) return { index: last, local: (sources[last] as Placed).duration };
  let index = 0;
  for (let i = last; i >= 0; i--) {
    if (t >= (sources[i] as Placed).offset - EPS) {
      index = i;
      break;
    }
  }
  const s = sources[index] as Placed;
  return { index, local: Math.min(Math.max(t - s.offset, 0), s.duration) };
}

/** The instants where one source ends and the next begins: every offset after the first. */
export function sourceJoins(sources: readonly Placed[]): number[] {
  return sources.slice(1).map((s) => s.offset);
}

/** True when `at` is a join. The fold never unsplits one. */
export function isJoin(at: number, sources: readonly Placed[]): boolean {
  return sourceJoins(sources).some((j) => Math.abs(j - at) < EPS);
}

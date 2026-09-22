// The edit laid out in output time, and every rule the timeline tools need.
// Built on `pieces()`, which already mirrors the engine's `timeline_with`
// piece layout; this adds output positions and mirrors the engine's
// `source_to_output_time` / `output_to_source_time`, so the timeline cannot
// disagree with the export.

import { EPS, cutRanges, normalizeCuts, owner, pieceStarts, pieces } from './editlist';
import type { Edit, Range, Word } from './types';

/** The server's cap on splits per project (engine/src/types.rs `MAX_SPLITS`). */
const MAX_SPLITS = 64;

export interface Segment {
  source: Range;
  output: Range;
  kind: 'source' | 'overdub' | 'title';
  /** Index into `edits` for an overdub or title segment. */
  index?: number;
}

/** A place the razor or range tool may snap to. `word === words.length` is the end of the media. */
export interface Stop {
  at: number;
  word: number;
}

export type RazorHit = { ok: true; at: number; x: number } | { ok: false; x: number };

function holdLength(edits: Edit[], index: number | undefined): number {
  const e = index === undefined ? undefined : edits[index];
  if (e?.kind === 'overdub') return e.audioDuration;
  if (e?.kind === 'title') return e.duration;
  return 0;
}

export function timelineSegments(
  duration: number,
  edits: Edit[],
  splits: number[],
  order: number[],
): Segment[] {
  let cursor = 0;
  return pieces(duration, edits, splits, order).map((p) => {
    const length = p.kind === 'source' ? p.source.end - p.source.start : holdLength(edits, p.index);
    const segment: Segment = {
      source: p.source,
      output: { start: cursor, end: cursor + length },
      kind: p.kind,
      ...(p.index === undefined ? {} : { index: p.index }),
    };
    cursor += length;
    return segment;
  });
}

export function timelineLength(segments: Segment[]): number {
  return segments[segments.length - 1]?.output.end ?? 0;
}

/** Source time to output time. Inside a cut: where the next piece in source order begins. */
export function sourceToOutput(t: number, segments: Segment[]): number {
  for (const s of segments) {
    if (s.kind === 'title') {
      if (Math.abs(t - s.source.start) < EPS) return s.output.start;
      continue;
    }
    if (t >= s.source.start && t < s.source.end) {
      return s.kind === 'source' ? s.output.start + (t - s.source.start) : s.output.start;
    }
  }
  let best: Segment | undefined;
  for (const s of segments) {
    if (s.source.start < t - EPS) continue;
    if (
      !best ||
      s.source.start < best.source.start ||
      (s.source.start === best.source.start && s.output.start < best.output.start)
    ) {
      best = s;
    }
  }
  return best ? best.output.start : timelineLength(segments);
}

/** Output time to the source frame on screen. Holds show their first frame. */
export function outputToSource(t: number, segments: Segment[]): number {
  for (const s of segments) {
    if (t >= s.output.start && t < s.output.end) {
      return s.kind === 'source' ? s.source.start + (t - s.output.start) : s.source.start;
    }
  }
  return segments[segments.length - 1]?.source.end ?? 0;
}

/** Output positions of every kept word start, plus the end of the media, sorted. */
export function wordStops(words: Word[], segments: Segment[]): Stop[] {
  const stops: Stop[] = [];
  words.forEach((w, word) => {
    for (const s of segments) {
      if (s.kind === 'source' && w.start >= s.source.start - EPS && w.start < s.source.end - EPS) {
        stops.push({ at: s.output.start + Math.max(0, w.start - s.source.start), word });
        return;
      }
      // An overdubbed run is one unit: only its first word is a stop.
      if (s.kind === 'overdub' && Math.abs(w.start - s.source.start) < EPS) {
        stops.push({ at: s.output.start, word });
        return;
      }
    }
  });
  stops.push({ at: timelineLength(segments), word: words.length });
  return stops.sort((a, b) => a.at - b.at);
}

export function nearestStop(t: number, stops: Stop[]): Stop | null {
  let best: Stop | null = null;
  for (const s of stops) if (!best || Math.abs(s.at - t) < Math.abs(best.at - t)) best = s;
  return best;
}

/** The server's split rules, plus: a split on an existing piece start would do nothing. */
export function canSplitAt(at: number, edits: Edit[], splits: number[], duration: number): boolean {
  if (splits.length >= MAX_SPLITS) return false;
  if (at <= EPS || at >= duration - EPS) return false;
  const strictlyInside = (r: Range) => at > r.start + EPS && at < r.end - EPS;
  if (cutRanges(edits).some(strictlyInside)) return false;
  if (edits.some((e) => e.kind === 'overdub' && strictlyInside(e))) return false;
  return !pieceStarts(edits, splits).some((s) => Math.abs(s - at) < EPS);
}

export function razorAt(
  t: number,
  words: Word[],
  segments: Segment[],
  edits: Edit[],
  splits: number[],
  duration: number,
): RazorHit {
  if (segments.some((s) => s.kind === 'title' && t >= s.output.start && t < s.output.end)) {
    return { ok: false, x: t };
  }
  const stop = nearestStop(t, wordStops(words, segments));
  if (!stop) return { ok: false, x: t };
  const word = words[stop.word];
  if (!word) return { ok: false, x: stop.at };
  return canSplitAt(word.start, edits, splits, duration)
    ? { ok: true, at: word.start, x: stop.at }
    : { ok: false, x: stop.at };
}

/** The band between two pointer positions, both ends snapped to word stops; null when empty. */
export function snappedBand(
  a: number,
  b: number,
  words: Word[],
  segments: Segment[],
): Range | null {
  const stops = wordStops(words, segments);
  const lo = nearestStop(Math.min(a, b), stops);
  const hi = nearestStop(Math.max(a, b), stops);
  if (!lo || !hi || hi.at - lo.at < EPS) return null;
  return { start: lo.at, end: hi.at };
}

/** The source ranges a band removes: overdubs only when wholly covered, titles never. */
export function rangeCuts(a: number, b: number, words: Word[], segments: Segment[]): Range[] {
  const band = snappedBand(a, b, words, segments);
  if (!band) return [];
  const out: Range[] = [];
  for (const s of segments) {
    if (s.kind === 'title') continue;
    const from = Math.max(band.start, s.output.start);
    const to = Math.min(band.end, s.output.end);
    if (to - from < EPS) continue;
    if (s.kind === 'overdub') {
      if (band.start <= s.output.start + EPS && band.end >= s.output.end - EPS) out.push(s.source);
      continue;
    }
    out.push({
      start: s.source.start + (from - s.output.start),
      end: s.source.start + (to - s.output.start),
    });
  }
  return normalizeCuts(out);
}

/** Each ordered piece's span in output time, including the holds it owns. */
export function clipSpans(ordered: Range[], segments: Segment[]): Range[] {
  const spans = ordered.map(() => ({ start: Infinity, end: -Infinity }));
  for (const s of segments) {
    let k = owner(ordered, s.source.start);
    if (k >= ordered.length) k = ordered.length - 1;
    const span = spans[k];
    if (!span) continue;
    span.start = Math.min(span.start, s.output.start);
    span.end = Math.max(span.end, s.output.end);
  }
  return spans.map((s) => (s.start === Infinity ? { start: 0, end: 0 } : s));
}

/** Where a source-anchored overlay shows in output time, adjacent windows merged. */
export function overlaySpans(range: Range, segments: Segment[], spanTitles: boolean): Range[] {
  const windows: Range[] = [];
  for (const s of segments) {
    if (s.kind === 'source') {
      const from = Math.max(range.start, s.source.start);
      const to = Math.min(range.end, s.source.end);
      if (to - from > EPS) {
        windows.push({
          start: s.output.start + (from - s.source.start),
          end: s.output.start + (to - s.source.start),
        });
      }
    } else if (s.kind === 'overdub') {
      if (range.start < s.source.end && range.end > s.source.start) windows.push(s.output);
    } else if (
      spanTitles &&
      range.start <= s.source.start + EPS &&
      range.end > s.source.start + EPS
    ) {
      windows.push(s.output);
    }
  }
  const merged: Range[] = [];
  for (const w of windows.sort((a, b) => a.start - b.start)) {
    const last = merged[merged.length - 1];
    if (last && w.start <= last.end + EPS) last.end = Math.max(last.end, w.end);
    else merged.push({ ...w });
  }
  return merged;
}

/** A pointer's x inside the lanes, as output time. */
export function pxToOutput(clientX: number, left: number, width: number, length: number): number {
  if (width <= 0) return 0;
  return Math.min(Math.max((clientX - left) / width, 0), 1) * length;
}

const STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600];

/** Round-second tick positions for the ruler, at most `maxTicks` intervals across. */
export function rulerTicks(length: number, maxTicks = 8): number[] {
  if (length <= 0) return [0];
  const step = STEPS.find((s) => length / s <= maxTicks) ?? 600;
  const ticks: number[] = [];
  for (let t = 0; t <= length + EPS; t += step) ticks.push(t);
  return ticks;
}

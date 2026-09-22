import { EPS } from './editlist';
import type { Range, Word } from './types';

/** Slot index 0..n for a pointer at `x`, given card centre x's in order. */
export function dropSlot(x: number, centres: number[]): number {
  let slot = 0;
  for (const c of centres) if (x > c) slot++;
  return slot;
}

/** The move that drops card `from` into `slot`, or null when nothing changes. */
export function moveFor(ordered: Range[], from: number, slot: number) {
  if (slot === from || slot === from + 1) return null;
  const piece = ordered[from];
  if (!piece) return null;
  const target = ordered[slot];
  return { piece: piece.start, before: target ? target.start : null };
}

/** The first few words a piece owns, ellipsised when more remain. */
export function firstWords(words: Word[], piece: Range, n = 4): string {
  const own = words.filter((w) => w.start >= piece.start - EPS && w.start < piece.end);
  const text = own
    .slice(0, n)
    .map((w) => w.text)
    .join(' ');
  return own.length > n ? `${text}…` : text;
}

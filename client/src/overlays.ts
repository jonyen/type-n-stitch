// Overlay helpers for B-roll, background audio, gain, and the preview's
// speech envelope. Mirrors the engine's overlay handling for the client.

import type { AudioEdit, BrollEdit, Edit, Word } from './types';

/** How much a ducked track attenuates under speech, as a linear factor. */
export const DUCK = 0.25;

export function brolls(edits: Edit[]): BrollEdit[] {
  return edits.filter((e): e is BrollEdit => e.kind === 'broll');
}

export function audios(edits: Edit[]): AudioEdit[] {
  return edits.filter((e): e is AudioEdit => e.kind === 'audio');
}

/** The B-roll clip covering source time `t`, if any. */
export function brollAt(t: number, edits: Edit[]): BrollEdit | undefined {
  return brolls(edits).find((b) => t >= b.start && t < b.end);
}

/** The background audio clips covering source time `t`. */
export function audiosAt(t: number, edits: Edit[]): AudioEdit[] {
  return audios(edits).filter((a) => t >= a.start && t < a.end);
}

/** Seconds into the asset at source time `t`. */
export function assetTime(edit: { start: number; offset: number }, t: number): number {
  return edit.offset + Math.max(0, t - edit.start);
}

export function gainToLinear(db: number): number {
  return Math.pow(10, db / 20);
}

/** True while a word is under the playhead (the preview's speech envelope). */
export function speaking(t: number, words: Word[]): boolean {
  return words.some((w) => t >= w.start && t < w.end);
}

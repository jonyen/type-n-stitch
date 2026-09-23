// What Delete removes. The editor's selections (words, a title card, a split,
// a timeline overlay bar) are meant to be exclusive; if two ever coexist, the
// word selection wins, matching what the selection toolbar shows.

import type { EditorAction } from './editor';
import type { LayerTrack } from './overlays';

/** A selected timeline bar: a layer on V2 or V3, or a music bed. */
export type OverlayRef =
  { kind: 'layer'; track: LayerTrack; start: number } | { kind: 'audio'; start: number };

/** The `data-overlay` value a timeline bar carries; the selection toolbar anchors on it. */
export function overlayKey(ref: OverlayRef): string {
  return ref.kind === 'layer' ? `v${ref.track}:${ref.start}` : `audio:${ref.start}`;
}

/** Same bar: kind, track and start. Two tracks can hold layers starting at one instant. */
export function sameOverlay(a: OverlayRef | null, b: OverlayRef): boolean {
  return a !== null && overlayKey(a) === overlayKey(b);
}

export interface Selections {
  /** A word selection exists. */
  hasWords: boolean;
  /** A selected title card's instant. */
  title: number | null;
  /** A selected split's start (only a split; a cut boundary cannot be joined). */
  clip: number | null;
  overlay: OverlayRef | null;
}

export function deleteAction({ hasWords, title, clip, overlay }: Selections): EditorAction {
  if (hasWords) return { type: 'deleteSelection' };
  if (title !== null) return { type: 'removeTitle', at: title };
  if (clip !== null) return { type: 'unsplit', at: clip };
  if (overlay) {
    return overlay.kind === 'layer'
      ? { type: 'removeLayer', track: overlay.track, start: overlay.start }
      : { type: 'removeAudio', start: overlay.start };
  }
  return { type: 'deleteSelection' };
}

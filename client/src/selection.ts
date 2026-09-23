// What Delete removes. The editor's selections (words, a title card, a split,
// a timeline overlay bar) are meant to be exclusive; if two ever coexist, the
// word selection wins, matching what the selection toolbar shows.

import type { OverlayRef } from './components/Timeline';
import type { EditorAction } from './editor';

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
    return overlay.kind === 'broll'
      ? { type: 'removeBroll', start: overlay.start }
      : { type: 'removeAudio', start: overlay.start };
  }
  return { type: 'deleteSelection' };
}

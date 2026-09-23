import { describe, expect, it } from 'vitest';

import { deleteAction, overlayKey, sameOverlay } from './selection';

const none = { hasWords: false, title: null, clip: null, overlay: null };

describe('deleteAction', () => {
  it('deletes the words when a word selection appears over a selected overlay', () => {
    expect(
      deleteAction({ ...none, hasWords: true, overlay: { kind: 'layer', track: 2, start: 2 } }),
    ).toEqual({ type: 'deleteSelection' });
    expect(deleteAction({ ...none, hasWords: true, overlay: { kind: 'audio', start: 0 } })).toEqual(
      { type: 'deleteSelection' },
    );
  });

  it('removes a selected layer on its own track, or a selected music bar', () => {
    expect(deleteAction({ ...none, overlay: { kind: 'layer', track: 2, start: 2 } })).toEqual({
      type: 'removeLayer',
      track: 2,
      start: 2,
    });
    expect(deleteAction({ ...none, overlay: { kind: 'layer', track: 3, start: 2 } })).toEqual({
      type: 'removeLayer',
      track: 3,
      start: 2,
    });
    expect(deleteAction({ ...none, overlay: { kind: 'audio', start: 0 } })).toEqual({
      type: 'removeAudio',
      start: 0,
    });
  });

  it('removes a selected title card or joins a selected split', () => {
    expect(deleteAction({ ...none, title: 3 })).toEqual({ type: 'removeTitle', at: 3 });
    expect(deleteAction({ ...none, clip: 5 })).toEqual({ type: 'unsplit', at: 5 });
  });

  it('falls back to deleting the (possibly empty) word selection', () => {
    expect(deleteAction(none)).toEqual({ type: 'deleteSelection' });
  });
});

describe('overlay refs', () => {
  it('key a bar by its track, so V2 and V3 bars at one instant stay apart', () => {
    expect(overlayKey({ kind: 'layer', track: 2, start: 12.5 })).toBe('v2:12.5');
    expect(overlayKey({ kind: 'layer', track: 3, start: 12.5 })).toBe('v3:12.5');
    expect(overlayKey({ kind: 'audio', start: 0 })).toBe('audio:0');
  });

  it('match only the same kind, track and start', () => {
    const v2 = { kind: 'layer', track: 2, start: 4 } as const;
    expect(sameOverlay(v2, { kind: 'layer', track: 2, start: 4 })).toBe(true);
    expect(sameOverlay(v2, { kind: 'layer', track: 3, start: 4 })).toBe(false);
    expect(sameOverlay({ kind: 'audio', start: 4 }, v2)).toBe(false);
    expect(sameOverlay(null, v2)).toBe(false);
  });
});

import { describe, expect, it } from 'vitest';

import { deleteAction } from './selection';

const none = { hasWords: false, title: null, clip: null, overlay: null };

describe('deleteAction', () => {
  it('deletes the words when a word selection appears over a selected overlay', () => {
    expect(deleteAction({ ...none, hasWords: true, overlay: { kind: 'broll', start: 2 } })).toEqual(
      { type: 'deleteSelection' },
    );
    expect(deleteAction({ ...none, hasWords: true, overlay: { kind: 'audio', start: 0 } })).toEqual(
      { type: 'deleteSelection' },
    );
  });

  it('removes a selected B-roll or music bar', () => {
    expect(deleteAction({ ...none, overlay: { kind: 'broll', start: 2 } })).toEqual({
      type: 'removeBroll',
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

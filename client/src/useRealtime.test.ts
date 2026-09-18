import { describe, expect, it } from 'vitest';

import { holdOrApply, type RemoteDoc } from './useRealtime';

const fold = (headSeq: number): RemoteDoc => ({ headSeq, edits: [], speakerNames: [] });

describe('holdOrApply', () => {
  it('applies a fold straight away when nothing of ours is in flight', () => {
    expect(holdOrApply(fold(3), null, 0)).toEqual({ apply: fold(3), held: null });
  });

  it('holds a fold that arrives while our own edit is un-acked', () => {
    expect(holdOrApply(fold(3), null, 1)).toEqual({ apply: null, held: fold(3) });
  });

  it('keeps only the newest held fold', () => {
    expect(holdOrApply(fold(5), fold(4), 2)).toEqual({ apply: null, held: fold(5) });
    expect(holdOrApply(fold(4), fold(5), 2)).toEqual({ apply: null, held: fold(5) });
  });

  it('drops anything held once the queue drains', () => {
    expect(holdOrApply(fold(6), fold(4), 0)).toEqual({ apply: fold(6), held: null });
  });
});

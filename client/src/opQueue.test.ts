import { describe, expect, it, vi } from 'vitest';

import { ApiError } from './api';
import { createOpQueue } from './opQueue';
import type { ClientOp, DocState } from './ops';

const doc = (headSeq: number): DocState => ({
  headSeq,
  edits: [],
  speakerNames: [],
  undoable: null,
  redoable: null,
});
const cut = (opId: string): ClientOp => ({ opId, kind: 'cut', start: 1, end: 2 });

/** A submit stub whose promises are resolved by hand, in order. */
function manualSubmit() {
  const calls: { ops: ClientOp[]; resolve: (d: DocState) => void; reject: (e: unknown) => void }[] =
    [];
  const submit = (ops: ClientOp[]) =>
    new Promise<DocState>((resolve, reject) => {
      calls.push({ ops, resolve, reject });
    });
  return { submit, calls };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe('createOpQueue', () => {
  it('sends one op at a time, in order, and resolves each with its own DocState', async () => {
    const { submit, calls } = manualSubmit();
    const q = createOpQueue(submit);
    const p1 = q.push(cut('a'));
    const p2 = q.push(cut('b'));
    await tick();
    expect(calls).toHaveLength(1);
    expect(calls[0]?.ops[0]?.opId).toBe('a');
    expect(q.pending).toBe(2);
    calls[0]?.resolve(doc(1));
    await expect(p1).resolves.toEqual(doc(1));
    await tick();
    expect(calls).toHaveLength(2);
    expect(calls[1]?.ops[0]?.opId).toBe('b');
    calls[1]?.resolve(doc(2));
    await expect(p2).resolves.toEqual(doc(2));
    expect(q.pending).toBe(0);
  });

  it('retries the same op after a network error and keeps the order', async () => {
    vi.useFakeTimers();
    const { submit, calls } = manualSubmit();
    const q = createOpQueue(submit, { retryDelayMs: 500 });
    const p1 = q.push(cut('a'));
    q.push(cut('b'));
    await vi.advanceTimersByTimeAsync(0);
    calls[0]?.reject(new ApiError('offline', 0));
    await vi.advanceTimersByTimeAsync(499);
    expect(calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(calls).toHaveLength(2);
    expect(calls[1]?.ops[0]?.opId).toBe('a');
    calls[1]?.resolve(doc(1));
    await expect(p1).resolves.toEqual(doc(1));
    vi.useRealTimers();
  });

  it('drops an op the server rejected and moves on', async () => {
    const { submit, calls } = manualSubmit();
    const q = createOpQueue(submit);
    const p1 = q.push(cut('a'));
    const p2 = q.push(cut('b'));
    await tick();
    calls[0]?.reject(new ApiError('range is outside the media', 400));
    await expect(p1).rejects.toThrow('range is outside the media');
    await tick();
    expect(calls[1]?.ops[0]?.opId).toBe('b');
    calls[1]?.resolve(doc(1));
    await expect(p2).resolves.toEqual(doc(1));
  });

  it('flush() retries immediately instead of waiting out the delay', async () => {
    vi.useFakeTimers();
    const { submit, calls } = manualSubmit();
    const q = createOpQueue(submit, { retryDelayMs: 10_000 });
    q.push(cut('a'));
    await vi.advanceTimersByTimeAsync(0);
    calls[0]?.reject(new ApiError('offline', 0));
    await vi.advanceTimersByTimeAsync(0);
    q.flush();
    await vi.advanceTimersByTimeAsync(0);
    expect(calls).toHaveLength(2);
    vi.useRealTimers();
  });
});

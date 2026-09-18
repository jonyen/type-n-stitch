// A serial queue for operations: one POST in flight at a time, in order,
// retried after network failures, dropped after a rejection. The server
// dedupes by opId, so retrying a request whose reply was lost is safe.

import { ApiError } from './api';
import type { ClientOp, DocState } from './ops';

export type Submit = (ops: ClientOp[]) => Promise<DocState>;

export interface OpQueue {
  /** Append and start flushing; resolves with the DocState the server returned for this op. */
  push(op: ClientOp): Promise<DocState>;
  /** Try again now (after a reconnect). */
  flush(): void;
  readonly pending: number;
}

interface Options {
  retryDelayMs?: number;
  onError?: (err: unknown, op: ClientOp) => 'retry' | 'drop';
}

interface Entry {
  op: ClientOp;
  resolve: (doc: DocState) => void;
  reject: (err: unknown) => void;
}

/** Network trouble and server errors are worth another try; 4xx means no. */
function defaultClassify(err: unknown): 'retry' | 'drop' {
  if (err instanceof ApiError) return err.status === 0 || err.status >= 500 ? 'retry' : 'drop';
  return 'retry';
}

export function createOpQueue(submit: Submit, opts: Options = {}): OpQueue {
  const retryDelayMs = opts.retryDelayMs ?? 1000;
  const classify = opts.onError ?? defaultClassify;
  const entries: Entry[] = [];
  let inFlight = false;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;

  const run = async () => {
    if (inFlight) return;
    const entry = entries[0];
    if (!entry) return;
    inFlight = true;
    try {
      const doc = await submit([entry.op]);
      entries.shift();
      entry.resolve(doc);
    } catch (err) {
      if (classify(err, entry.op) === 'drop') {
        entries.shift();
        entry.reject(err);
      } else {
        inFlight = false;
        retryTimer = setTimeout(() => {
          retryTimer = null;
          void run();
        }, retryDelayMs);
        return;
      }
    }
    inFlight = false;
    void run();
  };

  return {
    push(op) {
      return new Promise<DocState>((resolve, reject) => {
        entries.push({ op, resolve, reject });
        void run();
      });
    },
    flush() {
      if (retryTimer) {
        clearTimeout(retryTimer);
        retryTimer = null;
      }
      void run();
    },
    get pending() {
      return entries.length;
    },
  };
}

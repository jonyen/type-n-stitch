import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { connectRealtime, presenceEquals, wsUrl, type ServerMsg } from './realtime';

/** A WebSocket double: records sends, lets tests open/close/deliver. */
class FakeSocket {
  static instances: FakeSocket[] = [];
  static OPEN = 1;
  readyState = 0;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor(public url: string) {
    FakeSocket.instances.push(this);
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.closed = true;
    this.readyState = 3;
    this.onclose?.();
  }
  open() {
    this.readyState = 1;
    this.onopen?.();
  }
  deliver(msg: ServerMsg) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
  drop() {
    this.readyState = 3;
    this.onclose?.();
  }
}

const presence = (playhead: number) => ({ playhead, selection: null, caret: null, playing: false });

/** Non-null assertion as a function, so the lint rule against `!` stays meaningful elsewhere. */
function must<T>(v: T | null | undefined): T {
  if (v == null) throw new Error('expected a value');
  return v;
}

describe('wsUrl', () => {
  it('uses ws for http and wss for https', () => {
    expect(wsUrl('p1', { protocol: 'http:', host: 'localhost:5174' })).toBe(
      'ws://localhost:5174/api/projects/p1/ws',
    );
    expect(wsUrl('p1', { protocol: 'https:', host: 'x.test' })).toBe(
      'wss://x.test/api/projects/p1/ws',
    );
  });
});

describe('presenceEquals', () => {
  it('compares every field including the selection tuple', () => {
    expect(presenceEquals(presence(1), presence(1))).toBe(true);
    expect(presenceEquals(presence(1), presence(2))).toBe(false);
    expect(
      presenceEquals({ ...presence(1), selection: [1, 2] }, { ...presence(1), selection: [1, 2] }),
    ).toBe(true);
    expect(
      presenceEquals({ ...presence(1), selection: [1, 2] }, { ...presence(1), selection: [1, 3] }),
    ).toBe(false);
  });
});

describe('connectRealtime', () => {
  beforeEach(() => {
    FakeSocket.instances = [];
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  const opts = () => ({
    WebSocketImpl: FakeSocket as unknown as typeof WebSocket,
    baseDelayMs: 100,
    maxDelayMs: 1000,
    random: () => 0.5,
  });

  it('reports status, parses frames, and sends presence only when open and changed', () => {
    const onMessage = vi.fn();
    const onStatus = vi.fn();
    const rt = connectRealtime('p1', { onMessage, onStatus }, opts());
    const sock = must(FakeSocket.instances[0]);
    expect(onStatus).toHaveBeenLastCalledWith('connecting');
    rt.sendPresence(presence(1)); // queued before open
    sock.open();
    expect(onStatus).toHaveBeenLastCalledWith('open');
    expect(sock.sent).toHaveLength(1);
    expect(JSON.parse(must(sock.sent[0]))).toEqual({ t: 'presence', ...presence(1) });

    sock.deliver({ t: 'pong' });
    expect(onMessage).toHaveBeenCalledWith({ t: 'pong' });

    rt.sendPresence(presence(2));
    vi.advanceTimersByTime(100);
    expect(sock.sent).toHaveLength(2);
    expect(JSON.parse(must(sock.sent[1]))).toEqual({ t: 'presence', ...presence(2) });
    rt.sendPresence(presence(2)); // unchanged: nothing
    vi.advanceTimersByTime(100);
    expect(sock.sent).toHaveLength(2);
  });

  it('coalesces bursts to one frame per 100 ms carrying the latest state', () => {
    const rt = connectRealtime('p1', { onMessage: vi.fn(), onStatus: vi.fn() }, opts());
    const sock = must(FakeSocket.instances[0]);
    sock.open();
    rt.sendPresence(presence(1));
    rt.sendPresence(presence(2));
    rt.sendPresence(presence(3));
    vi.advanceTimersByTime(100);
    expect(sock.sent).toHaveLength(1);
    expect(JSON.parse(must(sock.sent[0])).playhead).toBe(3);
  });

  it('reconnects with backoff after an unexpected close and not after close()', () => {
    const onStatus = vi.fn();
    const rt = connectRealtime('p1', { onMessage: vi.fn(), onStatus }, opts());
    must(FakeSocket.instances[0]).open();
    must(FakeSocket.instances[0]).drop();
    expect(onStatus).toHaveBeenLastCalledWith('reconnecting');
    vi.advanceTimersByTime(99); // 100 * 2^0 * (0.5 + 0.5) = 100
    expect(FakeSocket.instances).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(FakeSocket.instances).toHaveLength(2);
    must(FakeSocket.instances[1]).drop(); // still not open: attempt 1 → 200 ms
    vi.advanceTimersByTime(200);
    expect(FakeSocket.instances).toHaveLength(3);
    must(FakeSocket.instances[2]).open(); // resets the attempt counter
    expect(onStatus).toHaveBeenLastCalledWith('open');

    rt.close();
    expect(onStatus).toHaveBeenLastCalledWith('closed');
    vi.advanceTimersByTime(5000);
    expect(FakeSocket.instances).toHaveLength(3);
  });

  it('caps the delay at maxDelayMs', () => {
    connectRealtime('p1', { onMessage: vi.fn(), onStatus: vi.fn() }, opts());
    for (let i = 0; i < 6; i++) {
      must(FakeSocket.instances[i]).drop();
      vi.advanceTimersByTime(1000);
    }
    expect(FakeSocket.instances).toHaveLength(7);
  });
});

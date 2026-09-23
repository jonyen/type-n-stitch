import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { addSource, ApiError, login, request, setUnauthorizedHandler } from './api';
import type { SourceView } from './types';

function stub401() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response(JSON.stringify({ error: 'sign in first' }), { status: 401 })),
  );
}

afterEach(() => {
  setUnauthorizedHandler(null);
  vi.unstubAllGlobals();
});

describe('request', () => {
  it('reports the session as gone when an ordinary call answers 401', async () => {
    stub401();
    const onUnauthorized = vi.fn();
    setUnauthorizedHandler(onUnauthorized);
    await expect(request('/api/projects')).rejects.toBeInstanceOf(ApiError);
    expect(onUnauthorized).toHaveBeenCalledTimes(1);
  });

  it('leaves the session alone when a sign-in is rejected', async () => {
    stub401();
    const onUnauthorized = vi.fn();
    setUnauthorizedHandler(onUnauthorized);
    await expect(login('ada@example.com', 'wrong')).rejects.toBeInstanceOf(ApiError);
    expect(onUnauthorized).not.toHaveBeenCalled();
  });
});

/** Just enough XMLHttpRequest for `upload`: open, send, upload progress and a reply. */
class FakeXhr {
  static last: FakeXhr | null = null;
  method = '';
  url = '';
  body: unknown = null;
  status = 0;
  statusText = '';
  responseText = '';
  upload: {
    onprogress: ((e: { lengthComputable: boolean; loaded: number; total: number }) => void) | null;
  } = { onprogress: null };
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onabort: (() => void) | null = null;
  ontimeout: (() => void) | null = null;
  open(method: string, url: string) {
    this.method = method;
    this.url = url;
  }
  send(body: unknown) {
    this.body = body;
    FakeXhr.last = this;
  }
  respond(status: number, body: unknown) {
    this.status = status;
    this.statusText = status < 300 ? 'OK' : 'Bad Request';
    this.responseText = JSON.stringify(body);
    this.onload?.();
  }
}

const view: SourceView = {
  index: 1,
  mediaId: 'm1',
  url: '/data/m1/source.mp4',
  filename: 'take2.mp4',
  kind: 'video',
  offset: 60,
  duration: 30.5,
  transcript: 'pending',
};

describe('addSource', () => {
  const file = new File(['abc'], 'take2.mp4', { type: 'video/mp4' });

  beforeEach(() => {
    FakeXhr.last = null;
    vi.stubGlobal('XMLHttpRequest', FakeXhr);
  });

  it('posts the file to /sources and reports upload progress', async () => {
    const onProgress = vi.fn();
    const done = addSource('p1', file, onProgress);
    const xhr = FakeXhr.last as unknown as FakeXhr;
    expect(xhr.method).toBe('POST');
    expect(xhr.url).toBe('/api/projects/p1/sources');
    expect((xhr.body as FormData).get('file')).toBeInstanceOf(File);
    xhr.upload.onprogress?.({ lengthComputable: true, loaded: 25, total: 100 });
    xhr.upload.onprogress?.({ lengthComputable: false, loaded: 50, total: 0 });
    xhr.respond(200, view);
    await expect(done).resolves.toEqual(view);
    expect(onProgress.mock.calls.map(([f]) => f)).toEqual([0.25, 1]);
  });

  it("rejects with the server's message", async () => {
    const done = addSource('p1', file);
    (FakeXhr.last as unknown as FakeXhr).respond(400, {
      error: 'a project holds at most 20 videos',
    });
    await expect(done).rejects.toMatchObject({
      message: 'a project holds at most 20 videos',
      status: 400,
    });
  });

  it('reports the session as gone on a 401', async () => {
    const onUnauthorized = vi.fn();
    setUnauthorizedHandler(onUnauthorized);
    const done = addSource('p1', file);
    (FakeXhr.last as unknown as FakeXhr).respond(401, { error: 'sign in first' });
    await expect(done).rejects.toBeInstanceOf(ApiError);
    expect(onUnauthorized).toHaveBeenCalledOnce();
  });

  it('says the server is unreachable on a network error', async () => {
    const done = addSource('p1', file);
    (FakeXhr.last as unknown as FakeXhr).onerror?.();
    await expect(done).rejects.toMatchObject({ status: 0 });
  });

  it('says the server is unreachable if the upload is aborted', async () => {
    const done = addSource('p1', file);
    (FakeXhr.last as unknown as FakeXhr).onabort?.();
    await expect(done).rejects.toMatchObject({ status: 0 });
  });

  it('says the server is unreachable if the upload times out', async () => {
    const done = addSource('p1', file);
    (FakeXhr.last as unknown as FakeXhr).ontimeout?.();
    await expect(done).rejects.toMatchObject({ status: 0 });
  });
});

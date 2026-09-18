import { afterEach, describe, expect, it, vi } from 'vitest';

import { ApiError, login, request, setUnauthorizedHandler } from './api';

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

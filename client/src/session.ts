// Who is signed in. `undefined` until the first /api/me answers.

import { useCallback, useEffect, useState } from 'react';

import { ApiError, fetchMe, logout, setUnauthorizedHandler } from './api';
import type { User } from './types';

export function useSession() {
  const [user, setUser] = useState<User | null | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;
    fetchMe()
      .then((u) => {
        if (!cancelled) setUser(u);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        // 401 means "nobody"; anything else is still "nobody" but worth a log.
        if (!(err instanceof ApiError && err.status === 401)) console.error(err);
        setUser(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // A 401 on any later request means the session went away (expired, or the
  // database was reset): fall back to the login screen rather than leaving
  // every panel complaining that you must sign in first.
  useEffect(() => {
    setUnauthorizedHandler(() => setUser(null));
    return () => setUnauthorizedHandler(null);
  }, []);

  const signOut = useCallback(async () => {
    await logout().catch(() => undefined);
    setUser(null);
  }, []);

  return { user, setUser, signOut };
}

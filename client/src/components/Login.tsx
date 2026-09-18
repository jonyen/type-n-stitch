import { useState, type SubmitEvent } from 'react';

import { login, register } from '../api';
import type { User } from '../types';

interface Props {
  /** No accounts exist yet: show "create the first account" instead of sign in. */
  needsSetup: boolean;
  onSignedIn: (user: User) => void;
}

export function Login({ needsSetup, onSignedIn }: Props) {
  const [mode, setMode] = useState<'login' | 'register'>(needsSetup ? 'register' : 'login');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const onSubmit = async (e: SubmitEvent<HTMLFormElement>) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const user =
        mode === 'login'
          ? await login(email, password)
          : await register(email, password, displayName);
      onSignedIn(user);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="login" onSubmit={onSubmit}>
      <h2>
        {mode === 'login'
          ? 'Sign in'
          : needsSetup
            ? 'Create the first account'
            : 'Create an account'}
      </h2>
      {mode === 'register' && (
        <label>
          Name
          <input
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
            disabled={busy}
            autoFocus
          />
        </label>
      )}
      <label>
        Email
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          disabled={busy}
          autoFocus={mode === 'login'}
          required
        />
      </label>
      <label>
        Password
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          disabled={busy}
          minLength={8}
          required
        />
      </label>
      {error && <p className="error">{error}</p>}
      <button type="submit" disabled={busy}>
        {mode === 'login' ? 'Sign in' : 'Create account'}
      </button>
      {!needsSetup && (
        <button
          type="button"
          className="ghost"
          disabled={busy}
          onClick={() => {
            setError(null);
            setMode(mode === 'login' ? 'register' : 'login');
          }}
        >
          {mode === 'login' ? 'Need an account?' : 'Have an account? Sign in'}
        </button>
      )}
    </form>
  );
}

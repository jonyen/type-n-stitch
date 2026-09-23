import { useState, type SubmitEvent } from 'react';

import { login, register } from '../api';
import type { User } from '../types';
import { cx } from '../cx';
import styles from './Login.module.css';
import ui from '../styles/ui.module.css';

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

  const title =
    mode === 'login' ? 'Sign in' : needsSetup ? 'Create the first account' : 'Create an account';
  const hint = needsSetup
    ? 'This account becomes the owner of any media already on this server.'
    : mode === 'login'
      ? 'Welcome back. Your projects are waiting.'
      : 'Projects, edits and comments are saved to your account.';

  return (
    <div className={styles.wrap}>
      <form className={styles.card} onSubmit={onSubmit}>
        <div className={styles.brand}>
          <span className={styles.logo} aria-hidden />
          <span className={styles.wordmark}>type-n-stitch</span>
          <span>edit media by editing its words</span>
        </div>
        <h2>{title}</h2>
        <p className={styles.hint}>{hint}</p>
        {mode === 'register' && (
          <label>
            Name
            <input
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              disabled={busy}
              placeholder="Ada Lovelace"
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
            placeholder="you@example.com"
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
            placeholder={mode === 'register' ? 'at least 8 characters' : ''}
            minLength={8}
            required
          />
        </label>
        {error && <p className={ui.error}>{error}</p>}
        <button type="submit" className={cx(ui.button, ui.primary, styles.submit)} disabled={busy}>
          {busy ? 'One moment…' : mode === 'login' ? 'Sign in' : 'Create account'}
        </button>
        {!needsSetup && (
          <button
            type="button"
            className={styles.link}
            disabled={busy}
            onClick={() => {
              setError(null);
              setMode(mode === 'login' ? 'register' : 'login');
            }}
          >
            {mode === 'login' ? 'Need an account? Create one' : 'Have an account? Sign in'}
          </button>
        )}
      </form>
    </div>
  );
}

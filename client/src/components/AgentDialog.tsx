import { useEffect, useRef, useState, type FormEvent } from 'react';

import { mcpAddCommand } from '../agent';
import { createToken, listTokens, revokeToken } from '../api';
import { cx } from '../cx';
import { relativeTime } from '../format';
import type { TokenInfo } from '../types';
import agent from './AgentDialog.module.css';
import { DialogFrame } from './DialogFrame';
import frame from './DialogFrame.module.css';
import ui from '../styles/ui.module.css';

interface Props {
  onCancel: () => void;
}

/** A label input, then (once minted) the token and the `claude mcp add` line, shown once. */
export function AgentDialog({ onCancel }: Props) {
  const [tokens, setTokens] = useState<TokenInfo[] | null>(null);
  const [label, setLabel] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [minted, setMinted] = useState<{ token: string; id: string } | null>(null);
  const first = useRef<HTMLInputElement>(null);

  useEffect(() => {
    first.current?.focus();
  }, []);

  useEffect(() => {
    listTokens()
      .then(setTokens)
      .catch((err) => setError(err instanceof Error ? err.message : String(err)));
  }, []);

  const create = async (e: FormEvent) => {
    e.preventDefault();
    if (!label.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const created = await createToken(label.trim());
      setMinted({ token: created.token, id: created.id });
      setTokens((prev) => [
        { id: created.id, label: created.label, createdAt: created.createdAt, lastUsedAt: null },
        ...(prev ?? []),
      ]);
      setLabel('');
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (id: string) => {
    setError(null);
    try {
      await revokeToken(id);
      setTokens((prev) => (prev ?? []).filter((t) => t.id !== id));
      if (minted?.id === id) setMinted(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <DialogFrame
      title="Connect an agent"
      description="Mint a token so Claude can join this project as its own agent — cursor, edits, and undo all its own."
      busy={busy || minted !== null}
      wide
      onClose={onCancel}
    >
      <div className={frame.form}>
        {minted ? (
          <MintedToken
            origin={window.location.origin}
            token={minted.token}
            onDone={() => setMinted(null)}
          />
        ) : (
          <form className={agent.create} onSubmit={create}>
            <label className={ui.field}>
              Label
              <input
                ref={first}
                type="text"
                value={label}
                maxLength={100}
                placeholder="e.g. laptop"
                onChange={(e) => setLabel(e.target.value)}
                disabled={busy}
              />
            </label>
            <button
              type="submit"
              className={cx(ui.button, ui.primary)}
              disabled={busy || !label.trim()}
            >
              {busy ? 'Creating…' : 'Create token'}
            </button>
          </form>
        )}

        {error && <p className={ui.error}>{error}</p>}

        <div className={agent.list}>
          <h3>Tokens</h3>
          {tokens === null ? (
            <p className={ui.muted}>Loading…</p>
          ) : tokens.length === 0 ? (
            <p className={ui.muted}>No tokens yet. Create one above to connect an agent.</p>
          ) : (
            <ul>
              {tokens.map((t) => (
                <li key={t.id} className={agent.row}>
                  <span className={agent.label}>{t.label}</span>
                  <span className={cx(ui.muted, agent.meta)}>
                    created {relativeTime(t.createdAt)} · last used{' '}
                    {t.lastUsedAt ? relativeTime(t.lastUsedAt) : 'never'}
                  </span>
                  <button
                    type="button"
                    className={cx(ui.button, ui.ghost)}
                    onClick={() => revoke(t.id)}
                  >
                    Revoke
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        {!minted && (
          <div className={frame.actions}>
            <span className={frame.spacer} />
            <button type="button" className={ui.button} onClick={onCancel}>
              Done
            </button>
          </div>
        )}
      </div>
    </DialogFrame>
  );
}

function MintedToken({
  origin,
  token,
  onDone,
}: {
  origin: string;
  token: string;
  onDone: () => void;
}) {
  const command = mcpAddCommand(origin, token);
  return (
    <div className={agent.minted}>
      <p className={ui.muted}>
        This token is shown once — copy it now. Anyone with it can edit as this agent.
      </p>
      <CopyBox label="Token" value={token} />
      <CopyBox label="claude mcp add" value={command} />
      <div className={frame.actions}>
        <span className={frame.spacer} />
        <button type="button" className={cx(ui.button, ui.primary)} onClick={onDone}>
          Done
        </button>
      </div>
    </div>
  );
}

function CopyBox({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access can be denied; the value is still selectable text.
    }
  };

  return (
    <div className={agent.copyBox}>
      <span className={cx(agent.copyLabel, ui.muted)}>{label}</span>
      <code className={agent.copyValue}>{value}</code>
      <button type="button" className={cx(ui.button, ui.ghost)} onClick={copy}>
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  );
}

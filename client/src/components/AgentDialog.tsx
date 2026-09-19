import { useEffect, useRef, useState, type FormEvent } from 'react';

import { mcpAddCommand } from '../agent';
import { createToken, listTokens, revokeToken } from '../api';
import { relativeTime } from '../format';
import type { TokenInfo } from '../types';

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
    <div
      className="modal-backdrop"
      onClick={busy || minted ? undefined : onCancel}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && !busy && !minted) onCancel();
      }}
    >
      <div className="modal agent-dialog" onClick={(e) => e.stopPropagation()}>
        <h2>Connect an agent</h2>
        <p className="muted">
          Mint a token so Claude can join this project as its own agent — cursor, edits, and undo
          all its own.
        </p>

        {minted ? (
          <MintedToken
            origin={window.location.origin}
            token={minted.token}
            onDone={() => setMinted(null)}
          />
        ) : (
          <form className="agent-create" onSubmit={create}>
            <label className="field">
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
            <button type="submit" className="primary" disabled={busy || !label.trim()}>
              {busy ? 'Creating…' : 'Create token'}
            </button>
          </form>
        )}

        {error && <p className="error">{error}</p>}

        <div className="token-list">
          <h3>Tokens</h3>
          {tokens === null ? (
            <p className="muted">Loading…</p>
          ) : tokens.length === 0 ? (
            <p className="muted">No tokens yet. Create one above to connect an agent.</p>
          ) : (
            <ul>
              {tokens.map((t) => (
                <li key={t.id} className="token-row">
                  <span className="token-label">{t.label}</span>
                  <span className="muted token-meta">
                    created {relativeTime(t.createdAt)} · last used{' '}
                    {t.lastUsedAt ? relativeTime(t.lastUsedAt) : 'never'}
                  </span>
                  <button type="button" className="ghost" onClick={() => revoke(t.id)}>
                    Revoke
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        {!minted && (
          <div className="actions">
            <span className="spacer" />
            <button type="button" onClick={onCancel}>
              Done
            </button>
          </div>
        )}
      </div>
    </div>
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
    <div className="minted-token">
      <p className="muted">
        This token is shown once — copy it now. Anyone with it can edit as this agent.
      </p>
      <CopyBox label="Token" value={token} />
      <CopyBox label="claude mcp add" value={command} />
      <div className="actions">
        <span className="spacer" />
        <button type="button" className="primary" onClick={onDone}>
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
    <div className="copy-box">
      <span className="copy-box-label muted">{label}</span>
      <code className="copy-box-value">{value}</code>
      <button type="button" className="ghost" onClick={copy}>
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  );
}

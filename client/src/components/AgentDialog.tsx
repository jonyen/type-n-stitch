import { Tabs } from 'radix-ui';
import { useEffect, useRef, useState, type FormEvent } from 'react';

import { authHeader, mcpAddCommand, mcpEndpoint, mcpJsonConfig, mcpRemoteConfig } from '../agent';
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

/** Shown in the setup snippets until a token has been minted this visit. */
const PLACEHOLDER_TOKEN = '<token>';

/** A label input, then (once minted) the token, shown once, plus setup snippets for any client. */
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

  const origin = window.location.origin;

  return (
    <DialogFrame
      title="Connect AI"
      description="Any MCP client — Claude Code, Cursor and others — can join a project as its own agent with this endpoint and a token."
      busy={busy || minted !== null}
      wide
      onClose={onCancel}
    >
      <div className={frame.form}>
        <div className={agent.endpoint}>
          <CopyBox label="MCP endpoint" value={mcpEndpoint(origin)} />
          <p className={ui.muted}>
            Clients authenticate with the header <code>Authorization: Bearer &lt;token&gt;</code>.
          </p>
        </div>

        {minted ? (
          <MintedToken token={minted.token} onDone={() => setMinted(null)} />
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

        <SetupSnippets origin={origin} token={minted?.token ?? null} />

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

function MintedToken({ token, onDone }: { token: string; onDone: () => void }) {
  return (
    <div className={agent.minted}>
      <p className={ui.muted}>
        This token is shown once — copy it now. Anyone with it can edit as this agent.
      </p>
      <CopyBox label="Token" value={token} />
      <div className={frame.actions}>
        <span className={frame.spacer} />
        <button type="button" className={cx(ui.button, ui.primary)} onClick={onDone}>
          Done
        </button>
      </div>
    </div>
  );
}

type SnippetTab = 'claude' | 'json' | 'remote' | 'other';

const TABS: { value: SnippetTab; label: string }[] = [
  { value: 'claude', label: 'Claude Code' },
  { value: 'json', label: 'Cursor / VS Code / Windsurf' },
  { value: 'remote', label: 'Claude Desktop / stdio' },
  { value: 'other', label: 'Other' },
];

/** A client picker: one setup snippet per tab, with the real token once one's minted. */
function SetupSnippets({ origin, token }: { origin: string; token: string | null }) {
  const [tab, setTab] = useState<SnippetTab>('claude');
  const value = token ?? PLACEHOLDER_TOKEN;

  return (
    <div className={agent.snippets}>
      <h3>Setup</h3>
      <Tabs.Root value={tab} onValueChange={(v) => setTab(v as SnippetTab)}>
        <Tabs.List className={agent.tabs} aria-label="Client">
          {TABS.map((t) => (
            <Tabs.Trigger key={t.value} value={t.value} className={agent.tab}>
              {t.label}
            </Tabs.Trigger>
          ))}
        </Tabs.List>

        <Tabs.Content value="claude" className={agent.panel}>
          <CopyBox label="claude mcp add" value={mcpAddCommand(origin, value)} multiline />
        </Tabs.Content>

        <Tabs.Content value="json" className={agent.panel}>
          <CopyBox label="JSON config" value={mcpJsonConfig(origin, value)} multiline />
        </Tabs.Content>

        <Tabs.Content value="remote" className={agent.panel}>
          <CopyBox label="JSON config" value={mcpRemoteConfig(origin, value)} multiline />
        </Tabs.Content>

        <Tabs.Content value="other" className={agent.panel}>
          <CopyBox label="URL" value={mcpEndpoint(origin)} />
          <CopyBox label="Header" value={authHeader(value)} />
        </Tabs.Content>
      </Tabs.Root>

      {!token && <p className={ui.muted}>Create a token above; it is shown once.</p>}
    </div>
  );
}

function CopyBox({
  label,
  value,
  multiline = false,
}: {
  label: string;
  value: string;
  multiline?: boolean;
}) {
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
    <div className={cx(agent.copyBox, multiline && agent.copyBoxMultiline)}>
      <span className={cx(agent.copyLabel, ui.muted)}>{label}</span>
      <code className={agent.copyValue}>{value}</code>
      <button
        type="button"
        className={cx(ui.button, ui.ghost)}
        aria-label={`Copy ${label}`}
        onClick={copy}
      >
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  );
}

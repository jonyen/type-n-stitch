import { DropdownMenu, Popover } from 'radix-ui';
import { useState } from 'react';

import { cx } from '../cx';
import { formatTime } from '../editlist';
import type { ConnectionStatus, Peer } from '../realtime';
import ui from '../styles/ui.module.css';
import type { ThemeChoice } from '../theme';
import type { ProjectSummary, Transition, User } from '../types';
import { Avatar } from './Avatar';
import { Presence } from './Presence';
import { ThemeToggle } from './ThemeToggle';
import { Tip } from './Tip';
import styles from './TopBar.module.css';

export type ExportState =
  | { status: 'idle' }
  | { status: 'rendering'; progress: number }
  | { status: 'done'; url: string; duration: number; bytes: number }
  | { status: 'error'; message: string };

/** Everything the editor's top bar shows and does; null on the home screen. */
export interface EditorControls {
  readOnly: boolean;
  canUndo: boolean;
  canRedo: boolean;
  onUndo: () => void;
  onRedo: () => void;
  fillerCount: number;
  pauseCount: number;
  twoWordFillers: boolean;
  onRemoveFillers: () => void;
  onTightenPauses: () => void;
  onTwoWordFillers: (on: boolean) => void;
  hasSelection: boolean;
  onAddTitle: () => void;
  onAddCaption: () => void;
  onAddBroll: () => void;
  onAddMusic: () => void;
  onSplit: () => void;
  transition: Transition;
  onTransition: (transition: Transition) => void;
  peers: Peer[];
  status: ConnectionStatus;
  lastError: string | null;
  exportState: ExportState;
  onExport: () => void;
}

interface Props {
  user: User;
  project: ProjectSummary | null;
  editor: EditorControls | null;
  theme: ThemeChoice;
  onTheme: (choice: ThemeChoice) => void;
  onHome: () => void;
  onAgent: () => void;
  onSignOut: () => void;
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  if (n >= 1024) return `${Math.round(n / 1024)} KB`;
  return `${n} B`;
}

function plural(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`;
}

export function TopBar({
  user,
  project,
  editor,
  theme,
  onTheme,
  onHome,
  onAgent,
  onSignOut,
}: Props) {
  return (
    <header className={styles.bar}>
      <button type="button" className={styles.brand} onClick={onHome} title="All projects">
        <span className={styles.logo} aria-hidden />
        {project ? (
          <span className={styles.title}>
            <strong>{project.title}</strong>
            <span className={ui.muted}>{project.media.filename}</span>
          </span>
        ) : (
          <span className={styles.title}>
            <strong>type-n-stitch</strong>
            <span className={ui.muted}>edit media by editing its words</span>
          </span>
        )}
      </button>

      {editor && <EditorTools editor={editor} />}

      <span className={styles.spacer} />

      {editor && (
        <Presence peers={editor.peers} status={editor.status} lastError={editor.lastError} />
      )}
      <button type="button" className={cx(ui.button, ui.ghost)} onClick={onAgent}>
        Agent
      </button>
      {editor && <ExportButton editor={editor} />}

      <DropdownMenu.Root>
        <DropdownMenu.Trigger className={styles.account} aria-label="Account">
          <Avatar user={user} size="sm" />
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className={styles.menu} sideOffset={6} align="end">
            <DropdownMenu.Label className={styles.label}>{user.displayName}</DropdownMenu.Label>
            <DropdownMenu.Separator className={styles.separator} />
            <DropdownMenu.Label className={styles.label}>Theme</DropdownMenu.Label>
            <ThemeToggle value={theme} onChange={onTheme} itemClassName={styles.item} />
            <DropdownMenu.Separator className={styles.separator} />
            <DropdownMenu.Item className={styles.item} onSelect={onSignOut}>
              Sign out
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
    </header>
  );
}

function EditorTools({ editor }: { editor: EditorControls }) {
  const off = editor.readOnly;
  return (
    <div className={styles.group}>
      <Tip label="Undo (⌘Z)">
        <button
          type="button"
          className={cx(ui.iconButton, ui.ghost)}
          onClick={editor.onUndo}
          disabled={off || !editor.canUndo}
          aria-label="Undo"
        >
          ↶
        </button>
      </Tip>
      <Tip label="Redo (⇧⌘Z)">
        <button
          type="button"
          className={cx(ui.iconButton, ui.ghost)}
          onClick={editor.onRedo}
          disabled={off || !editor.canRedo}
          aria-label="Redo"
        >
          ↷
        </button>
      </Tip>
      <span className={styles.divider} />

      <DropdownMenu.Root>
        <DropdownMenu.Trigger className={cx(ui.button, ui.ghost)} disabled={off}>
          Clean up ▾
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className={styles.menu} sideOffset={6} align="start">
            <DropdownMenu.Item
              className={styles.item}
              disabled={editor.fillerCount === 0}
              onSelect={editor.onRemoveFillers}
            >
              Remove {plural(editor.fillerCount, 'filler')}
            </DropdownMenu.Item>
            <DropdownMenu.Item
              className={styles.item}
              disabled={editor.pauseCount === 0}
              onSelect={editor.onTightenPauses}
            >
              Tighten {plural(editor.pauseCount, 'pause')}
            </DropdownMenu.Item>
            <DropdownMenu.Separator className={styles.separator} />
            <DropdownMenu.CheckboxItem
              className={styles.item}
              checked={editor.twoWordFillers}
              onCheckedChange={(checked) => editor.onTwoWordFillers(checked === true)}
              onSelect={(e) => e.preventDefault()}
            >
              <DropdownMenu.ItemIndicator>✓ </DropdownMenu.ItemIndicator>
              Count “you know” and “I mean” as fillers
            </DropdownMenu.CheckboxItem>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>

      <DropdownMenu.Root>
        <DropdownMenu.Trigger className={cx(ui.button, ui.ghost)} disabled={off}>
          Insert ▾
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className={styles.menu} sideOffset={6} align="start">
            <DropdownMenu.Item className={styles.item} onSelect={editor.onAddTitle}>
              Title card
            </DropdownMenu.Item>
            <DropdownMenu.Item
              className={styles.item}
              disabled={!editor.hasSelection}
              onSelect={editor.onAddCaption}
            >
              Caption
            </DropdownMenu.Item>
            <DropdownMenu.Item
              className={styles.item}
              disabled={!editor.hasSelection}
              onSelect={editor.onAddBroll}
            >
              B-roll…
            </DropdownMenu.Item>
            <DropdownMenu.Item className={styles.item} onSelect={editor.onAddMusic}>
              Music…
            </DropdownMenu.Item>
            <DropdownMenu.Separator className={styles.separator} />
            <DropdownMenu.Item className={styles.item} onSelect={editor.onSplit}>
              Split here
              <span className={styles.hint}>at the selection, else the playhead</span>
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>

      <label className={styles.transition}>
        <span className={ui.muted}>Transitions</span>
        <select
          value={editor.transition}
          disabled={off}
          aria-label="Transitions"
          onChange={(e) => editor.onTransition(e.target.value as Transition)}
        >
          <option value="none">Jump cut</option>
          <option value="dip">Dip to black</option>
          <option value="crossfade" disabled>
            Crossfade (soon)
          </option>
        </select>
      </label>
    </div>
  );
}

function ExportButton({ editor }: { editor: EditorControls }) {
  const [open, setOpen] = useState(false);
  const state = editor.exportState;
  const rendering = state.status === 'rendering';
  return (
    <Popover.Root open={open && state.status !== 'idle'} onOpenChange={setOpen}>
      <Popover.Anchor asChild>
        <button
          type="button"
          className={cx(ui.button, ui.primary)}
          disabled={editor.readOnly || rendering}
          onClick={() => {
            setOpen(true);
            editor.onExport();
          }}
        >
          {rendering ? (
            <>
              <span className={ui.spinnerSmall} aria-hidden /> Rendering…
            </>
          ) : (
            'Export'
          )}
        </button>
      </Popover.Anchor>
      <Popover.Portal>
        <Popover.Content
          className={styles.popover}
          align="end"
          sideOffset={8}
          onOpenAutoFocus={(e) => e.preventDefault()}
        >
          {state.status === 'rendering' && (
            <div className={styles.progress}>
              <span>Rendering {Math.round(state.progress * 100)}%</span>
              <div
                className={styles.track}
                role="progressbar"
                aria-valuenow={Math.round(state.progress * 100)}
                aria-valuemin={0}
                aria-valuemax={100}
              >
                <div className={styles.fill} style={{ width: `${state.progress * 100}%` }} />
              </div>
            </div>
          )}
          {state.status === 'done' && (
            <div className={styles.progress}>
              <span>
                Rendered {formatTime(state.duration)} · {formatBytes(state.bytes)}
              </span>
              <a className={cx(ui.button, ui.primary)} href={state.url} download>
                Download {state.url.split('/').pop()}
              </a>
            </div>
          )}
          {state.status === 'error' && <p className={ui.error}>{state.message}</p>}
          <Popover.Close className={cx(ui.iconButton, ui.ghost, styles.close)} aria-label="Close">
            ✕
          </Popover.Close>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

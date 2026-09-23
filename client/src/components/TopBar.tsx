import { DropdownMenu, Popover } from 'radix-ui';
import { useEffect, useRef, useState } from 'react';

import { cx } from '../cx';
import { formatTime } from '../editlist';
import { MEDIA_ACCEPT } from '../importQueue';
import type { ConnectionStatus, Peer } from '../realtime';
import ui from '../styles/ui.module.css';
import type { ThemeChoice } from '../theme';
import type { ProjectSummary, Transition, User } from '../types';
import { Avatar } from './Avatar';
import { Presence } from './Presence';
import { ThemeToggle } from './ThemeToggle';
import { Tip } from './Tip';
import styles from './TopBar.module.css';

/** A finished render carries `seq`, the document version (`headSeq`) it was started from. */
export type ExportState =
  | { status: 'idle' }
  | { status: 'rendering'; progress: number }
  | { status: 'done'; url: string; duration: number; bytes: number; seq: number }
  | { status: 'error'; message: string; seq: number };

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
  onAddLayer: () => void;
  onAddMusic: () => void;
  /** Append these files to the end of the main track. */
  onAddVideos: (files: File[]) => void;
  /** An Add video… is still uploading; another waits for it. */
  addingVideos: boolean;
  onOverdub: () => void;
  onSplit: () => void;
  transition: Transition;
  onTransition: (transition: Transition) => void;
  peers: Peer[];
  status: ConnectionStatus;
  lastError: string | null;
  exportState: ExportState;
  /** The document version now; a finished render of another one is out of date. */
  headSeq: number;
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
        Connect AI
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
            <DropdownMenu.Item className={styles.item} onSelect={onAgent}>
              Connect AI…
            </DropdownMenu.Item>
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
  const videoInput = useRef<HTMLInputElement>(null);
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
            <DropdownMenu.Item
              className={styles.item}
              disabled={editor.addingVideos}
              onSelect={() => videoInput.current?.click()}
            >
              Add video…
              <span className={styles.hint}>appended at the end</span>
            </DropdownMenu.Item>
            <DropdownMenu.Separator className={styles.separator} />
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
              onSelect={editor.onAddLayer}
            >
              Layer…
            </DropdownMenu.Item>
            <DropdownMenu.Item className={styles.item} onSelect={editor.onAddMusic}>
              Music…
            </DropdownMenu.Item>
            <DropdownMenu.Item
              className={styles.item}
              disabled={!editor.hasSelection}
              onSelect={editor.onOverdub}
            >
              Overdub…
            </DropdownMenu.Item>
            <DropdownMenu.Separator className={styles.separator} />
            <DropdownMenu.Item className={styles.item} onSelect={editor.onSplit}>
              Split here
              <span className={styles.hint}>at the selection, else the playhead</span>
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
      <input
        ref={videoInput}
        type="file"
        accept={MEDIA_ACCEPT}
        multiple
        hidden
        data-testid="add-video-input"
        onChange={(e) => {
          const files = Array.from(e.target.files ?? []);
          e.target.value = '';
          if (files.length > 0) editor.onAddVideos(files);
        }}
      />

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
  // The edit has changed since this render finished: Export renders afresh.
  const stale =
    (state.status === 'done' || state.status === 'error') && state.seq !== editor.headSeq;

  // A render that finishes (or fails) while the popover is closed still
  // needs to be seen: bring it back rather than leaving the result stranded
  // behind the button.
  useEffect(() => {
    if (state.status === 'done' || state.status === 'error') setOpen(true);
  }, [state.status]);

  return (
    <Popover.Root open={open && state.status !== 'idle'} onOpenChange={setOpen}>
      <Popover.Anchor asChild>
        <button
          type="button"
          className={cx(ui.button, ui.primary)}
          disabled={editor.readOnly}
          onClick={() => {
            // A current result (or a render in flight) is only reopened, not
            // started again; "Export again" inside the popover does that. A
            // result of an older edit is replaced.
            if (state.status === 'idle' || stale) editor.onExport();
            setOpen(true);
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
          onInteractOutside={(e) => {
            // A render in flight must not be dismissed and lost.
            if (rendering) e.preventDefault();
          }}
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
          {stale && (
            <p className={ui.muted}>Out of date: the edit has changed since this render.</p>
          )}
          {state.status === 'done' && (
            <div className={styles.progress}>
              <span>
                Rendered {formatTime(state.duration)} · {formatBytes(state.bytes)}
              </span>
              <a className={cx(ui.button, ui.primary)} href={state.url} download>
                Download {state.url.split('/').pop()}
              </a>
              <button type="button" className={cx(ui.button, ui.ghost)} onClick={editor.onExport}>
                Export again
              </button>
            </div>
          )}
          {state.status === 'error' && (
            <div className={styles.progress}>
              <p className={ui.error}>{state.message}</p>
              <button type="button" className={cx(ui.button, ui.ghost)} onClick={editor.onExport}>
                Export again
              </button>
            </div>
          )}
          <Popover.Close className={cx(ui.iconButton, ui.ghost, styles.close)} aria-label="Close">
            ✕
          </Popover.Close>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

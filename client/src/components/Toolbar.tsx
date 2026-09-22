import { formatTime } from '../editlist';
import type { Transition } from '../types';

export type ExportState =
  | { status: 'idle' }
  | { status: 'rendering'; progress: number }
  | { status: 'done'; url: string; duration: number; bytes: number }
  | { status: 'error'; message: string };

interface Props {
  hasSelection: boolean;
  /** A title card is selected instead of words; Delete removes it. */
  hasTitleSelection: boolean;
  /** A clip divider is selected; Delete removes that split. */
  hasClipSelection: boolean;
  canUndo: boolean;
  canRedo: boolean;
  /** Viewers and commenters see the editor, but cannot change it. */
  readOnly: boolean;
  /** Filler words and long pauses not yet cut. */
  fillerCount: number;
  pauseCount: number;
  twoWordFillers: boolean;
  showCuts: boolean;
  exportState: ExportState;
  /** The project-wide transition between output pieces. */
  transition: Transition;
  onDelete: () => void;
  onAddTitle: () => void;
  onAddCaption: () => void;
  onSplit: () => void;
  onAddBroll: () => void;
  onAddMusic: () => void;
  onTransition: (transition: Transition) => void;
  onOverdub: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onRemoveFillers: () => void;
  onTightenPauses: () => void;
  onTwoWordFillers: (on: boolean) => void;
  onShowCuts: (on: boolean) => void;
  onExport: () => void;
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  if (n >= 1024) return `${Math.round(n / 1024)} KB`;
  return `${n} B`;
}

function plural(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`;
}

export function Toolbar({
  hasSelection,
  hasTitleSelection,
  hasClipSelection,
  canUndo,
  canRedo,
  readOnly,
  fillerCount,
  pauseCount,
  twoWordFillers,
  showCuts,
  exportState,
  transition,
  onDelete,
  onAddTitle,
  onAddCaption,
  onSplit,
  onAddBroll,
  onAddMusic,
  onTransition,
  onOverdub,
  onUndo,
  onRedo,
  onRemoveFillers,
  onTightenPauses,
  onTwoWordFillers,
  onShowCuts,
  onExport,
}: Props) {
  const rendering = exportState.status === 'rendering';
  return (
    <div className="toolbar">
      <div className="actions">
        <button
          type="button"
          onClick={onDelete}
          disabled={readOnly || (!hasSelection && !hasTitleSelection && !hasClipSelection)}
          title="Delete / Backspace"
        >
          Delete
        </button>
        <button
          type="button"
          className="accent"
          onClick={onOverdub}
          disabled={readOnly || !hasSelection}
          title="Replace the selected words with synthesized speech"
        >
          Overdub
        </button>
        <button type="button" onClick={onUndo} disabled={readOnly || !canUndo} title="⌘Z / Ctrl-Z">
          Undo
        </button>
        <button
          type="button"
          onClick={onRedo}
          disabled={readOnly || !canRedo}
          title="⇧⌘Z / Ctrl-Shift-Z"
        >
          Redo
        </button>
        <span className="spacer" />
        <button
          type="button"
          className="primary"
          onClick={onExport}
          disabled={readOnly || rendering}
        >
          {rendering ? (
            <>
              <span className="spinner small" aria-hidden /> Rendering…
            </>
          ) : (
            'Export'
          )}
        </button>
      </div>

      <div className="actions">
        <button
          type="button"
          onClick={onRemoveFillers}
          disabled={readOnly || fillerCount === 0}
          title="Cut every um, uh, hmm… in one step"
        >
          Remove {plural(fillerCount, 'filler')}
        </button>
        <button
          type="button"
          onClick={onTightenPauses}
          disabled={readOnly || pauseCount === 0}
          title="Shorten every pause over 0.6 s to 0.25 s"
        >
          Tighten {plural(pauseCount, 'pause')}
        </button>
        <label className="toggle" title="Also treat “you know” and “I mean” as fillers">
          <input
            type="checkbox"
            checked={twoWordFillers}
            onChange={(e) => onTwoWordFillers(e.target.checked)}
          />
          + “you know”
        </label>
        <button
          type="button"
          onClick={onAddTitle}
          disabled={readOnly}
          title="Insert a full-frame title card at the selection, or at the playhead"
        >
          Add title
        </button>
        <button
          type="button"
          onClick={onAddCaption}
          disabled={readOnly || !hasSelection}
          title="Draw text over the picture while the selected words play"
        >
          Add caption
        </button>
        <button
          type="button"
          onClick={onSplit}
          disabled={readOnly}
          title="Split into clips at the selected word, or at the playhead"
        >
          Split here
        </button>
        <button
          type="button"
          onClick={onAddBroll}
          disabled={readOnly || !hasSelection}
          title="Show a video shot over the selected words"
        >
          Add B-roll
        </button>
        <button
          type="button"
          onClick={onAddMusic}
          disabled={readOnly}
          title="Play music under the selected words, or the whole edit"
        >
          Add music
        </button>
        <label className="toggle" title="How the pieces either side of a cut meet">
          Transitions
          <select
            value={transition}
            disabled={readOnly}
            aria-label="Transitions"
            onChange={(e) => onTransition(e.target.value as Transition)}
          >
            <option value="none">Jump cut</option>
            <option value="dip">Dip to black</option>
            <option value="crossfade" disabled>
              Crossfade (soon)
            </option>
          </select>
        </label>
        <span className="spacer" />
        <label className="toggle" title="Off: read the transcript as the output will sound">
          <input
            type="checkbox"
            checked={showCuts}
            onChange={(e) => onShowCuts(e.target.checked)}
          />
          Show cuts
        </label>
      </div>

      {exportState.status === 'rendering' && (
        <div
          className="export-progress"
          role="progressbar"
          aria-valuenow={Math.round(exportState.progress * 100)}
        >
          <div className="bar">
            <div className="fill" style={{ width: `${Math.round(exportState.progress * 100)}%` }} />
          </div>
          <span className="muted">{Math.round(exportState.progress * 100)}%</span>
        </div>
      )}
      {exportState.status === 'done' && (
        <p className="export-result">
          Rendered {formatTime(exportState.duration)} · {formatBytes(exportState.bytes)} ·{' '}
          <a href={exportState.url} download>
            Download {exportState.url.split('/').pop()}
          </a>
        </p>
      )}
      {exportState.status === 'error' && <p className="error">{exportState.message}</p>}

      <ul className="legend">
        <li>
          <span className="swatch selected" /> selected
        </li>
        <li>
          <span className="swatch cut" /> cut
        </li>
        <li>
          <span className="swatch overdub" /> overdub
        </li>
        <li>
          <span className="swatch active" /> playing
        </li>
        <li className="muted">click a word to seek · shift-click to extend · space plays</li>
      </ul>
    </div>
  );
}

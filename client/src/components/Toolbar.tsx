import { formatTime } from '../editlist';

export type ExportState =
  | { status: 'idle' }
  | { status: 'rendering' }
  | { status: 'done'; url: string; duration: number }
  | { status: 'error'; message: string };

interface Props {
  hasSelection: boolean;
  canUndo: boolean;
  /** Filler words and long pauses not yet cut. */
  fillerCount: number;
  pauseCount: number;
  twoWordFillers: boolean;
  showCuts: boolean;
  exportState: ExportState;
  onDelete: () => void;
  onOverdub: () => void;
  onUndo: () => void;
  onRemoveFillers: () => void;
  onTightenPauses: () => void;
  onTwoWordFillers: (on: boolean) => void;
  onShowCuts: (on: boolean) => void;
  onExport: () => void;
}

function plural(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`;
}

export function Toolbar({
  hasSelection,
  canUndo,
  fillerCount,
  pauseCount,
  twoWordFillers,
  showCuts,
  exportState,
  onDelete,
  onOverdub,
  onUndo,
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
          disabled={!hasSelection}
          title="Delete / Backspace"
        >
          Delete
        </button>
        <button
          type="button"
          className="accent"
          onClick={onOverdub}
          disabled={!hasSelection}
          title="Replace the selected words with synthesized speech"
        >
          Overdub
        </button>
        <button type="button" onClick={onUndo} disabled={!canUndo} title="⌘Z / Ctrl-Z">
          Undo
        </button>
        <span className="spacer" />
        <button type="button" className="primary" onClick={onExport} disabled={rendering}>
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
          disabled={fillerCount === 0}
          title="Cut every um, uh, hmm… in one step"
        >
          Remove {plural(fillerCount, 'filler')}
        </button>
        <button
          type="button"
          onClick={onTightenPauses}
          disabled={pauseCount === 0}
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
          “you know” / “I mean”
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

      {exportState.status === 'done' && (
        <p className="export-result">
          Rendered {formatTime(exportState.duration)} ·{' '}
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

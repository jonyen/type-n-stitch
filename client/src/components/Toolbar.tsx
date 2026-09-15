import { formatTime } from '../editlist';

export type ExportState =
  | { status: 'idle' }
  | { status: 'rendering' }
  | { status: 'done'; url: string; duration: number }
  | { status: 'error'; message: string };

interface Props {
  hasSelection: boolean;
  canUndo: boolean;
  exportState: ExportState;
  onDelete: () => void;
  onOverdub: () => void;
  onUndo: () => void;
  onExport: () => void;
}

export function Toolbar({
  hasSelection,
  canUndo,
  exportState,
  onDelete,
  onOverdub,
  onUndo,
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

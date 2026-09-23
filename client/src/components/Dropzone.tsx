import { useRef, useState, type DragEvent, type ReactNode } from 'react';

import { cx } from '../cx';
import { byName, itemsFor, MEDIA_ACCEPT, moveItem, type ImportItem } from '../importQueue';
import ui from '../styles/ui.module.css';
import type { LibraryItem } from '../types';
import styles from './Home.module.css';
import { ImportList } from './ImportList';
import { Library } from './Library';

interface Props {
  /** Import these files as one project, in this order. */
  onImport: (files: File[]) => void;
  onLibraryClip: (item: LibraryItem) => void;
  busy: string | null;
  error: string | null;
  /** The import under way, file by file; empty when none is running. */
  imports: ImportItem[];
  /** Rendered between the how-to steps and the sample library. */
  children?: ReactNode;
}

export function Dropzone({ onImport, onLibraryClip, busy, error, imports, children }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);
  // Chosen but not yet imported, in the order they will be stitched.
  const [staged, setStaged] = useState<File[]>([]);

  const add = (list: FileList | File[] | null | undefined) => {
    const files = Array.from(list ?? []);
    if (files.length === 0) return;
    // One file with nothing staged opens straight away, as it always has.
    if (files.length === 1 && staged.length === 0) {
      onImport(files);
      return;
    }
    setStaged([...staged, ...byName(files)]);
  };

  const start = () => {
    const files = staged;
    setStaged([]);
    onImport(files);
  };

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setOver(false);
    add(e.dataTransfer.files);
  };

  return (
    <div className={styles.wrap}>
      <button
        type="button"
        className={cx(styles.dropzone, over && styles.over, busy && styles.busy)}
        onClick={() => input.current?.click()}
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={onDrop}
        disabled={busy !== null}
      >
        <input
          ref={input}
          type="file"
          accept={MEDIA_ACCEPT}
          multiple
          hidden
          data-testid="media-input"
          onChange={(e) => {
            add(e.target.files);
            e.target.value = '';
          }}
        />
        {busy ? (
          <>
            <span className={ui.spinner} aria-hidden />
            <strong>{busy}</strong>
            <span className={ui.muted}>whisper.cpp is listening, hold on…</span>
          </>
        ) : staged.length > 0 ? (
          <>
            <strong>Drop more files</strong>
            <span className={ui.muted}>or click to add them; drag the list to set their order</span>
          </>
        ) : (
          <>
            <strong>Drop audio or video files</strong>
            <span className={ui.muted}>
              mp3, wav, m4a, mp4, mov, or click to browse. Several make one project, in order.
            </span>
          </>
        )}
      </button>
      {error && <p className={ui.error}>{error}</p>}
      {imports.length > 0 ? (
        <ImportList items={imports} />
      ) : (
        staged.length > 0 && (
          <div className={styles.staged}>
            <ImportList
              items={itemsFor(staged)}
              onMove={(from, to) => setStaged(moveItem(staged, from, to))}
              onRemove={(index) => setStaged(staged.filter((_, k) => k !== index))}
            />
            <div className={styles.stagedActions}>
              <button
                type="button"
                className={cx(ui.button, ui.ghost)}
                onClick={() => setStaged([])}
              >
                Clear
              </button>
              <button
                type="button"
                className={cx(ui.button, ui.primary)}
                onClick={start}
                disabled={busy !== null}
              >
                Import {staged.length === 1 ? '1 file' : `${staged.length} files`}
              </button>
            </div>
          </div>
        )
      )}
      <ol className={styles.how}>
        <li>Transcribe with word timestamps</li>
        <li>Select words, press Delete to cut them</li>
        <li>Overdub a phrase in a cloned voice</li>
        <li>Export the stitched result</li>
      </ol>
      {children}
      <Library onOpen={onLibraryClip} disabled={busy !== null} />
    </div>
  );
}

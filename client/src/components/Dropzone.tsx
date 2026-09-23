import { useRef, useState, type DragEvent, type ReactNode } from 'react';

import type { LibraryItem } from '../types';
import { cx } from '../cx';
import { Library } from './Library';
import styles from './Home.module.css';
import ui from '../styles/ui.module.css';

const ACCEPT = '.mp3,.wav,.m4a,.mp4,.mov,audio/*,video/*';

interface Props {
  onFile: (file: File) => void;
  onLibraryClip: (item: LibraryItem) => void;
  busy: string | null;
  error: string | null;
  /** Rendered between the how-to steps and the sample library. */
  children?: ReactNode;
}

export function Dropzone({ onFile, onLibraryClip, busy, error, children }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setOver(false);
    const file = e.dataTransfer.files[0];
    if (file) onFile(file);
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
          accept={ACCEPT}
          hidden
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) onFile(file);
            e.target.value = '';
          }}
        />
        {busy ? (
          <>
            <span className={ui.spinner} aria-hidden />
            <strong>{busy}</strong>
            <span className={ui.muted}>whisper.cpp is listening, hold on…</span>
          </>
        ) : (
          <>
            <strong>Drop an audio or video file</strong>
            <span className={ui.muted}>mp3, wav, m4a, mp4, mov, or click to browse</span>
          </>
        )}
      </button>
      {error && <p className={ui.error}>{error}</p>}
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

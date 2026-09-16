import { useRef, useState, type DragEvent } from 'react';

import type { LibraryItem } from '../types';
import { Library } from './Library';

const ACCEPT = '.mp3,.wav,.m4a,.mp4,.mov,audio/*,video/*';

interface Props {
  onFile: (file: File) => void;
  onLibraryClip: (item: LibraryItem) => void;
  busy: string | null;
  error: string | null;
}

export function Dropzone({ onFile, onLibraryClip, busy, error }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setOver(false);
    const file = e.dataTransfer.files[0];
    if (file) onFile(file);
  };

  return (
    <div className="dropzone-wrap">
      <button
        type="button"
        className={`dropzone${over ? ' over' : ''}${busy ? ' busy' : ''}`}
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
            <span className="spinner" aria-hidden />
            <strong>{busy}</strong>
            <span className="muted">whisper.cpp is listening, hold on…</span>
          </>
        ) : (
          <>
            <strong>Drop an audio or video file</strong>
            <span className="muted">mp3, wav, m4a, mp4, mov, or click to browse</span>
          </>
        )}
      </button>
      {error && <p className="error">{error}</p>}
      <ol className="how">
        <li>Transcribe with word timestamps</li>
        <li>Select words, press Delete to cut them</li>
        <li>Overdub a phrase in a cloned voice</li>
        <li>Export the stitched result</li>
      </ol>
      <Library onOpen={onLibraryClip} disabled={busy !== null} />
    </div>
  );
}

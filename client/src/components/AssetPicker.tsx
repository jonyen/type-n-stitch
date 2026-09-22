import { useEffect, useRef, useState } from 'react';
import { formatTime } from '../editlist';
import type { Asset, MediaKind } from '../types';

interface Props {
  assets: Asset[];
  kind: MediaKind;
  value: Asset | null;
  onChange: (a: Asset) => void;
  onUpload: (f: File) => Promise<Asset>;
  /** Focus the first interactive control (a card, or the upload button) on mount. */
  autoFocus?: boolean;
}

export function AssetPicker({ assets, kind, value, onChange, onUpload, autoFocus }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const list = assets.filter((a) => a.kind === kind);
  useEffect(() => {
    if (autoFocus) first.current?.focus();
  }, [autoFocus]);
  const upload = async (file: File) => {
    setBusy(true);
    setError(null);
    try {
      onChange(await onUpload(file));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="asset-picker">
      <div className="asset-grid" role="radiogroup">
        {list.map((a, i) => (
          <button
            key={a.id}
            ref={i === 0 ? first : undefined}
            type="button"
            role="radio"
            aria-checked={value?.id === a.id}
            className={`asset-card${value?.id === a.id ? ' selected' : ''}`}
            disabled={busy}
            onClick={() => onChange(a)}
          >
            <span
              className={`poster ${a.kind}`}
              style={a.poster ? { backgroundImage: `url(${a.poster})` } : undefined}
            >
              {!a.poster && (
                <span className="poster-glyph" aria-hidden>
                  ♪
                </span>
              )}
              <span className="length">{formatTime(a.duration)}</span>
            </span>
            <span className="asset-name">{a.name}</span>
          </button>
        ))}
        <button
          ref={list.length === 0 ? first : undefined}
          type="button"
          className="asset-card add"
          disabled={busy}
          onClick={() => input.current?.click()}
        >
          {busy ? <span className="spinner small" aria-hidden /> : '+'}{' '}
          {busy ? 'Uploading…' : `Upload ${kind}`}
        </button>
      </div>
      <input
        ref={input}
        type="file"
        hidden
        accept={kind === 'video' ? 'video/*,.mp4,.mov' : 'audio/*,.mp3,.wav,.m4a'}
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) void upload(f);
          e.target.value = '';
        }}
      />
      {error && <p className="error">{error}</p>}
      {list.length === 0 && !busy && <p className="muted">No {kind} assets yet — upload one.</p>}
    </div>
  );
}

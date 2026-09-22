import { useRef, useState } from 'react';
import { formatTime } from '../editlist';
import type { Asset, MediaKind } from '../types';

interface Props {
  assets: Asset[];
  kind: MediaKind;
  value: Asset | null;
  onChange: (a: Asset) => void;
  onUpload: (f: File) => Promise<Asset>;
}

export function AssetPicker({ assets, kind, value, onChange, onUpload }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const list = assets.filter((a) => a.kind === kind);
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
        {list.map((a) => (
          <button
            key={a.id}
            type="button"
            role="radio"
            aria-checked={value?.id === a.id}
            className={`asset-card${value?.id === a.id ? ' selected' : ''}`}
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

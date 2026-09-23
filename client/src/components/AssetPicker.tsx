import { useEffect, useRef, useState } from 'react';
import { cx } from '../cx';
import { formatTime } from '../editlist';
import type { Asset, MediaKind, SourceView } from '../types';
import picker from './AssetPicker.module.css';
import ui from '../styles/ui.module.css';

interface Props {
  assets: Asset[];
  /** The project's own videos, offered first so a stretch of one can go over another. */
  sources?: SourceView[] | undefined;
  kind: MediaKind;
  value: Asset | null;
  onChange: (a: Asset) => void;
  onUpload: (f: File) => Promise<Asset>;
  /** Focus the first interactive control (a card, or the upload button) on mount. */
  autoFocus?: boolean;
}

/**
 * A project source as a pickable card. Its id is the source's media id, which
 * is what a layer taken from it names; its offset is seconds into that file.
 */
export function sourceAsset(s: SourceView): Asset {
  return {
    id: s.mediaId,
    kind: s.kind,
    name: `Video ${s.index + 1} · ${s.filename}`,
    ext: s.filename.split('.').pop() ?? '',
    duration: s.duration,
    width: null,
    height: null,
    createdAt: 0,
    url: s.url,
    poster: null,
  };
}

export function AssetPicker({
  assets,
  sources,
  kind,
  value,
  onChange,
  onUpload,
  autoFocus,
}: Props) {
  const input = useRef<HTMLInputElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const own = (sources ?? []).filter((s) => s.kind === kind).map(sourceAsset);
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
  const card = (a: Asset, isFirst: boolean) => (
    <button
      key={a.id}
      ref={isFirst ? first : undefined}
      type="button"
      role="radio"
      aria-checked={value?.id === a.id}
      className={cx(picker.card, value?.id === a.id && picker.selected)}
      disabled={busy}
      onClick={() => onChange(a)}
    >
      <span
        className={picker.poster}
        style={a.poster ? { backgroundImage: `url(${a.poster})` } : undefined}
      >
        {!a.poster && (
          <span className={picker.glyph} aria-hidden>
            {a.kind === 'video' ? '▶' : '♪'}
          </span>
        )}
        <span className={picker.length}>{formatTime(a.duration)}</span>
      </span>
      <span className={picker.name}>{a.name}</span>
    </button>
  );
  return (
    <div className={picker.picker}>
      {own.length > 0 && (
        <>
          <p className={picker.heading} aria-hidden>
            This project’s videos
          </p>
          <div className={picker.grid} role="radiogroup" aria-label="This project’s videos">
            {own.map((a, i) => card(a, i === 0))}
          </div>
          <p className={picker.heading} aria-hidden>
            Uploads
          </p>
        </>
      )}
      <div
        className={picker.grid}
        role="radiogroup"
        aria-label={own.length > 0 ? 'Uploads' : undefined}
      >
        {list.map((a, i) => card(a, own.length === 0 && i === 0))}
        <button
          ref={own.length === 0 && list.length === 0 ? first : undefined}
          type="button"
          className={cx(picker.card, picker.add)}
          disabled={busy}
          onClick={() => input.current?.click()}
        >
          {busy ? <span className={ui.spinnerSmall} aria-hidden /> : '+'}{' '}
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
      {error && <p className={ui.error}>{error}</p>}
      {list.length === 0 && own.length === 0 && !busy && (
        <p className={ui.muted}>No {kind} assets yet — upload one.</p>
      )}
    </div>
  );
}

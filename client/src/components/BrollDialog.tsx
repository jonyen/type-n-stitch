import { useEffect, useRef, useState, type FormEvent } from 'react';
import { formatTime } from '../editlist';
import type { Asset } from '../types';
import { AssetPicker } from './AssetPicker';

interface Props {
  assets: Asset[];
  rangeLength: number;
  original: string;
  onUpload: (f: File) => Promise<Asset>;
  onSubmit: (asset: Asset, offset: number) => void;
  onCancel: () => void;
}

export function BrollDialog({
  assets,
  rangeLength,
  original,
  onUpload,
  onSubmit,
  onCancel,
}: Props) {
  const [asset, setAsset] = useState<Asset | null>(null);
  const [offset, setOffset] = useState(0);
  const preview = useRef<HTMLVideoElement>(null);
  const max = asset ? Math.max(0, asset.duration - rangeLength) : 0;
  const tooShort = asset !== null && asset.duration + 1e-6 < rangeLength;
  useEffect(() => {
    setOffset(0);
  }, [asset]);
  useEffect(() => {
    if (preview.current) preview.current.currentTime = offset;
  }, [offset, asset]);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (asset && !tooShort) onSubmit(asset, offset);
  };
  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <form
        className="modal wide"
        onClick={(e) => e.stopPropagation()}
        onSubmit={submit}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.stopPropagation();
            onCancel();
          }
        }}
      >
        <h2>Add B-roll</h2>
        <p className="muted">
          Cover the picture while “{original}” plays ({formatTime(rangeLength)}). The voice keeps
          going.
        </p>
        <AssetPicker
          assets={assets}
          kind="video"
          value={asset}
          onChange={setAsset}
          onUpload={onUpload}
        />
        {asset && (
          <>
            <video
              ref={preview}
              className="asset-preview"
              src={asset.url}
              muted
              playsInline
              preload="auto"
            />
            <label className="field">
              Start {formatTime(offset)} into the shot
              <input
                type="range"
                min={0}
                max={max}
                step={0.1}
                value={offset}
                disabled={max === 0}
                onChange={(e) => setOffset(Number(e.target.value))}
              />
            </label>
            {tooShort && <p className="error">This shot is shorter than the selected words.</p>}
          </>
        )}
        <div className="actions">
          <span className="spacer" />
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={!asset || tooShort}>
            Add B-roll
          </button>
        </div>
      </form>
    </div>
  );
}

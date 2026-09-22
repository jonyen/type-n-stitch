import { useEffect, useRef, useState, type FormEvent } from 'react';
import type { Asset, AudioEdit } from '../types';
import { AssetPicker } from './AssetPicker';

interface Props {
  assets: Asset[];
  initial?: AudioEdit | undefined;
  wholeEdit: boolean;
  onUpload: (f: File) => Promise<Asset>;
  onSubmit: (asset: Asset | null, gain: number, duck: boolean) => void;
  onRemove?: (() => void) | undefined;
  onCancel: () => void;
}

export function AudioDialog({
  assets,
  initial,
  wholeEdit,
  onUpload,
  onSubmit,
  onRemove,
  onCancel,
}: Props) {
  const [asset, setAsset] = useState<Asset | null>(
    initial ? (assets.find((a) => a.id === initial.media) ?? null) : null,
  );
  const [gain, setGain] = useState(String(initial?.gain ?? 0));
  const [duck, setDuck] = useState(initial?.duck ?? true);
  const gainInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    gainInput.current?.focus();
    gainInput.current?.select();
  }, []);
  const db = Number(gain);
  const validGain = Number.isFinite(db) && db >= -30 && db <= 12;
  const ready = validGain && (initial !== undefined || asset !== null);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (ready) onSubmit(initial ? null : asset, db, duck);
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
        <h2>{initial ? 'Music' : 'Add music'}</h2>
        <p className="muted">
          {initial
            ? `Playing ${asset?.name ?? initial.media}.`
            : wholeEdit
              ? 'Under the whole edit.'
              : 'Under the selected words.'}
        </p>
        {!initial && (
          <AssetPicker
            assets={assets}
            kind="audio"
            value={asset}
            onChange={setAsset}
            onUpload={onUpload}
          />
        )}
        <label className="field">
          Level (dB, −30 to 12)
          <input
            ref={gainInput}
            type="number"
            min={-30}
            max={12}
            step={1}
            value={gain}
            onChange={(e) => setGain(e.target.value)}
          />
        </label>
        {validGain && db > 0 && (
          <p className="muted">
            Boost above 0 dB is applied on export; the preview cannot play louder than the source.
          </p>
        )}
        <label className="toggle">
          <input type="checkbox" checked={duck} onChange={(e) => setDuck(e.target.checked)} /> Duck
          under speech
        </label>
        <div className="actions">
          {initial && onRemove && (
            <button type="button" className="danger" onClick={onRemove}>
              Remove
            </button>
          )}
          <span className="spacer" />
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={!ready}>
            {initial ? 'Save' : 'Add music'}
          </button>
        </div>
      </form>
    </div>
  );
}

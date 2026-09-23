import { useEffect, useRef, useState, type FormEvent } from 'react';
import { cx } from '../cx';
import type { Asset, AudioEdit } from '../types';
import { AssetPicker } from './AssetPicker';
import { DialogFrame } from './DialogFrame';
import frame from './DialogFrame.module.css';
import ui from '../styles/ui.module.css';

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
    <DialogFrame
      title={initial ? 'Music' : 'Add music'}
      description={
        initial
          ? `Playing ${asset?.name ?? initial.media}.`
          : wholeEdit
            ? 'Under the whole edit.'
            : 'Under the selected words.'
      }
      wide
      onClose={onCancel}
    >
      <form className={frame.form} onSubmit={submit}>
        {!initial && (
          <AssetPicker
            assets={assets}
            kind="audio"
            value={asset}
            onChange={setAsset}
            onUpload={onUpload}
          />
        )}
        <label className={ui.field}>
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
          <p className={ui.muted}>
            Boost above 0 dB is applied on export; the preview cannot play louder than the source.
          </p>
        )}
        <label className={ui.toggle}>
          <input type="checkbox" checked={duck} onChange={(e) => setDuck(e.target.checked)} /> Duck
          under speech
        </label>
        <div className={frame.actions}>
          {initial && onRemove && (
            <button type="button" className={cx(ui.button, ui.danger)} onClick={onRemove}>
              Remove
            </button>
          )}
          <span className={frame.spacer} />
          <button type="button" className={ui.button} onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className={cx(ui.button, ui.primary)} disabled={!ready}>
            {initial ? 'Save' : 'Add music'}
          </button>
        </div>
      </form>
    </DialogFrame>
  );
}

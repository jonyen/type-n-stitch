import { useEffect, useRef, useState, type FormEvent } from 'react';
import { cx } from '../cx';
import { formatTime } from '../editlist';
import type { Asset } from '../types';
import { AssetPicker } from './AssetPicker';
import { DialogFrame } from './DialogFrame';
import frame from './DialogFrame.module.css';
import ui from '../styles/ui.module.css';
import brollStyles from './BrollDialog.module.css';

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
    <DialogFrame
      title="Add B-roll"
      description={
        <>
          Cover the picture while “{original}” plays ({formatTime(rangeLength)}). The voice keeps
          going.
        </>
      }
      wide
      onClose={onCancel}
    >
      <form className={frame.form} onSubmit={submit}>
        <AssetPicker
          assets={assets}
          kind="video"
          value={asset}
          onChange={setAsset}
          onUpload={onUpload}
          autoFocus
        />
        {asset && (
          <>
            <video
              ref={preview}
              className={brollStyles.preview}
              src={asset.url}
              muted
              playsInline
              preload="auto"
            />
            <label className={ui.field}>
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
            {tooShort && <p className={ui.error}>This shot is shorter than the selected words.</p>}
          </>
        )}
        <div className={frame.actions}>
          <span className={frame.spacer} />
          <button type="button" className={ui.button} onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className={cx(ui.button, ui.primary)} disabled={!asset || tooShort}>
            Add B-roll
          </button>
        </div>
      </form>
    </DialogFrame>
  );
}

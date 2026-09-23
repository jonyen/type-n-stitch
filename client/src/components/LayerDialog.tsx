import { useEffect, useRef, useState, type FormEvent } from 'react';

import { cx } from '../cx';
import { formatTime } from '../editlist';
import { FRAMES, mediaName, type LayerTrack } from '../overlays';
import ui from '../styles/ui.module.css';
import type { Asset, Frame, LayerEdit, SourceView } from '../types';
import { AssetPicker } from './AssetPicker';
import { DialogFrame } from './DialogFrame';
import frameStyles from './DialogFrame.module.css';
import styles from './LayerDialog.module.css';

/** What the dialog hands back. `media` and `offset` matter only when adding. */
export interface LayerChoice {
  media: string;
  offset: number;
  track: LayerTrack;
  frame: Frame;
  audio: number | null;
}

interface Props {
  assets: Asset[];
  /** The project's own videos; the picker offers them next to the uploads. */
  sources: SourceView[];
  /** The layer being changed (SetLayer). Absent when adding over the selected words. */
  initial?: LayerEdit | undefined;
  /** The words the layer covers, and their length in seconds. */
  original: string;
  rangeLength: number;
  onUpload: (f: File) => Promise<Asset>;
  onSubmit: (choice: LayerChoice) => void;
  onRemove?: (() => void) | undefined;
  onCancel: () => void;
}

/** The level a layer's sound starts at when it is switched on, in dB. */
export const DEFAULT_LEVEL = -6;

const TRACKS: LayerTrack[] = [2, 3];

const FRAME_NAMES: Record<Frame, string> = {
  full: 'Full frame',
  pipTopLeft: 'Top left',
  pipTopRight: 'Top right',
  pipBottomLeft: 'Bottom left',
  pipBottomRight: 'Bottom right',
};

export function LayerDialog({
  assets,
  sources,
  initial,
  original,
  rangeLength,
  onUpload,
  onSubmit,
  onRemove,
  onCancel,
}: Props) {
  const [asset, setAsset] = useState<Asset | null>(null);
  const [offset, setOffset] = useState(0);
  const [track, setTrack] = useState<LayerTrack>(initial?.track ?? 2);
  const [frame, setFrame] = useState<Frame>(initial?.frame ?? 'full');
  const [sound, setSound] = useState(initial !== undefined && initial.audio !== null);
  const [level, setLevel] = useState(initial?.audio ?? DEFAULT_LEVEL);
  const preview = useRef<HTMLVideoElement>(null);
  const max = asset ? Math.max(0, asset.duration - rangeLength) : 0;
  const tooShort = !initial && asset !== null && asset.duration + 1e-6 < rangeLength;
  const ready = initial !== undefined || (asset !== null && !tooShort);
  useEffect(() => {
    setOffset(0);
  }, [asset]);
  useEffect(() => {
    if (preview.current) preview.current.currentTime = offset;
  }, [offset, asset]);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!ready) return;
    const audio = sound ? level : null;
    if (initial) onSubmit({ media: initial.media, offset: initial.offset, track, frame, audio });
    else if (asset) onSubmit({ media: asset.id, offset, track, frame, audio });
  };
  const name = initial ? (mediaName(initial.media, assets, sources) ?? 'missing file') : '';
  return (
    <DialogFrame
      title={initial ? 'Layer' : 'Add layer'}
      description={
        initial ? (
          <>
            Showing {name} over “{original}” ({formatTime(rangeLength)}).
          </>
        ) : (
          <>
            Show a clip over “{original}” ({formatTime(rangeLength)}). The voice keeps going.
          </>
        )
      }
      wide
      onClose={onCancel}
    >
      <form className={frameStyles.form} onSubmit={submit}>
        {!initial && (
          <>
            <AssetPicker
              assets={assets}
              sources={sources}
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
                  className={styles.preview}
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
                {tooShort && (
                  <p className={ui.error}>This shot is shorter than the selected words.</p>
                )}
              </>
            )}
          </>
        )}

        <div className={styles.row}>
          <span className={styles.label} aria-hidden>
            Track
          </span>
          <div className={styles.segmented} role="radiogroup" aria-label="Track">
            {TRACKS.map((t) => (
              <button
                key={t}
                type="button"
                role="radio"
                aria-checked={track === t}
                className={cx(styles.option, track === t && styles.on)}
                onClick={() => setTrack(t)}
              >
                V{t}
              </button>
            ))}
          </div>
        </div>

        <div className={styles.row}>
          <span className={styles.label} aria-hidden>
            Frame
          </span>
          <div className={styles.segmented} role="radiogroup" aria-label="Frame">
            {FRAMES.map((f) => (
              <button
                key={f}
                type="button"
                role="radio"
                aria-checked={frame === f}
                className={cx(styles.option, frame === f && styles.on)}
                onClick={() => setFrame(f)}
              >
                <span className={styles.thumb} aria-hidden>
                  <span className={cx(styles.pane, styles[f])} />
                </span>
                {FRAME_NAMES[f]}
              </button>
            ))}
          </div>
        </div>

        <div className={styles.row}>
          <span className={styles.label} aria-hidden>
            Sound
          </span>
          <div className={styles.sound}>
            <label className={ui.toggle}>
              <input type="checkbox" checked={sound} onChange={(e) => setSound(e.target.checked)} />{' '}
              Play its sound
            </label>
            <label className={cx(ui.field, styles.level)}>
              Level {level} dB
              <input
                type="range"
                min={-30}
                max={12}
                step={1}
                value={level}
                disabled={!sound}
                onChange={(e) => setLevel(Number(e.target.value))}
              />
            </label>
          </div>
        </div>
        {sound && level > 0 && (
          <p className={ui.muted}>
            Boost above 0 dB is applied on export; the preview cannot play louder than the source.
          </p>
        )}

        <div className={frameStyles.actions}>
          {initial && onRemove && (
            <button type="button" className={cx(ui.button, ui.danger)} onClick={onRemove}>
              Remove
            </button>
          )}
          <span className={frameStyles.spacer} />
          <button type="button" className={ui.button} onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className={cx(ui.button, ui.primary)} disabled={!ready}>
            {initial ? 'Save' : 'Add layer'}
          </button>
        </div>
      </form>
    </DialogFrame>
  );
}

import { useMemo, useRef, useState, type PointerEvent } from 'react';

import type { Thumbnails } from '../api';
import { dropSlot, firstWords, moveFor } from '../clipstrip';
import { cx } from '../cx';
import { formatTime } from '../editlist';
import { audios, brolls } from '../overlays';
import type { Peer } from '../realtime';
import {
  clipSpans,
  outputToSource,
  overlaySpans,
  pxToOutput,
  rulerTicks,
  sourceToOutput,
  timelineLength,
  type Segment,
} from '../timeline';
import type { Asset, AudioEdit, BrollEdit, Edit, Range, Word } from '../types';
import { ScrubPreview } from './ScrubPreview';
import styles from './Timeline.module.css';

export interface OverlayRef {
  kind: 'broll' | 'audio';
  start: number;
}

export interface TimelineProps {
  words: Word[];
  edits: Edit[];
  assets: Asset[];
  ordered: Range[];
  segments: Segment[];
  /** The playhead, in source seconds. */
  currentTime: number;
  peers: Peer[];
  thumbs: Thumbnails | null;
  readOnly: boolean;
  selectedClip: number | null;
  selectedOverlay: OverlayRef | null;
  onSeek: (sourceTime: number) => void;
  onSelectClip: (pieceStart: number) => void;
  onMoveClip: (piece: number, before: number | null) => void;
  onSelectOverlay: (ref: OverlayRef | null) => void;
  onOpenAudio: (start: number) => void;
}

interface Drag {
  from: number;
  slot: number;
  x0: number;
  moved: boolean;
}

/** Past this many pixels a press on a clip is a drag, not a click. */
const DRAG_SLOP = 4;

export function Timeline(props: TimelineProps) {
  const { words, edits, assets, ordered, segments, currentTime, peers, thumbs, readOnly } = props;
  const length = timelineLength(segments);
  const lanes = useRef<HTMLDivElement>(null);
  const spans = useMemo(() => clipSpans(ordered, segments), [ordered, segments]);
  const scrubbing = useRef(false);
  const [hover, setHover] = useState<{ t: number; x: number; width: number } | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);

  const pct = (t: number) =>
    `${length > 0 ? (Math.min(Math.max(t, 0), length) / length) * 100 : 0}%`;
  const nameOf = (id: string) => assets.find((a) => a.id === id)?.name ?? 'missing file';

  const pointerAt = (clientX: number) => {
    const r = lanes.current?.getBoundingClientRect();
    if (!r) return null;
    return { t: pxToOutput(clientX, r.left, r.width, length), x: clientX - r.left, width: r.width };
  };

  // Background: click to seek, drag to scrub.
  const onLanesDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const p = pointerAt(e.clientX);
    if (!p) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    scrubbing.current = true;
    props.onSelectOverlay(null);
    props.onSeek(outputToSource(p.t, segments));
  };
  const onLanesMove = (e: PointerEvent<HTMLDivElement>) => {
    const p = pointerAt(e.clientX);
    if (!p) return;
    setHover(p);
    if (scrubbing.current) props.onSeek(outputToSource(p.t, segments));
  };
  const onLanesUp = () => {
    scrubbing.current = false;
  };

  // Clips: press and release to select, drag to reorder.
  const clipCentres = () =>
    Array.from(lanes.current?.querySelectorAll<HTMLElement>('[data-clip]') ?? []).map((el) => {
      const r = el.getBoundingClientRect();
      return r.left + r.width / 2;
    });
  const onClipDown = (k: number, e: PointerEvent<HTMLButtonElement>) => {
    e.stopPropagation();
    if (e.button !== 0) return;
    const piece = ordered[k];
    if (readOnly) {
      if (piece) props.onSelectClip(piece.start);
      return;
    }
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ from: k, slot: k, x0: e.clientX, moved: false });
  };
  const onClipMove = (e: PointerEvent<HTMLButtonElement>) => {
    if (!drag) return;
    const moved = drag.moved || Math.abs(e.clientX - drag.x0) > DRAG_SLOP;
    setDrag({ ...drag, moved, slot: moved ? dropSlot(e.clientX, clipCentres()) : drag.slot });
  };
  const onClipUp = (k: number, e: PointerEvent<HTMLButtonElement>) => {
    if (!drag) return;
    // Recompute from this event: a move and its release can land in one frame,
    // before the state update from the move has rendered.
    const moved = drag.moved || Math.abs(e.clientX - drag.x0) > DRAG_SLOP;
    const slot = moved ? dropSlot(e.clientX, clipCentres()) : drag.slot;
    setDrag(null);
    if (moved) {
      const op = moveFor(ordered, drag.from, slot);
      if (op) props.onMoveClip(op.piece, op.before);
      return;
    }
    const piece = ordered[k];
    if (piece) props.onSelectClip(piece.start);
  };
  const dropAt =
    drag?.moved && moveFor(ordered, drag.from, drag.slot)
      ? (spans[drag.slot]?.start ?? length)
      : null;

  const bars = (kind: OverlayRef['kind'], list: (BrollEdit | AudioEdit)[]) =>
    list.flatMap((e) => {
      const selected =
        props.selectedOverlay?.kind === kind && props.selectedOverlay.start === e.start;
      const label = `${kind === 'broll' ? 'B-roll' : 'Music'} ${nameOf(e.media)}`;
      return overlaySpans(e, segments, kind === 'audio').map((w, i) => (
        <button
          key={`${kind}-${e.start}-${i}`}
          type="button"
          className={cx(styles.bar, styles[kind], selected && styles.selected)}
          style={{ left: pct(w.start), width: pct(w.end - w.start) }}
          data-overlay={`${kind}:${e.start}`}
          aria-label={label}
          aria-pressed={selected}
          title={kind === 'audio' ? `${label} · double-click to edit` : label}
          onPointerDown={(ev) => ev.stopPropagation()}
          onClick={() => props.onSelectOverlay({ kind, start: e.start })}
          onDoubleClick={kind === 'audio' ? () => props.onOpenAudio(e.start) : undefined}
        >
          <span>{nameOf(e.media)}</span>
        </button>
      ));
    });

  return (
    <div className={styles.timeline}>
      <div className={styles.labels} aria-hidden>
        <span className={styles.rulerLabel} />
        <span>Clips</span>
        <span className={styles.overlayLabel}>B-roll</span>
        <span className={styles.overlayLabel}>Music</span>
      </div>
      <div
        ref={lanes}
        className={styles.lanes}
        data-testid="timeline-lanes"
        onPointerDown={onLanesDown}
        onPointerMove={onLanesMove}
        onPointerUp={onLanesUp}
        onPointerCancel={onLanesUp}
        onPointerLeave={() => {
          if (!scrubbing.current) setHover(null);
        }}
      >
        <div className={styles.ruler}>
          {rulerTicks(length).map((t) => (
            <span key={t} className={styles.tick} style={{ left: pct(t) }}>
              {formatTime(t)}
            </span>
          ))}
        </div>

        <div className={styles.lane} data-lane="clips">
          {ordered.map((piece, k) => {
            const span = spans[k] ?? { start: 0, end: 0 };
            const text = firstWords(words, piece);
            return (
              <button
                key={piece.start}
                type="button"
                data-clip={k}
                className={cx(
                  styles.clip,
                  props.selectedClip !== null &&
                    Math.abs(props.selectedClip - piece.start) < 1e-6 &&
                    styles.selected,
                  drag?.from === k && drag.moved && styles.dragging,
                )}
                style={{ left: pct(span.start), width: pct(span.end - span.start) }}
                aria-label={`Clip ${k + 1}: ${text}`}
                title={readOnly ? text : `${text} · drag to reorder`}
                onPointerDown={(e) => onClipDown(k, e)}
                onPointerMove={onClipMove}
                onPointerUp={(e) => onClipUp(k, e)}
                onPointerCancel={() => setDrag(null)}
              >
                <span>{text || '…'}</span>
              </button>
            );
          })}
          {dropAt !== null && <span className={styles.drop} style={{ left: pct(dropAt) }} />}
        </div>

        <div className={cx(styles.lane, styles.overlayLane)} data-lane="broll">
          {bars('broll', brolls(edits))}
        </div>
        <div className={cx(styles.lane, styles.overlayLane)} data-lane="music">
          {bars('audio', audios(edits))}
        </div>

        {peers.map((p) => (
          <span
            key={p.connId}
            className={styles.peer}
            style={{
              left: pct(sourceToOutput(p.state.playhead, segments)),
              background: p.user.color,
            }}
            title={p.user.displayName}
          />
        ))}
        <span
          className={styles.playhead}
          style={{ left: pct(sourceToOutput(currentTime, segments)) }}
        />
        {hover && length > 0 && (
          <ScrubPreview
            label={formatTime(hover.t)}
            frameTime={outputToSource(hover.t, segments)}
            x={hover.x}
            trackWidth={hover.width}
            thumbs={thumbs}
          />
        )}
      </div>
    </div>
  );
}

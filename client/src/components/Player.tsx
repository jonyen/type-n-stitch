import { useEffect, useRef, useState, type PointerEvent, type RefObject } from 'react';

import { fetchThumbnails, type Thumbnails } from '../api';
import {
  cutRanges,
  formatTime,
  outputDuration,
  overdubAt,
  overdubs,
  skipTarget,
} from '../editlist';
import type { Edit, Media, MediaKind } from '../types';
import type { Playback } from '../usePlayback';

interface Props {
  media: Media;
  /** Thumbnails are served per project, not per media. */
  projectId: string;
  mediaRef: RefObject<HTMLVideoElement | null>;
  edits: Edit[];
  playback: Playback;
}

export function Player({ media, projectId, mediaRef, edits, playback }: Props) {
  const { duration } = media;
  const cutCount = cutRanges(edits).length;
  const overdubCount = overdubs(edits).length;
  const removed = duration - outputDuration(duration, edits);
  const pct = (t: number) => `${(Math.min(Math.max(t, 0), duration) / duration) * 100}%`;

  const thumbs = useThumbnails(projectId, media.kind);
  const [hover, setHover] = useState<{ ratio: number; width: number } | null>(null);
  const dragging = useRef(false);

  const ratioAt = (e: PointerEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    return {
      ratio: Math.min(Math.max((e.clientX - rect.left) / rect.width, 0), 1),
      width: rect.width,
    };
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    dragging.current = true;
    const at = ratioAt(e);
    setHover(at);
    playback.seek(at.ratio * duration);
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const at = ratioAt(e);
    setHover(at);
    if (dragging.current) playback.seek(at.ratio * duration);
  };

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    dragging.current = false;
    if (e.pointerType !== 'mouse') setHover(null);
  };

  return (
    <div className="player">
      <div className={`frame ${media.kind}`}>
        <video
          ref={mediaRef}
          src={media.url}
          preload="auto"
          playsInline
          onClick={playback.toggle}
          muted={playback.overdubbing !== null}
        />
        {media.kind === 'audio' && <div className="audio-badge">audio</div>}
        {playback.overdubbing && (
          <div className="overdub-badge">Overdub: “{playback.overdubbing.text}”</div>
        )}
      </div>

      <div className="transport">
        <button
          type="button"
          className="play"
          onClick={playback.toggle}
          aria-label={playback.playing ? 'Pause' : 'Play'}
          title="Space"
        >
          {playback.playing ? <PauseIcon /> : <PlayIcon />}
        </button>
        <div className="scrub-wrap">
          <div
            className="scrubber"
            role="slider"
            aria-label="Position"
            aria-valuemin={0}
            aria-valuemax={duration}
            aria-valuenow={playback.currentTime}
            tabIndex={-1}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={onPointerUp}
            onPointerLeave={() => {
              if (!dragging.current) setHover(null);
            }}
          >
            {cutRanges(edits).map((r) => (
              <span
                key={`c${r.start}`}
                className="mark cut"
                style={{ left: pct(r.start), width: pct(r.end - r.start) }}
              />
            ))}
            {overdubs(edits).map((r) => (
              <span
                key={`o${r.start}`}
                className="mark overdub"
                style={{ left: pct(r.start), width: pct(r.end - r.start) }}
              />
            ))}
            <span className="playhead" style={{ left: pct(playback.currentTime) }} />
          </div>
          {hover && duration > 0 && (
            <ScrubPreview
              time={hover.ratio * duration}
              x={hover.ratio * hover.width}
              trackWidth={hover.width}
              thumbs={thumbs}
              edits={edits}
            />
          )}
        </div>
        <div className="time">
          <span>{formatTime(playback.currentTime)}</span>
          <span className="muted"> / {formatTime(duration)}</span>
        </div>
      </div>
      <div className="player-meta muted">
        {media.filename} · source {formatTime(duration)} · output{' '}
        <strong>{formatTime(outputDuration(duration, edits))}</strong>
        {removed > 0.05 && <> · {removed.toFixed(1)} s removed</>}
        {cutCount > 0 && (
          <>
            {' '}
            · {cutCount} {cutCount === 1 ? 'cut' : 'cuts'}
          </>
        )}
        {overdubCount > 0 && (
          <>
            {' '}
            · {overdubCount} {overdubCount === 1 ? 'overdub' : 'overdubs'}
          </>
        )}
      </div>
    </div>
  );
}

/** Sprite sheet for the scrubber preview; null for audio or until it loads. */
function useThumbnails(projectId: string, kind: MediaKind): Thumbnails | null {
  const [thumbs, setThumbs] = useState<Thumbnails | null>(null);
  useEffect(() => {
    setThumbs(null);
    if (kind !== 'video') return;
    let cancelled = false;
    fetchThumbnails(projectId)
      .then((t) => {
        if (!cancelled) setThumbs(t);
      })
      .catch(() => {
        // No preview frames; the time label still shows.
      });
    return () => {
      cancelled = true;
    };
  }, [projectId, kind]);
  return thumbs;
}

interface PreviewProps {
  time: number;
  /** Pointer position and track width in px, for placing the card. */
  x: number;
  trackWidth: number;
  thumbs: Thumbnails | null;
  edits: Edit[];
}

/** Frame and timestamp above the scrubber at the hovered position. */
function ScrubPreview({ time, x, trackWidth, thumbs, edits }: PreviewProps) {
  const cardWidth = thumbs ? thumbs.width : 64;
  const left = Math.min(Math.max(x - cardWidth / 2, 0), Math.max(trackWidth - cardWidth, 0));
  const cut = skipTarget(time, cutRanges(edits)) !== null;
  const overdub = overdubAt(time, edits);

  let frame = null;
  if (thumbs) {
    const index = Math.min(Math.floor(time / thumbs.interval), thumbs.count - 1);
    const col = index % thumbs.columns;
    const row = Math.floor(index / thumbs.columns);
    frame = (
      <span
        className="scrub-frame"
        style={{
          width: thumbs.width,
          height: thumbs.height,
          backgroundImage: `url(${thumbs.url})`,
          backgroundSize: `${thumbs.columns * thumbs.width}px ${thumbs.rows * thumbs.height}px`,
          backgroundPosition: `-${col * thumbs.width}px -${row * thumbs.height}px`,
        }}
      />
    );
  }

  return (
    <div
      className={`scrub-preview${cut ? ' cut' : ''}`}
      style={{ left, width: cardWidth }}
      aria-hidden
    >
      {frame}
      <span className="scrub-time">
        {formatTime(time)}
        {cut && ' · cut'}
        {!cut && overdub && ' · overdub'}
      </span>
    </div>
  );
}

function PlayIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden>
      <path d="M7 5v14l12-7z" fill="currentColor" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden>
      <path d="M6 5h4v14H6zM14 5h4v14h-4z" fill="currentColor" />
    </svg>
  );
}

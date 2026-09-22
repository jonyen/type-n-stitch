import { useMemo, useRef, useState, type PointerEvent, type RefObject } from 'react';

import type { Thumbnails } from '../api';
import {
  captionsAt,
  cutRanges,
  formatTime,
  joins,
  nearDipJoin,
  outputDuration,
  overdubAt,
  overdubs,
  pieces,
  skipTarget,
  titles,
} from '../editlist';
import { audios, brolls } from '../overlays';
import type { Peer } from '../realtime';
import type { Asset, Edit, Media, Transition, Word } from '../types';
import type { Playback } from '../usePlayback';
import { Overlays } from './Overlays';
import { Caption, TitleCard } from './TitleCard';

interface Props {
  media: Media;
  /** The scrubber sprite sheet, fetched per project. */
  thumbs: Thumbnails | null;
  mediaRef: RefObject<HTMLVideoElement | null>;
  edits: Edit[];
  assets: Asset[];
  words: Word[];
  playback: Playback;
  peers: Peer[];
  /** The project default transition, for the dip preview. */
  transition: Transition;
}

export function Player({
  media,
  thumbs,
  mediaRef,
  edits,
  assets,
  words,
  playback,
  peers,
  transition,
}: Props) {
  const { duration } = media;
  const cutCount = cutRanges(edits).length;
  const overdubCount = overdubs(edits).length;
  const removed = duration - outputDuration(duration, edits);
  const pct = (t: number) => `${(Math.min(Math.max(t, 0), duration) / duration) * 100}%`;

  // The output timeline, so the preview can dim the frame around dipping joins.
  const timeline = useMemo(() => {
    const list = pieces(duration, edits);
    return { list, joinList: joins(list, edits, transition) };
  }, [duration, edits, transition]);
  const fading = nearDipJoin(playback.currentTime, timeline.list, timeline.joinList);

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
      <div className={`frame ${media.kind}${fading ? ' fading' : ''}`}>
        <video
          ref={mediaRef}
          src={media.url}
          preload="auto"
          playsInline
          onClick={playback.toggle}
          muted={playback.overdubbing !== null}
        />
        <Overlays edits={edits} assets={assets} words={words} playback={playback} />
        {media.kind === 'audio' && <div className="audio-badge">audio</div>}
        {playback.overdubbing && (
          <div className="overdub-badge">Overdub: “{playback.overdubbing.text}”</div>
        )}
        {captionsAt(playback.currentTime, edits).map((c, i) => (
          <Caption key={`${i}-${c.start}-${c.position}`} caption={c} />
        ))}
        {playback.titling && <TitleCard title={playback.titling} />}
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
            {brolls(edits).map((b) => (
              <span
                key={`b${b.start}`}
                className="mark broll"
                style={{ left: pct(b.start), width: pct(b.end - b.start) }}
              />
            ))}
            {audios(edits).map((a) => (
              <span
                key={`a${a.start}`}
                className="mark audio"
                style={{ left: pct(a.start), width: pct(a.end - a.start) }}
              />
            ))}
            {titles(edits).map((t, i) => (
              <span key={`t${i}-${t.at}`} className="mark title" style={{ left: pct(t.at) }} />
            ))}
            <span className="playhead" style={{ left: pct(playback.currentTime) }} />
            {peers.map((p) => (
              <span
                key={p.connId}
                className={`peer-head${p.state.playing ? ' playing' : ''}`}
                style={{ left: pct(p.state.playhead), background: p.user.color }}
                title={p.user.displayName}
              />
            ))}
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

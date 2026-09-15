import type { MouseEvent, RefObject } from 'react';

import { cutRanges, formatTime, outputDuration, overdubs } from '../editlist';
import type { Edit, Media } from '../types';
import type { Playback } from '../usePlayback';

interface Props {
  media: Media;
  mediaRef: RefObject<HTMLVideoElement | null>;
  edits: Edit[];
  playback: Playback;
}

export function Player({ media, mediaRef, edits, playback }: Props) {
  const { duration } = media;
  const pct = (t: number) => `${(Math.min(Math.max(t, 0), duration) / duration) * 100}%`;

  const onScrub = (e: MouseEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const ratio = (e.clientX - rect.left) / rect.width;
    playback.seek(ratio * duration);
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
        <div
          className="scrubber"
          role="slider"
          aria-label="Position"
          aria-valuemin={0}
          aria-valuemax={duration}
          aria-valuenow={playback.currentTime}
          tabIndex={-1}
          onClick={onScrub}
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
        <div className="time">
          <span>{formatTime(playback.currentTime)}</span>
          <span className="muted"> / {formatTime(duration)}</span>
        </div>
      </div>
      <div className="player-meta muted">
        {media.filename} · output {formatTime(outputDuration(duration, edits))}
      </div>
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

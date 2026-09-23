import { useMemo, type RefObject } from 'react';

import { cx } from '../cx';
import { captionsAt, cutRanges, formatTime, joins, nearDipJoin, overdubs } from '../editlist';
import { timelineLength, type Segment } from '../timeline';
import type { Asset, Edit, Media, Transition, Word } from '../types';
import type { Playback } from '../usePlayback';
import { Overlays } from './Overlays';
import styles from './Player.module.css';
import { Caption, TitleCard } from './TitleCard';

interface Props {
  media: Media;
  mediaRef: RefObject<HTMLVideoElement | null>;
  edits: Edit[];
  assets: Asset[];
  words: Word[];
  playback: Playback;
  /** The edit in output time; the same layout the timeline draws. */
  segments: Segment[];
  /** The project default transition, for the dip preview. */
  transition: Transition;
}

export function Player({
  media,
  mediaRef,
  edits,
  assets,
  words,
  playback,
  segments,
  transition,
}: Props) {
  // Dims the frame near a dipping join. Uses the reordered layout, so the
  // preview dims at the same joins the export fades.
  const joinList = useMemo(() => joins(segments, edits, transition), [segments, edits, transition]);
  const fading = nearDipJoin(playback.currentTime, segments, joinList);
  const outputLength = timelineLength(segments);
  const cutCount = cutRanges(edits).length;
  const overdubCount = overdubs(edits).length;

  return (
    <div className={styles.player}>
      <div
        className={cx(
          styles.frame,
          media.kind === 'audio' && styles.audio,
          fading && styles.fading,
        )}
      >
        <video
          ref={mediaRef}
          src={media.url}
          preload="auto"
          playsInline
          onClick={playback.toggle}
          muted={playback.overdubbing !== null}
        />
        <Overlays edits={edits} assets={assets} words={words} playback={playback} />
        {media.kind === 'audio' && <div className={styles.audioBadge}>audio</div>}
        {playback.overdubbing && (
          <div className={styles.overdubBadge}>Overdub: “{playback.overdubbing.text}”</div>
        )}
        {captionsAt(playback.currentTime, edits).map((c, i) => (
          <Caption key={`${i}-${c.start}-${c.position}`} caption={c} />
        ))}
        {playback.titling && <TitleCard title={playback.titling} />}
      </div>

      <div className={styles.transport}>
        <button
          type="button"
          className={styles.play}
          onClick={playback.toggle}
          aria-label={playback.playing ? 'Pause' : 'Play'}
          title="Space"
        >
          {playback.playing ? <PauseIcon /> : <PlayIcon />}
        </button>
        <span className={styles.time}>
          {formatTime(playback.outputTime)}
          <span className={styles.total}> / {formatTime(outputLength)}</span>
        </span>
        <span className={styles.meta}>
          source {formatTime(media.duration)}
          {cutCount > 0 && ` · ${cutCount} ${cutCount === 1 ? 'cut' : 'cuts'}`}
          {overdubCount > 0 && ` · ${overdubCount} ${overdubCount === 1 ? 'overdub' : 'overdubs'}`}
        </span>
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

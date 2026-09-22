import type { Thumbnails } from '../api';
import styles from './ScrubPreview.module.css';

interface Props {
  /** The time to print (output time on the timeline). */
  label: string;
  /** The source time whose frame to show. */
  frameTime: number;
  /** Pointer position and track width in px, for placing the card. */
  x: number;
  trackWidth: number;
  thumbs: Thumbnails | null;
}

/** A frame from the thumbnail sprite and a timestamp, above the hovered spot. */
export function ScrubPreview({ label, frameTime, x, trackWidth, thumbs }: Props) {
  const cardWidth = thumbs ? thumbs.width : 64;
  const left = Math.min(Math.max(x - cardWidth / 2, 0), Math.max(trackWidth - cardWidth, 0));
  let frame = null;
  if (thumbs) {
    const index = Math.min(Math.max(Math.floor(frameTime / thumbs.interval), 0), thumbs.count - 1);
    const col = index % thumbs.columns;
    const row = Math.floor(index / thumbs.columns);
    frame = (
      <span
        className={styles.frame}
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
    <div className={styles.card} style={{ left, width: cardWidth }} aria-hidden>
      {frame}
      <span className={styles.time}>{label}</span>
    </div>
  );
}

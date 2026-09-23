// The preview's stand-ins for what the engine rasterises at export time: a
// full-frame title card and a caption box over the picture.

import { cx } from '../cx';
import type { CaptionEdit, CaptionPos, TitleEdit, TitleStyle } from '../types';
import styles from './TitleCard.module.css';

const CARD: Record<TitleStyle, string | undefined> = {
  dark: styles.dark,
  light: styles.light,
  accent: styles.accent,
};

const PLACE: Record<CaptionPos, string | undefined> = {
  bottomLeft: styles.bottomLeft,
  bottomCenter: styles.bottomCenter,
  topLeft: styles.topLeft,
};

export function TitleCard({ title }: { title: TitleEdit }) {
  return (
    <div className={cx(styles.card, CARD[title.style])} aria-live="polite">
      <div className={styles.text}>{title.text}</div>
      {title.subtitle && <div className={styles.sub}>{title.subtitle}</div>}
    </div>
  );
}

export function Caption({ caption }: { caption: CaptionEdit }) {
  return <div className={cx(styles.caption, PLACE[caption.position])}>{caption.text}</div>;
}

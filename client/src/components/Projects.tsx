import { clipLength, relativeTime } from '../format';
import type { ProjectSummary } from '../types';
import { cx } from '../cx';
import { sheetFor, spriteStyle, THUMBS_FILE } from '../thumbs';
import styles from './Library.module.css';
import ui from '../styles/ui.module.css';

interface Props {
  items: ProjectSummary[];
  onOpen: (project: ProjectSummary) => void;
  disabled: boolean;
}

/** The signed-in user's projects, newest first; hidden until there is one. */
export function Projects({ items, onOpen, disabled }: Props) {
  if (items.length === 0) return null;
  return (
    <section className={styles.library} aria-labelledby="projects-heading">
      <div className={styles.head}>
        <h2 id="projects-heading">Your projects</h2>
        <span className={ui.muted}>
          {items.length} {items.length === 1 ? 'project' : 'projects'}
        </span>
      </div>
      <ul className={styles.grid}>
        {items.map((p) => (
          <li key={p.id}>
            <button
              type="button"
              className={styles.card}
              disabled={disabled}
              onClick={() => onOpen(p)}
            >
              <span className={styles.poster}>
                <span className={styles.glyph} aria-hidden>
                  {p.media.kind === 'audio' ? '♪' : '▶'}
                </span>
                {p.media.kind === 'video' && (
                  <span
                    className={styles.frame}
                    aria-hidden
                    style={(() => {
                      const sheet = sheetFor(p.media.duration);
                      // A tenth of the way in: the first frame is often black.
                      return spriteStyle(
                        sheet,
                        `/data/${p.media.id}/${THUMBS_FILE}`,
                        Math.floor(sheet.count / 10),
                      );
                    })()}
                  />
                )}
                <span className={styles.length}>{clipLength(p.media.duration)}</span>
              </span>
              <strong>{p.title}</strong>
              <span className={styles.meta}>
                <span className={cx(styles.role, p.role === 'owner' && styles.owner)}>
                  {p.role}
                </span>
                <span>opened {relativeTime(p.createdAt)}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

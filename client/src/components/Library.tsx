import { useEffect, useState } from 'react';

import { listLibrary } from '../api';
import { clipLength } from '../format';
import type { LibraryItem } from '../types';
import { cx } from '../cx';
import styles from './Library.module.css';
import ui from '../styles/ui.module.css';

interface Props {
  onOpen: (item: LibraryItem) => void;
  disabled: boolean;
}

/** Starter clips from samples/library.json; hidden when the manifest is empty. */
export function Library({ onOpen, disabled }: Props) {
  const [items, setItems] = useState<LibraryItem[]>([]);

  useEffect(() => {
    listLibrary()
      .then(setItems)
      .catch(() => setItems([]));
  }, []);

  if (items.length === 0) return null;
  const missing = items.filter((i) => !i.available).length;

  return (
    <section className={styles.library} aria-labelledby="library-heading">
      <div className={styles.head}>
        <h2 id="library-heading">Start from a sample</h2>
        {missing > 0 && (
          <span className={ui.muted}>
            {missing} not downloaded yet · run <code>npm run library</code>
          </span>
        )}
      </div>
      <ul className={styles.grid}>
        {items.map((item) => (
          <li key={item.slug} className={cx(styles.item, !item.available && styles.unavailable)}>
            <button
              type="button"
              className={styles.card}
              disabled={disabled || !item.available}
              onClick={() => onOpen(item)}
            >
              <span className={styles.poster}>
                {item.poster ? (
                  <img src={item.poster} alt="" loading="lazy" />
                ) : (
                  <span className={styles.glyph} aria-hidden>
                    {item.kind === 'audio' ? '♪' : '▶'}
                  </span>
                )}
                {item.duration !== null && (
                  <span className={styles.length}>{clipLength(item.duration)}</span>
                )}
              </span>
              <strong>{item.title}</strong>
              <span className={styles.blurb}>{item.blurb}</span>
            </button>
            <span className={styles.credit}>
              {item.sourceUrl ? (
                <a href={item.sourceUrl} target="_blank" rel="noreferrer" title={item.sourceTitle}>
                  {item.author}
                </a>
              ) : (
                item.author
              )}
              {item.sourceUrl && ' · CC BY'}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

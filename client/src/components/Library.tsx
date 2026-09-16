import { useEffect, useState } from 'react';

import { listLibrary } from '../api';
import type { LibraryItem } from '../types';

interface Props {
  onOpen: (item: LibraryItem) => void;
  disabled: boolean;
}

function formatLength(seconds: number): string {
  const s = Math.round(seconds);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
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
    <section className="library" aria-labelledby="library-heading">
      <div className="library-head">
        <h2 id="library-heading">Or start from a sample</h2>
        {missing > 0 && (
          <span className="muted">
            {missing} not downloaded yet · run <code>npm run library</code>
          </span>
        )}
      </div>
      <ul className="library-grid">
        {items.map((item) => (
          <li key={item.slug} className={item.available ? '' : 'unavailable'}>
            <button
              type="button"
              className="library-card"
              disabled={disabled || !item.available}
              onClick={() => onOpen(item)}
            >
              <span className={`poster ${item.kind}`}>
                {item.poster ? (
                  <img src={item.poster} alt="" loading="lazy" />
                ) : (
                  <span className="poster-glyph" aria-hidden>
                    {item.kind === 'audio' ? '♪' : '▶'}
                  </span>
                )}
                {item.duration !== null && (
                  <span className="length">{formatLength(item.duration)}</span>
                )}
              </span>
              <strong>{item.title}</strong>
              <span className="muted">{item.blurb}</span>
            </button>
            <span className="credit">
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

import { useState, type DragEvent } from 'react';

import { cx } from '../cx';
import type { ImportItem } from '../importQueue';
import ui from '../styles/ui.module.css';
import styles from './ImportList.module.css';

interface Props {
  items: ImportItem[];
  /** Reorder a staged file. Absent once the import runs: the list is then read-only. */
  onMove?: (from: number, to: number) => void;
  onRemove?: (index: number) => void;
}

/** The files of an import, in the order they will land on the timeline. */
export function ImportList({ items, onMove, onRemove }: Props) {
  const [dragFrom, setDragFrom] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);
  const editable = onMove !== undefined;

  const end = () => {
    setDragFrom(null);
    setOver(null);
  };
  const drop = (to: number) => (e: DragEvent) => {
    e.preventDefault();
    if (dragFrom !== null && dragFrom !== to) onMove?.(dragFrom, to);
    end();
  };

  return (
    <ol className={styles.list} aria-label="Files to import">
      {items.map((item, i) => (
        <li
          key={item.id}
          className={cx(
            styles.row,
            editable && styles.movable,
            over === i && dragFrom !== null && dragFrom !== i && styles.over,
            dragFrom === i && styles.dragging,
            item.status === 'error' && styles.failed,
          )}
          draggable={editable}
          onDragStart={
            editable
              ? (e) => {
                  // Firefox starts no drag without data.
                  e.dataTransfer?.setData('text/plain', item.name);
                  setDragFrom(i);
                }
              : undefined
          }
          onDragOver={
            editable
              ? (e) => {
                  e.preventDefault();
                  setOver(i);
                }
              : undefined
          }
          onDrop={editable ? drop(i) : undefined}
          onDragEnd={editable ? end : undefined}
        >
          {editable && (
            <span className={styles.handle} aria-hidden>
              ⋮⋮
            </span>
          )}
          <span className={styles.number}>{i + 1}</span>
          <span className={styles.name} title={item.name}>
            {item.name}
          </span>
          {editable ? (
            <span className={styles.actions}>
              <button
                type="button"
                className={cx(ui.iconButton, ui.ghost)}
                aria-label={`Move ${item.name} up`}
                disabled={i === 0}
                onClick={() => onMove?.(i, i - 1)}
              >
                ↑
              </button>
              <button
                type="button"
                className={cx(ui.iconButton, ui.ghost)}
                aria-label={`Move ${item.name} down`}
                disabled={i === items.length - 1}
                onClick={() => onMove?.(i, i + 1)}
              >
                ↓
              </button>
              <button
                type="button"
                className={cx(ui.iconButton, ui.ghost)}
                aria-label={`Remove ${item.name}`}
                onClick={() => onRemove?.(i)}
              >
                ✕
              </button>
            </span>
          ) : (
            <Status item={item} />
          )}
        </li>
      ))}
    </ol>
  );
}

function Status({ item }: { item: ImportItem }) {
  const percent = Math.round(item.progress * 100);
  switch (item.status) {
    case 'queued':
      return <span className={styles.status}>Waiting</span>;
    case 'uploading':
      return (
        <span className={styles.status}>
          <span
            className={styles.bar}
            role="progressbar"
            aria-label={`Uploading ${item.name}`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
          >
            <span className={styles.fill} style={{ width: `${percent}%` }} />
          </span>
          {percent}%
        </span>
      );
    case 'done':
      return <span className={cx(styles.status, styles.done)}>Added</span>;
    case 'error':
      return (
        <span className={cx(styles.status, ui.error)} role="alert">
          {item.error}
        </span>
      );
  }
}

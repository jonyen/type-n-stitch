import { Popover } from 'radix-ui';
import { useMemo } from 'react';

import { cx } from '../cx';
import ui from '../styles/ui.module.css';
import styles from './SelectionToolbar.module.css';

interface Props {
  /** Index of the first selected word; the toolbar floats above it. */
  anchorIndex: number | null;
  open: boolean;
  onDelete: () => void;
  onOverdub: () => void;
  onCaption: () => void;
  onBroll: () => void;
}

/**
 * The actions that need a word selection, floating above it. Anchored to the
 * word's element by its `data-index`, so it follows scrolling. It never takes
 * focus: typing shortcuts keep working on the transcript.
 */
export function SelectionToolbar({
  anchorIndex,
  open,
  onDelete,
  onOverdub,
  onCaption,
  onBroll,
}: Props) {
  const anchor = useMemo(
    () => ({
      current: {
        getBoundingClientRect: () =>
          document.querySelector(`[data-index="${anchorIndex}"]`)?.getBoundingClientRect() ??
          new DOMRect(),
      },
    }),
    [anchorIndex],
  );
  if (anchorIndex === null) return null;
  const button = cx(ui.button, ui.ghost, styles.action);
  return (
    <Popover.Root open={open}>
      <Popover.Anchor virtualRef={anchor} />
      <Popover.Portal>
        <Popover.Content
          className={styles.bar}
          side="top"
          align="start"
          sideOffset={6}
          role="toolbar"
          aria-label="Selection"
          onOpenAutoFocus={(e) => e.preventDefault()}
          onCloseAutoFocus={(e) => e.preventDefault()}
        >
          <button type="button" className={cx(button, styles.delete)} onClick={onDelete}>
            Delete
          </button>
          <button type="button" className={button} onClick={onOverdub}>
            Overdub
          </button>
          <button type="button" className={button} onClick={onCaption}>
            Caption
          </button>
          <button type="button" className={button} onClick={onBroll}>
            B-roll
          </button>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

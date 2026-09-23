import { Popover } from 'radix-ui';
import { useMemo } from 'react';

import { cx } from '../cx';
import ui from '../styles/ui.module.css';
import styles from './SelectionToolbar.module.css';
import type { OverlayRef } from './Timeline';

interface Props {
  /** Index of the first selected word; the toolbar floats above it, full actions. */
  anchorIndex: number | null;
  /** A selected title card's instant; Delete-only, anchored to the card. */
  titleAt: number | null;
  /** A selected clip divider's start; Delete-only, anchored to the clip. */
  clipStart: number | null;
  /** A selected B-roll or music bar; Delete-only, anchored to the timeline bar. */
  overlay: OverlayRef | null;
  open: boolean;
  onDelete: () => void;
  onOverdub: () => void;
  onCaption: () => void;
  onBroll: () => void;
  /**
   * Escape while the toolbar shows: clear the selection. The popover's layer
   * takes the key before the editor's own Escape handler can see it. Clicks
   * elsewhere do not dismiss it; they make their own selection.
   */
  onDismiss: () => void;
}

/** What the toolbar is anchored to, and whether it shows the full action set. */
function target(
  anchorIndex: number | null,
  titleAt: number | null,
  clipStart: number | null,
  overlay: OverlayRef | null,
): { selector: string; deleteOnly: boolean; side: 'top' | 'bottom' } | null {
  if (anchorIndex !== null)
    return { selector: `[data-index="${anchorIndex}"]`, deleteOnly: false, side: 'top' };
  if (titleAt !== null)
    return { selector: `[data-at="${titleAt}"]`, deleteOnly: true, side: 'top' };
  if (clipStart !== null)
    return { selector: `[data-clip-start="${clipStart}"]`, deleteOnly: true, side: 'top' };
  // Below the bar, so it does not cover the lane above it.
  if (overlay !== null)
    return {
      selector: `[data-overlay="${overlay.kind}:${overlay.start}"]`,
      deleteOnly: true,
      side: 'bottom',
    };
  return null;
}

/**
 * The actions that need a selection, floating above it: the full set over a
 * word selection, Delete-only over a selected title card, clip divider or
 * timeline overlay bar.
 * Anchored to the element by a stable data attribute, with a `contextElement`
 * so it follows the transcript panel's own scrolling, not just the window's.
 * It never takes focus: typing shortcuts keep working underneath.
 */
export function SelectionToolbar({
  anchorIndex,
  titleAt,
  clipStart,
  overlay,
  open,
  onDelete,
  onOverdub,
  onCaption,
  onBroll,
  onDismiss,
}: Props) {
  const t = target(anchorIndex, titleAt, clipStart, overlay);
  const selector = t?.selector ?? null;
  const anchor = useMemo(
    () => ({
      current: {
        getBoundingClientRect: () =>
          (selector ? document.querySelector(selector) : null)?.getBoundingClientRect() ??
          new DOMRect(),
        // A getter, so floating-ui's autoUpdate re-reads it as the element
        // comes and goes, instead of capturing a stale (or missing) node.
        get contextElement() {
          return (selector ? document.querySelector(selector) : null) ?? undefined;
        },
      },
    }),
    [selector],
  );
  if (!t) return null;
  // A 0×0 rect at the viewport origin is worse than not showing the toolbar:
  // wait for the anchored element to exist and have a box (a hidden one,
  // like an overlay lane in a narrow window, has none).
  const box = document.querySelector(t.selector)?.getBoundingClientRect();
  if (!box || (box.width === 0 && box.height === 0)) return null;
  const button = cx(ui.button, ui.ghost, styles.action);
  return (
    <Popover.Root open={open}>
      <Popover.Anchor virtualRef={anchor} />
      <Popover.Portal>
        <Popover.Content
          className={styles.bar}
          side={t.side}
          align="start"
          sideOffset={6}
          role="toolbar"
          aria-label="Selection"
          onOpenAutoFocus={(e) => e.preventDefault()}
          onCloseAutoFocus={(e) => e.preventDefault()}
          onEscapeKeyDown={() => onDismiss()}
        >
          <button type="button" className={cx(button, styles.delete)} onClick={onDelete}>
            Delete
          </button>
          {!t.deleteOnly && (
            <>
              <button type="button" className={button} onClick={onOverdub}>
                Overdub
              </button>
              <button type="button" className={button} onClick={onCaption}>
                Caption
              </button>
              <button type="button" className={button} onClick={onBroll}>
                B-roll
              </button>
            </>
          )}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

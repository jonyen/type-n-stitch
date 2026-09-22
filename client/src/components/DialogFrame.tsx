import { Dialog } from 'radix-ui';
import { useEffect, useState, type ReactNode } from 'react';

import { cx } from '../cx';
import styles from './DialogFrame.module.css';

interface Props {
  title: string;
  description?: ReactNode;
  /** 640 px instead of 520 px, for pickers. */
  wide?: boolean;
  /** While true, Escape and outside clicks do not close the dialog. */
  busy?: boolean;
  onClose: () => void;
  children: ReactNode;
}

/**
 * The one modal frame. Radix moves focus in on open and traps it; this
 * component gives it back to the opener on close (see below).
 */
export function DialogFrame({
  title,
  description,
  wide = false,
  busy = false,
  onClose,
  children,
}: Props) {
  // Dialogs are mounted only while open, so the whole Radix tree unmounts on
  // close and Radix cannot return focus itself. Remember the element that had
  // focus when the dialog appeared, and give focus back to it on unmount.
  const [returnTo] = useState(() =>
    typeof document === 'undefined' ? null : (document.activeElement as HTMLElement | null),
  );
  useEffect(
    () => () => {
      if (returnTo?.isConnected) returnTo.focus();
    },
    [returnTo],
  );
  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className={styles.overlay} />
        <Dialog.Content
          className={cx(styles.content, wide && styles.wide)}
          onCloseAutoFocus={(e) => e.preventDefault()}
          {...(description === undefined ? { 'aria-describedby': undefined } : {})}
        >
          <Dialog.Title className={styles.title}>{title}</Dialog.Title>
          {description !== undefined && (
            <Dialog.Description className={styles.description}>{description}</Dialog.Description>
          )}
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

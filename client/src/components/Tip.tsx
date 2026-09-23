import { Tooltip } from 'radix-ui';
import type { ReactElement } from 'react';

import { passThrough } from '../keyboardGuard';

import styles from './Tip.module.css';

/** A tooltip on one trigger element. The element must accept a ref (a DOM element does). */
export function Tip({ label, children }: { label: string; children: ReactElement }) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content className={styles.tip} sideOffset={6} onEscapeKeyDown={passThrough}>
          {label}
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

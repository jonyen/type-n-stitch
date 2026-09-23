import { DropdownMenu } from 'radix-ui';

import type { ThemeChoice } from '../theme';

const OPTIONS: { value: ThemeChoice; label: string }[] = [
  { value: 'dark', label: 'Dark' },
  { value: 'light', label: 'Light' },
  { value: 'system', label: 'Match system' },
];

interface Props {
  value: ThemeChoice;
  onChange: (choice: ThemeChoice) => void;
  /** Class for each item, from the menu that hosts this group. */
  itemClassName?: string | undefined;
}

/** Theme radio items. Lives inside a Radix DropdownMenu.Content (the account menu). */
export function ThemeToggle({ value, onChange, itemClassName }: Props) {
  return (
    <DropdownMenu.RadioGroup value={value} onValueChange={(v) => onChange(v as ThemeChoice)}>
      {OPTIONS.map((o) => (
        <DropdownMenu.RadioItem key={o.value} value={o.value} className={itemClassName}>
          <DropdownMenu.ItemIndicator>✓ </DropdownMenu.ItemIndicator>
          {o.label}
        </DropdownMenu.RadioItem>
      ))}
    </DropdownMenu.RadioGroup>
  );
}

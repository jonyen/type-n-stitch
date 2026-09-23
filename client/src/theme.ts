// The viewer's theme choice. Dark is the default; "system" follows the OS.
// Storage is passed in so the rules are testable and a blocked or missing
// localStorage never breaks rendering.

export type ThemeChoice = 'dark' | 'light' | 'system';

export const THEME_KEY = 'tns-theme';

const CHOICES: readonly ThemeChoice[] = ['dark', 'light', 'system'];

export function readTheme(storage: Pick<Storage, 'getItem'> | null): ThemeChoice {
  try {
    const stored = storage?.getItem(THEME_KEY);
    return CHOICES.find((c) => c === stored) ?? 'dark';
  } catch {
    return 'dark';
  }
}

export function saveTheme(storage: Pick<Storage, 'setItem'> | null, choice: ThemeChoice): void {
  try {
    storage?.setItem(THEME_KEY, choice);
  } catch {
    // Private windows and blocked site data: the choice lasts for this page only.
  }
}

/** `tokens.css` keys every theme off `data-theme` on the root element. */
export function applyTheme(root: HTMLElement, choice: ThemeChoice): void {
  root.dataset.theme = choice;
}

/** `window.localStorage`, or null where touching it throws. */
export function safeStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

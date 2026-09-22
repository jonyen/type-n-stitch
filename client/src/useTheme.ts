import { useCallback, useEffect, useState } from 'react';

import { applyTheme, readTheme, safeStorage, saveTheme, type ThemeChoice } from './theme';

/** The viewer's theme choice, applied to `<html>` and remembered in this browser. */
export function useTheme(): [ThemeChoice, (choice: ThemeChoice) => void] {
  const [choice, setChoice] = useState<ThemeChoice>(() => readTheme(safeStorage()));
  useEffect(() => {
    applyTheme(document.documentElement, choice);
  }, [choice]);
  const set = useCallback((next: ThemeChoice) => {
    saveTheme(safeStorage(), next);
    setChoice(next);
  }, []);
  return [choice, set];
}

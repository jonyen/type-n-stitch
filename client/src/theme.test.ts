import { describe, expect, it } from 'vitest';

import { THEME_KEY, applyTheme, readTheme, saveTheme } from './theme';

function memory(initial: Record<string, string> = {}) {
  const data = { ...initial };
  return {
    data,
    getItem: (k: string) => data[k] ?? null,
    setItem: (k: string, v: string) => {
      data[k] = v;
    },
  };
}

describe('theme', () => {
  it('defaults to dark with no storage or no stored choice', () => {
    expect(readTheme(null)).toBe('dark');
    expect(readTheme(memory())).toBe('dark');
  });

  it('reads a stored choice and ignores anything unknown', () => {
    expect(readTheme(memory({ [THEME_KEY]: 'light' }))).toBe('light');
    expect(readTheme(memory({ [THEME_KEY]: 'system' }))).toBe('system');
    expect(readTheme(memory({ [THEME_KEY]: 'neon' }))).toBe('dark');
  });

  it('survives storage that throws', () => {
    const broken = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    };
    expect(readTheme(broken)).toBe('dark');
    expect(() => saveTheme(broken, 'light')).not.toThrow();
  });

  it('saves the choice under the theme key', () => {
    const store = memory();
    saveTheme(store, 'system');
    expect(store.data[THEME_KEY]).toBe('system');
  });

  it('applies the choice as a data attribute', () => {
    const root = { dataset: {} as DOMStringMap } as HTMLElement;
    applyTheme(root, 'light');
    expect(root.dataset.theme).toBe('light');
    applyTheme(root, 'system');
    expect(root.dataset.theme).toBe('system');
  });
});

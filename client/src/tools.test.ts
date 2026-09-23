import { describe, expect, it } from 'vitest';

import { TOOLS, isTypingTarget, toolForKey } from './tools';

const key = (
  k: string,
  mods: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean }> = {},
) => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  ...mods,
});

describe('tools', () => {
  it('maps V, C and X to the three tools, either case', () => {
    expect(toolForKey(key('v'))).toBe('select');
    expect(toolForKey(key('C'))).toBe('razor');
    expect(toolForKey(key('x'))).toBe('range');
    expect(toolForKey(key('z'))).toBeNull();
  });

  it('leaves modified keys alone, so ⌘C still copies', () => {
    expect(toolForKey(key('c', { metaKey: true }))).toBeNull();
    expect(toolForKey(key('x', { ctrlKey: true }))).toBeNull();
    expect(toolForKey(key('v', { altKey: true }))).toBeNull();
  });

  it('knows when focus is in a text field', () => {
    expect(isTypingTarget(null)).toBe(false);
    const inField = { closest: (s: string) => (s.includes('input') ? {} : null) };
    const onPage = { closest: () => null };
    expect(isTypingTarget(inField as unknown as EventTarget)).toBe(true);
    expect(isTypingTarget(onPage as unknown as EventTarget)).toBe(false);
  });

  it('describes every tool with its key', () => {
    expect(TOOLS.map((t) => [t.id, t.key])).toEqual([
      ['select', 'V'],
      ['razor', 'C'],
      ['range', 'X'],
    ]);
  });
});

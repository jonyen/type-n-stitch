// The timeline's mouse tools, and the keyboard rules for switching them.

export type Tool = 'select' | 'razor' | 'range';

export interface ToolInfo {
  id: Tool;
  label: string;
  key: string;
  icon: string;
  hint: string;
}

export const TOOLS: readonly ToolInfo[] = [
  {
    id: 'select',
    label: 'Select',
    key: 'V',
    icon: '↖',
    hint: 'Click to seek · drag a clip to reorder · click a bar to select it',
  },
  {
    id: 'razor',
    label: 'Razor',
    key: 'C',
    icon: '✂',
    hint: 'Click a clip or a word to split it there',
  },
  {
    id: 'range',
    label: 'Range',
    key: 'X',
    icon: '⇥⇤',
    hint: 'Drag across the timeline or across words to cut them out',
  },
];

/** The tool a key press selects, or null. Modified keys never switch tools. */
export function toolForKey(e: {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}): Tool | null {
  if (e.metaKey || e.ctrlKey || e.altKey) return null;
  const k = e.key.toUpperCase();
  return TOOLS.find((t) => t.key === k)?.id ?? null;
}

/** Whether a key press is going into a text field, where shortcuts must not fire. */
export function isTypingTarget(target: EventTarget | null): boolean {
  const node = target as { closest?: (selector: string) => unknown } | null;
  return Boolean(node?.closest?.('input, textarea, select, [contenteditable]'));
}

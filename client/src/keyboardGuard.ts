import { isTypingTarget } from './tools';

/**
 * True when the editor's global keyboard shortcuts (Delete, ⌘Z, Space, Escape,
 * arrows) should be ignored for this keydown: a form control, a Radix
 * menu/listbox/popper, or a Radix dialog already owns it (its own
 * Space/Escape/arrow handling would otherwise double-fire on the editor
 * underneath), or something upstream already called `preventDefault()` on it.
 */
export function shouldIgnoreGlobalKey(target: Element | null, defaultPrevented: boolean): boolean {
  if (defaultPrevented) return true;
  if (!target) return false;
  return (
    isTypingTarget(target) ||
    target.closest(
      '[role="menu"], [role="menubar"], [role="listbox"], [data-radix-popper-content-wrapper], [role="dialog"]',
    ) !== null
  );
}

import { isTypingTarget } from './tools';

/** Key events a layer closed itself on without owning them; see `passThrough`. */
const passedThrough = new WeakSet<Event>();

/**
 * Let the editor still act on a key a Radix layer dismissed itself on. A
 * tooltip closes on Escape and, like every dismissable layer, marks it
 * handled; but it is only a hint, so Escape must still clear the selection
 * and return to Select underneath it.
 */
export function passThrough(e: Event): void {
  passedThrough.add(e);
}

/** Whether something upstream already handled this key, for `shouldIgnoreGlobalKey`. */
export function handledUpstream(e: Event): boolean {
  return e.defaultPrevented && !passedThrough.has(e);
}

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

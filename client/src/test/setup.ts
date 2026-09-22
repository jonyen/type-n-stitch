// Shims for component tests. Node-environment tests import this too, so
// every shim checks that a DOM exists first.
import { cleanup } from '@testing-library/react';
import { afterEach, vi } from 'vitest';

if (typeof window !== 'undefined') {
  afterEach(() => cleanup());

  // Radix positions popovers with ResizeObserver, which jsdom lacks.
  window.ResizeObserver ??= vi.fn(() => ({
    observe: vi.fn(),
    unobserve: vi.fn(),
    disconnect: vi.fn(),
  })) as unknown as typeof ResizeObserver;

  // jsdom implements neither pointer capture nor scrollIntoView.
  Element.prototype.hasPointerCapture ??= vi.fn(() => false);
  Element.prototype.setPointerCapture ??= vi.fn();
  Element.prototype.releasePointerCapture ??= vi.fn();
  Element.prototype.scrollIntoView ??= vi.fn();
}

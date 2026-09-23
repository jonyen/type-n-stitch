// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { shouldIgnoreGlobalKey } from '../keyboardGuard';
import type { OverlayRef } from '../selection';
import { SelectionToolbar } from './SelectionToolbar';

function toolbar(
  overlay: OverlayRef | null,
  onDismiss = vi.fn(),
  anchorIndex: number | null = null,
) {
  const onDelete = vi.fn();
  const onLayer = vi.fn();
  render(
    <SelectionToolbar
      anchorIndex={anchorIndex}
      titleAt={null}
      clipStart={null}
      overlay={overlay}
      open
      onDelete={onDelete}
      onOverdub={vi.fn()}
      onCaption={vi.fn()}
      onLayer={onLayer}
      onDismiss={onDismiss}
    />,
  );
  return { onDelete, onLayer };
}

afterEach(() => vi.restoreAllMocks());

/** jsdom lays nothing out; give every element a visible box unless told otherwise. */
function layout(visible: boolean) {
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(
    visible ? new DOMRect(10, 500, 80, 20) : new DOMRect(0, 0, 0, 0),
  );
}

describe('SelectionToolbar over words', () => {
  it('offers Layer where B-roll was', () => {
    layout(true);
    render(<button type="button" data-index="3" />);
    const { onLayer } = toolbar(null, vi.fn(), 3);
    const bar = screen.getByRole('toolbar', { name: 'Selection' });
    expect(Array.from(bar.querySelectorAll('button'), (b) => b.textContent)).toEqual([
      'Delete',
      'Overdub',
      'Caption',
      'Layer',
    ]);
    fireEvent.click(screen.getByRole('button', { name: 'Layer' }));
    expect(onLayer).toHaveBeenCalledOnce();
  });
});

describe('SelectionToolbar over a selected overlay bar', () => {
  it('offers Delete only, below the layer’s bar on its own track', () => {
    layout(true);
    render(
      <>
        <button type="button" data-overlay="v3:2" />
        <button type="button" data-overlay="v2:2" />
      </>,
    );
    const { onDelete } = toolbar({ kind: 'layer', track: 2, start: 2 });
    const bar = screen.getByRole('toolbar', { name: 'Selection' });
    expect(bar.getAttribute('data-side')).toBe('bottom');
    expect(Array.from(bar.querySelectorAll('button'), (b) => b.textContent)).toEqual(['Delete']);
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(onDelete).toHaveBeenCalledOnce();
  });

  it('stays closed while the bar is not in the document', () => {
    layout(true);
    toolbar({ kind: 'audio', start: 0 });
    expect(screen.queryByRole('toolbar')).toBeNull();
  });

  it('stays closed while the bar is hidden (no box, e.g. lanes hidden in a narrow window)', () => {
    layout(false);
    render(<button type="button" data-overlay="audio:0" />);
    toolbar({ kind: 'audio', start: 0 });
    expect(screen.queryByRole('toolbar')).toBeNull();
  });
});

describe('SelectionToolbar and Escape', () => {
  it('Escape dismisses it through onDismiss, the editor’s clear, though its layer takes the key', async () => {
    layout(true);
    const user = userEvent.setup();
    render(<button type="button" data-index="3" />);
    // The editor's own Escape handler, guarded as App's is.
    const appEscape = vi.fn();
    const onKey = (e: KeyboardEvent) => {
      if (!shouldIgnoreGlobalKey(e.target as Element | null, e.defaultPrevented)) appEscape();
    };
    window.addEventListener('keydown', onKey);
    const onDismiss = vi.fn();
    toolbar(null, onDismiss, 3);
    expect(screen.getByRole('toolbar', { name: 'Selection' })).toBeTruthy();
    await user.keyboard('{Escape}');
    window.removeEventListener('keydown', onKey);
    // The popover's layer swallowed the key, so the clear has to come from it.
    expect(appEscape).not.toHaveBeenCalled();
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it('a click elsewhere (another word) does not dismiss it', async () => {
    layout(true);
    const user = userEvent.setup();
    render(
      <>
        <button type="button" data-index="3" />
        <button type="button">elsewhere</button>
      </>,
    );
    const onDismiss = vi.fn();
    toolbar(null, onDismiss, 3);
    await user.click(screen.getByRole('button', { name: 'elsewhere' }));
    expect(onDismiss).not.toHaveBeenCalled();
  });
});

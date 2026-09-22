// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { SelectionToolbar } from './SelectionToolbar';

function toolbar(overlay: { kind: 'broll' | 'audio'; start: number } | null) {
  const onDelete = vi.fn();
  render(
    <SelectionToolbar
      anchorIndex={null}
      titleAt={null}
      clipStart={null}
      overlay={overlay}
      open
      onDelete={onDelete}
      onOverdub={vi.fn()}
      onCaption={vi.fn()}
      onBroll={vi.fn()}
    />,
  );
  return onDelete;
}

afterEach(() => vi.restoreAllMocks());

/** jsdom lays nothing out; give every element a visible box unless told otherwise. */
function layout(visible: boolean) {
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(
    visible ? new DOMRect(10, 500, 80, 20) : new DOMRect(0, 0, 0, 0),
  );
}

describe('SelectionToolbar over a selected overlay bar', () => {
  it('offers Delete only, below the bar', () => {
    layout(true);
    render(<button type="button" data-overlay="broll:2" />);
    const onDelete = toolbar({ kind: 'broll', start: 2 });
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

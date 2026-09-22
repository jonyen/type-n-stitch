// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

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

describe('SelectionToolbar over a selected overlay bar', () => {
  it('offers Delete only, anchored to the bar', () => {
    render(<button type="button" data-overlay="broll:2" />);
    const onDelete = toolbar({ kind: 'broll', start: 2 });
    const bar = screen.getByRole('toolbar', { name: 'Selection' });
    expect(Array.from(bar.querySelectorAll('button'), (b) => b.textContent)).toEqual(['Delete']);
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(onDelete).toHaveBeenCalledOnce();
  });

  it('stays closed while the bar is not in the document', () => {
    toolbar({ kind: 'audio', start: 0 });
    expect(screen.queryByRole('toolbar')).toBeNull();
  });
});

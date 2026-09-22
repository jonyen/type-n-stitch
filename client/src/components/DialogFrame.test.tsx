// @vitest-environment jsdom
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { CaptionDialog } from './CaptionDialog';
import { DialogFrame } from './DialogFrame';

function Harness({ busy = false }: { busy?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Open
      </button>
      {open && (
        <DialogFrame title="Name it" busy={busy} onClose={() => setOpen(false)}>
          <input aria-label="Name" />
        </DialogFrame>
      )}
    </>
  );
}

describe('DialogFrame', () => {
  it('takes focus on open and gives it back on close', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const opener = screen.getByRole('button', { name: 'Open' });
    await user.click(opener);
    expect(screen.getByRole('dialog', { name: 'Name it' })).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Name' }));
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(opener);
  });

  it('stays open on Escape while busy', async () => {
    const user = userEvent.setup();
    render(<Harness busy />);
    await user.click(screen.getByRole('button', { name: 'Open' }));
    await user.keyboard('{Escape}');
    expect(screen.getByRole('dialog', { name: 'Name it' })).toBeTruthy();
  });

  it('frames a real dialog: the caption dialog focuses its text field', () => {
    render(<CaptionDialog original="three selected words" onSubmit={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByRole('dialog', { name: 'Add caption' })).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Caption' }));
  });
});

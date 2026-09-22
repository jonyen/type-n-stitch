// @vitest-environment jsdom
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Tooltip } from 'radix-ui';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';

import type { Tool } from '../tools';
import { ToolToolbar } from './ToolToolbar';

function Harness({
  readOnly = false,
  shortcuts = true,
}: {
  readOnly?: boolean;
  shortcuts?: boolean;
}) {
  const [tool, setTool] = useState<Tool>('select');
  return (
    <Tooltip.Provider>
      <ToolToolbar tool={tool} onChange={setTool} readOnly={readOnly} shortcuts={shortcuts} />
      <input aria-label="Name" />
    </Tooltip.Provider>
  );
}

const pressed = (name: RegExp) =>
  screen.getByRole('radio', { name }).getAttribute('aria-checked') === 'true';

describe('ToolToolbar', () => {
  it('switches tools on click and shows the active tool’s hint', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    expect(pressed(/Select/)).toBe(true);
    await user.click(screen.getByRole('radio', { name: /Razor/ }));
    expect(pressed(/Razor/)).toBe(true);
    expect(screen.getByText('Click a clip or a word to split it there')).toBeTruthy();
  });

  it('switches with V, C and X, and Escape returns to Select', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.keyboard('x');
    expect(pressed(/Range/)).toBe(true);
    await user.keyboard('c');
    expect(pressed(/Razor/)).toBe(true);
    await user.keyboard('{Escape}');
    expect(pressed(/Select/)).toBe(true);
  });

  it('ignores the shortcuts while typing and while disabled', async () => {
    const user = userEvent.setup();
    const { unmount } = render(<Harness />);
    await user.click(screen.getByRole('textbox', { name: 'Name' }));
    await user.keyboard('x');
    expect(pressed(/Select/)).toBe(true);
    unmount();
    render(<Harness shortcuts={false} />);
    await user.keyboard('x');
    expect(pressed(/Select/)).toBe(true);
  });

  it('leaves the keys to an open menu', async () => {
    const user = userEvent.setup();
    render(
      <>
        <Harness />
        <div role="menu" tabIndex={-1} aria-label="File" />
      </>,
    );
    screen.getByRole('menu', { name: 'File' }).focus();
    await user.keyboard('x');
    expect(pressed(/Select/)).toBe(true);
  });

  it('offers only Select to a viewer', async () => {
    const user = userEvent.setup();
    render(<Harness readOnly />);
    expect(screen.getByRole('radio', { name: /Razor/ }).hasAttribute('disabled')).toBe(true);
    await user.keyboard('c');
    expect(pressed(/Select/)).toBe(true);
  });
});

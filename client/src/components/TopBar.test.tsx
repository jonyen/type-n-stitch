// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Tooltip } from 'radix-ui';
import { describe, expect, it, vi } from 'vitest';

import { TopBar, type EditorControls, type ExportState } from './TopBar';

function controls(exportState: ExportState, headSeq: number): EditorControls {
  return {
    readOnly: false,
    canUndo: false,
    canRedo: false,
    onUndo: vi.fn(),
    onRedo: vi.fn(),
    fillerCount: 0,
    pauseCount: 0,
    twoWordFillers: false,
    onRemoveFillers: vi.fn(),
    onTightenPauses: vi.fn(),
    onTwoWordFillers: vi.fn(),
    hasSelection: false,
    onAddTitle: vi.fn(),
    onAddCaption: vi.fn(),
    onAddLayer: vi.fn(),
    onAddMusic: vi.fn(),
    onAddVideos: vi.fn(),
    onOverdub: vi.fn(),
    onSplit: vi.fn(),
    addingVideos: false,
    transition: 'none',
    onTransition: vi.fn(),
    peers: [],
    status: 'open',
    lastError: null,
    exportState,
    headSeq,
    onExport: vi.fn(),
  };
}

function setup(exportState: ExportState, headSeq: number, overrides: Partial<EditorControls> = {}) {
  const editor = { ...controls(exportState, headSeq), ...overrides };
  render(
    <Tooltip.Provider>
      <TopBar
        user={{ id: 'u', email: 'u@example.com', displayName: 'U', color: '#888' }}
        project={null}
        editor={editor}
        theme="system"
        onTheme={vi.fn()}
        onHome={vi.fn()}
        onAgent={vi.fn()}
        onSignOut={vi.fn()}
      />
    </Tooltip.Provider>,
  );
  return editor;
}

const done = (seq: number): ExportState => ({
  status: 'done',
  url: '/data/p/export.mp4',
  duration: 60,
  bytes: 2048,
  seq,
});

describe('Export', () => {
  it('reopens a current render instead of starting another', async () => {
    const user = userEvent.setup();
    const editor = setup(done(3), 3);
    expect(screen.getByText(/Rendered 1:00.0/)).toBeTruthy();
    expect(screen.queryByText(/out of date/i)).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Export' }));
    expect(editor.onExport).not.toHaveBeenCalled();
  });

  it('marks a render out of date once the edit has changed, and Export renders afresh', async () => {
    const user = userEvent.setup();
    const editor = setup(done(3), 5);
    expect(screen.getByText(/out of date/i)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Export again' })).toBeTruthy();
    await user.click(screen.getByRole('button', { name: 'Export' }));
    expect(editor.onExport).toHaveBeenCalledOnce();
  });

  it('treats a failed render of an older edit the same way', async () => {
    const user = userEvent.setup();
    const editor = setup({ status: 'error', message: 'ffmpeg failed', seq: 3 }, 4);
    expect(screen.getByText(/out of date/i)).toBeTruthy();
    await user.click(screen.getByRole('button', { name: 'Export' }));
    expect(editor.onExport).toHaveBeenCalledOnce();
  });

  it('keeps the progress reachable after Escape closes it mid-render', async () => {
    const user = userEvent.setup();
    const editor = setup({ status: 'rendering', progress: 0.4 }, 3);
    const button = screen.getByRole('button', { name: /Rendering/ });
    await user.click(button);
    expect(screen.getByRole('progressbar')).toBeTruthy();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('progressbar')).toBeNull();
    await user.click(button);
    expect(screen.getByRole('progressbar')).toBeTruthy();
    // Reopening never starts a second render.
    expect(editor.onExport).not.toHaveBeenCalled();
  });
});

describe('Insert → Add video…', () => {
  it('opens a multi-file picker and hands over every chosen file', async () => {
    const user = userEvent.setup();
    const editor = setup({ status: 'idle' }, 1);
    const input = screen.getByTestId('add-video-input') as HTMLInputElement;
    expect(input.multiple).toBe(true);
    const click = vi.spyOn(input, 'click').mockImplementation(() => undefined);
    await user.click(screen.getByRole('button', { name: 'Insert ▾' }));
    await user.click(await screen.findByRole('menuitem', { name: /Add video/ }));
    expect(click).toHaveBeenCalledOnce();
    const a = new File(['a'], 'a.mp4', { type: 'video/mp4' });
    const b = new File(['b'], 'b.mov', { type: 'video/quicktime' });
    fireEvent.change(input, { target: { files: [a, b] } });
    expect(editor.onAddVideos).toHaveBeenCalledWith([a, b]);
  });

  it('is disabled while videos are being added', async () => {
    const user = userEvent.setup();
    setup({ status: 'idle' }, 1, { addingVideos: true });
    await user.click(screen.getByRole('button', { name: 'Insert ▾' }));
    const item = await screen.findByRole('menuitem', { name: /Add video/ });
    expect(item.getAttribute('aria-disabled')).toBe('true');
  });
});

describe('Insert', () => {
  it('adds a layer from Insert → Layer… when words are selected', async () => {
    const user = userEvent.setup();
    const editor = setup({ status: 'idle' }, 0, { hasSelection: true });
    await user.click(screen.getByRole('button', { name: 'Insert ▾' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Layer…' }));
    expect(editor.onAddLayer).toHaveBeenCalledOnce();
  });

  it('offers no B-roll item any more', async () => {
    const user = userEvent.setup();
    setup({ status: 'idle' }, 0, { hasSelection: true });
    await user.click(screen.getByRole('button', { name: 'Insert ▾' }));
    await screen.findByRole('menuitem', { name: 'Layer…' });
    expect(screen.queryByRole('menuitem', { name: /B-roll/ })).toBeNull();
  });
});

// @vitest-environment jsdom
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ImportItem } from '../importQueue';
import { Dropzone } from './Dropzone';

// The sample library fetches on mount; it is not under test here.
vi.mock('./Library', () => ({ Library: () => null }));

const file = (name: string) => new File(['x'], name, { type: 'video/mp4' });

function setup(imports: ImportItem[] = []) {
  const onImport = vi.fn();
  render(
    <Dropzone
      onImport={onImport}
      onLibraryClip={vi.fn()}
      busy={null}
      error={null}
      imports={imports}
    />,
  );
  return { onImport };
}

const choose = (files: File[]) =>
  fireEvent.change(screen.getByTestId('media-input'), { target: { files } });

const rows = () =>
  within(screen.getByRole('list', { name: 'Files to import' })).getAllByRole('listitem');

const names = () => rows().map((li) => li.querySelector('[title]')?.getAttribute('title'));

const importedNames = (onImport: ReturnType<typeof vi.fn>) =>
  (onImport.mock.calls[0]?.[0] as File[]).map((f) => f.name);

describe('Dropzone with several files', () => {
  it('imports a single file straight away, as before', () => {
    const { onImport } = setup();
    choose([file('a.mp4')]);
    expect(importedNames(onImport)).toEqual(['a.mp4']);
    expect(screen.queryByRole('list', { name: 'Files to import' })).toBeNull();
  });

  it('stages several files sorted by name, and imports them in the order shown', () => {
    const { onImport } = setup();
    choose([file('take10.mp4'), file('take2.mp4'), file('take1.mp4')]);
    expect(onImport).not.toHaveBeenCalled();
    expect(names()).toEqual(['take1.mp4', 'take2.mp4', 'take10.mp4']);
    fireEvent.click(screen.getByRole('button', { name: 'Move take10.mp4 up' }));
    expect(names()).toEqual(['take1.mp4', 'take10.mp4', 'take2.mp4']);
    fireEvent.click(screen.getByRole('button', { name: 'Import 3 files' }));
    expect(importedNames(onImport)).toEqual(['take1.mp4', 'take10.mp4', 'take2.mp4']);
  });

  it('reorders by dragging a row onto another', () => {
    setup();
    choose([file('a.mp4'), file('b.mp4'), file('c.mp4')]);
    const [first, , third] = rows();
    fireEvent.dragStart(third as HTMLElement);
    fireEvent.dragOver(first as HTMLElement);
    fireEvent.drop(first as HTMLElement);
    expect(names()).toEqual(['c.mp4', 'a.mp4', 'b.mp4']);
  });

  it('removes a staged file, and adds later files after the ones listed', () => {
    const { onImport } = setup();
    choose([file('b.mp4'), file('a.mp4')]);
    fireEvent.click(screen.getByRole('button', { name: 'Remove a.mp4' }));
    expect(names()).toEqual(['b.mp4']);
    choose([file('z.mp4'), file('c.mp4')]);
    expect(names()).toEqual(['b.mp4', 'c.mp4', 'z.mp4']);
    fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
    expect(screen.queryByRole('list', { name: 'Files to import' })).toBeNull();
    expect(onImport).not.toHaveBeenCalled();
  });

  it('shows the running import with progress and per-file errors, and nothing to rearrange', () => {
    setup([
      { id: '0-a', name: 'a.mp4', status: 'done', progress: 1, error: null },
      { id: '1-b', name: 'b.mp4', status: 'uploading', progress: 0.42, error: null },
      { id: '2-c', name: 'c.mov', status: 'error', progress: 0, error: 'unreadable media' },
      { id: '3-d', name: 'd.mp4', status: 'queued', progress: 0, error: null },
    ]);
    expect(screen.getByText('Added')).toBeTruthy();
    expect(
      screen.getByRole('progressbar', { name: 'Uploading b.mp4' }).getAttribute('aria-valuenow'),
    ).toBe('42');
    expect(screen.getByRole('alert').textContent).toBe('unreadable media');
    expect(screen.getByText('Waiting')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /^Import/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /^Move/ })).toBeNull();
  });
});

// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type { Asset, LayerEdit, SourceView } from '../types';
import { LayerDialog } from './LayerDialog';

const clip: Asset = {
  id: 'a1',
  kind: 'video',
  name: 'a1.mp4',
  ext: 'mp4',
  duration: 30,
  width: 1920,
  height: 1080,
  createdAt: 0,
  url: '/data/p/assets/a1',
  poster: null,
};
const short: Asset = { ...clip, id: 'a2', name: 'short.mp4', duration: 1 };
const sources: SourceView[] = [
  {
    index: 0,
    mediaId: 'm1',
    url: '/data/m1.mp4',
    filename: 'take1.mp4',
    kind: 'video',
    offset: 0,
    duration: 30,
    transcript: 'ready',
  },
  {
    index: 1,
    mediaId: 'm2',
    url: '/data/m2.mp4',
    filename: 'take2.mp4',
    kind: 'video',
    offset: 30,
    duration: 20,
    transcript: 'ready',
  },
];
const onV3: LayerEdit = {
  kind: 'layer',
  track: 3,
  start: 3,
  end: 5,
  media: 'm2',
  offset: 10,
  frame: 'pipTopRight',
  audio: -6,
};

function setup(initial?: LayerEdit) {
  const onSubmit = vi.fn();
  const onRemove = vi.fn();
  render(
    <LayerDialog
      assets={[clip, short]}
      sources={sources}
      initial={initial}
      original="hello there"
      rangeLength={4}
      onUpload={vi.fn()}
      onSubmit={onSubmit}
      onRemove={initial ? onRemove : undefined}
      onCancel={vi.fn()}
    />,
  );
  return { onSubmit, onRemove };
}

const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const level = () => screen.getByRole('slider', { name: /^Level/ }) as HTMLInputElement;

describe('LayerDialog, adding over the selected words', () => {
  it('waits for a clip, then adds it on the chosen track, frame and sound', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup();
    expect(button('Add layer').disabled).toBe(true);
    await user.click(screen.getByRole('radio', { name: /a1\.mp4/ }));
    await user.click(screen.getByRole('radio', { name: 'V3' }));
    await user.click(screen.getByRole('radio', { name: 'Top right' }));
    await user.click(screen.getByRole('checkbox', { name: 'Play its sound' }));
    await user.click(button('Add layer'));
    expect(onSubmit).toHaveBeenCalledWith({
      media: 'a1',
      offset: 0,
      track: 3,
      frame: 'pipTopRight',
      audio: -6,
    });
  });

  it('defaults to V2, full frame, sound off', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup();
    expect(screen.getByRole('radio', { name: 'V2' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Full frame' }).getAttribute('aria-checked')).toBe(
      'true',
    );
    expect(level().disabled).toBe(true);
    await user.click(screen.getByRole('radio', { name: /a1\.mp4/ }));
    await user.click(button('Add layer'));
    expect(onSubmit).toHaveBeenCalledWith({
      media: 'a1',
      offset: 0,
      track: 2,
      frame: 'full',
      audio: null,
    });
  });

  it('offers the project’s own videos, so a stretch of one can go over another', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup();
    expect(screen.getByRole('radiogroup', { name: 'This project’s videos' })).toBeTruthy();
    await user.click(screen.getByRole('radio', { name: /Video 2 · take2\.mp4/ }));
    await user.click(button('Add layer'));
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ media: 'm2', offset: 0 }));
  });

  it('sets the sound’s level', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup();
    await user.click(screen.getByRole('radio', { name: /a1\.mp4/ }));
    await user.click(screen.getByRole('checkbox', { name: 'Play its sound' }));
    fireEvent.change(level(), { target: { value: '-12' } });
    await user.click(button('Add layer'));
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ audio: -12 }));
  });

  it('refuses a shot shorter than the selected words', async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole('radio', { name: /short\.mp4/ }));
    expect(screen.getByText(/shorter than the selected words/)).toBeTruthy();
    expect(button('Add layer').disabled).toBe(true);
  });
});

describe('LayerDialog, changing a layer', () => {
  it('changes the track, frame and sound, with no picker', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup(onV3);
    expect(screen.getByText(/Showing take2\.mp4/)).toBeTruthy();
    expect(screen.queryByRole('radio', { name: /a1\.mp4/ })).toBeNull();
    expect(screen.getByRole('radio', { name: 'V3' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Top right' }).getAttribute('aria-checked')).toBe(
      'true',
    );
    expect(
      (screen.getByRole('checkbox', { name: 'Play its sound' }) as HTMLInputElement).checked,
    ).toBe(true);
    expect(level().value).toBe('-6');
    await user.click(screen.getByRole('radio', { name: 'V2' }));
    await user.click(screen.getByRole('radio', { name: 'Full frame' }));
    await user.click(screen.getByRole('checkbox', { name: 'Play its sound' }));
    await user.click(button('Save'));
    expect(onSubmit).toHaveBeenCalledWith({
      media: 'm2',
      offset: 10,
      track: 2,
      frame: 'full',
      audio: null,
    });
  });

  it('removes it', async () => {
    const user = userEvent.setup();
    const { onRemove } = setup(onV3);
    await user.click(button('Remove'));
    expect(onRemove).toHaveBeenCalledOnce();
  });
});

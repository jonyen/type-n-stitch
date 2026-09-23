// @vitest-environment jsdom
import { fireEvent, render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it, vi } from 'vitest';

import type { Asset, Edit, SourceView, Word } from '../types';
import { Transcript } from './Transcript';

const words: Word[] = ['one', 'two', 'three'].map((text, i) => ({
  id: `w${i}`,
  text,
  start: i,
  end: i + 0.5,
}));
const upload: Asset = {
  id: 'a1',
  kind: 'video',
  name: 'a1.mp4',
  ext: 'mp4',
  duration: 30,
  width: null,
  height: null,
  createdAt: 0,
  url: '/data/p/assets/a1',
  poster: null,
};
const take2: SourceView = {
  index: 1,
  mediaId: 'm2',
  url: '/data/m2.mp4',
  filename: 'take2.mp4',
  kind: 'video',
  offset: 30,
  duration: 20,
  transcript: 'ready',
};

function setup(overrides: Partial<ComponentProps<typeof Transcript>> = {}) {
  const onWordDrag = vi.fn();
  const onWordDragEnd = vi.fn();
  const onLayerClick = vi.fn();
  render(
    <Transcript
      words={words}
      edits={[]}
      selected={null}
      activeWord={-1}
      playing={false}
      showCuts
      onWordClick={vi.fn()}
      onWordDrag={onWordDrag}
      onOverdubClick={vi.fn()}
      selectedTitle={null}
      onTitleClick={vi.fn()}
      onTitleOpen={vi.fn()}
      onCaptionClick={vi.fn()}
      onCutTransition={vi.fn()}
      speakers={null}
      speakerNames={[]}
      onRenameSpeaker={vi.fn()}
      readOnly={false}
      peers={[]}
      ordered={[{ start: 0, end: 3 }]}
      splits={[]}
      selectedClip={null}
      onClipClick={vi.fn()}
      assets={[upload]}
      sources={[take2]}
      onLayerClick={onLayerClick}
      onAudioClick={vi.fn()}
      tool="range"
      onWordDragEnd={onWordDragEnd}
      {...overrides}
    />,
  );
  return { onWordDrag, onWordDragEnd, onLayerClick };
}

describe('Transcript word drag (the Range tool cuts on its end)', () => {
  it('a plain click is no drag: nothing ends, so nothing is cut', () => {
    const { onWordDragEnd } = setup();
    fireEvent.mouseDown(screen.getByRole('button', { name: 'two' }), { button: 0 });
    fireEvent.mouseUp(window);
    expect(onWordDragEnd).not.toHaveBeenCalled();
  });

  it('a press dragged across words ends the drag on release', () => {
    const { onWordDrag, onWordDragEnd } = setup();
    fireEvent.mouseDown(screen.getByRole('button', { name: 'one' }), { button: 0 });
    fireEvent.mouseEnter(screen.getByRole('button', { name: 'two' }));
    fireEvent.mouseUp(window);
    expect(onWordDrag).toHaveBeenCalledWith(1);
    expect(onWordDragEnd).toHaveBeenCalledOnce();
    // The next plain click starts over.
    fireEvent.mouseDown(screen.getByRole('button', { name: 'three' }), { button: 0 });
    fireEvent.mouseUp(window);
    expect(onWordDragEnd).toHaveBeenCalledOnce();
  });
});

describe('Transcript layer tags', () => {
  const layered: Edit[] = [
    {
      kind: 'layer',
      track: 2,
      start: 0,
      end: 1,
      media: 'a1',
      offset: 0,
      frame: 'full',
      audio: null,
    },
    {
      kind: 'layer',
      track: 3,
      start: 1,
      end: 3,
      media: 'm2',
      offset: 4,
      frame: 'pipTopRight',
      audio: -6,
    },
  ];

  it('reads "V3 · name · PiP ↗", with a speaker when its sound is on', () => {
    setup({ edits: layered });
    const v2 = screen.getByRole('button', { name: 'V2 · a1.mp4' });
    expect(within(v2).queryByRole('img')).toBeNull();
    const v3 = screen.getByRole('button', { name: /^V3 · take2\.mp4 · PiP ↗/ });
    expect(within(v3).getByRole('img', { name: 'Sound on, -6 dB' })).toBeTruthy();
  });

  it('follows the first word each layer covers', () => {
    setup({ edits: layered });
    const tag = screen.getByRole('button', { name: /^V3 · take2\.mp4/ });
    expect(tag.getAttribute('data-layer-tag')).toBe('3:1');
    // The word "two" (index 1) is the tag's nearest button before it.
    expect(tag.previousElementSibling?.textContent).toBe('two');
  });

  it('selects the layer on click, with its track', () => {
    const { onLayerClick } = setup({ edits: layered });
    fireEvent.click(screen.getByRole('button', { name: /^V3 · take2\.mp4/ }));
    expect(onLayerClick).toHaveBeenCalledWith(3, 1);
  });
});

const video = (
  index: number,
  offset: number,
  duration: number,
  transcript: SourceView['transcript'],
): SourceView => ({
  index,
  mediaId: `m${index}`,
  url: `/m${index}`,
  filename: `take${index + 1}.mp4`,
  kind: 'video',
  offset,
  duration,
  transcript,
});

describe('Transcript with several videos', () => {
  // The three words fill video 1, [0, 3); video 2 is [3, 8) with no words yet.
  const twoClips = {
    ordered: [
      { start: 0, end: 3 },
      { start: 3, end: 8 },
    ],
    splits: [3],
  };

  it('greys out a video that is still transcribing, in its own clip', () => {
    setup({ ...twoClips, sources: [video(0, 0, 3, 'ready'), video(1, 3, 5, 'running')] });
    const block = screen.getByRole('status');
    expect(block.textContent).toBe('Transcribing video 2…');
    expect(block.closest('section')?.getAttribute('data-clip-start')).toBe('3');
  });

  it('says so when a transcript failed, and shows nothing once it is ready', () => {
    setup({ ...twoClips, sources: [video(0, 0, 3, 'ready'), video(1, 3, 5, 'error')] });
    expect(screen.getByRole('status').textContent).toBe('Video 2 could not be transcribed');
  });

  it('labels a join as where a video starts, not as a split Delete could join', () => {
    setup({ ...twoClips, sources: [video(0, 0, 3, 'ready'), video(1, 3, 5, 'ready')] });
    expect(screen.queryByRole('status')).toBeNull();
    const divider = screen.getByRole('button', { name: /^Video 2 · Clip 2/ });
    expect(divider.getAttribute('title')).toMatch(/video 2 starts/i);
  });
});

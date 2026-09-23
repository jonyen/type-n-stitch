// @vitest-environment jsdom
import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { DUCK, gainToLinear } from '../overlays';
import type { Asset, Edit, SourceView, Word } from '../types';
import type { Playback } from '../usePlayback';
import { Overlays } from './Overlays';

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
// V2 full frame and muted over [2, 6); V3 a top-right PiP of video 2, with sound, over [3, 5).
const edits: Edit[] = [
  {
    kind: 'layer',
    track: 3,
    start: 3,
    end: 5,
    media: 'm2',
    offset: 10,
    frame: 'pipTopRight',
    audio: -6,
  },
  {
    kind: 'layer',
    track: 2,
    start: 2,
    end: 6,
    media: 'a1',
    offset: 0,
    frame: 'full',
    audio: null,
  },
];

function playbackAt(currentTime: number, playing = false): Playback {
  return {
    playing,
    currentTime,
    outputTime: currentTime,
    atEnd: false,
    activeWord: -1,
    overdubbing: null,
    titling: null,
    toggle: vi.fn(),
    seek: vi.fn(),
    seekOutput: vi.fn(),
  } as Playback;
}

function show(t: number, opts: { playing?: boolean; words?: Word[]; edits?: Edit[] } = {}) {
  return render(
    <Overlays
      edits={opts.edits ?? edits}
      assets={[upload]}
      sources={[take2]}
      words={opts.words ?? []}
      playback={playbackAt(t, opts.playing)}
    />,
  );
}

const stack = (root: HTMLElement) => Array.from(root.querySelectorAll<HTMLElement>('[data-layer]'));
const videoOf = (el: HTMLElement) => el.querySelector('video') as HTMLVideoElement;

let play: ReturnType<typeof vi.fn>;
beforeEach(() => {
  // jsdom implements neither; play must return a promise, as browsers do.
  play = vi.fn(() => Promise.resolve());
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(play);
  vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => undefined);
});
afterEach(() => vi.restoreAllMocks());

describe('Overlays: the layer stack', () => {
  it('stacks one video per visible layer, upper tracks painted last', () => {
    const { container } = show(4);
    const layers = stack(container);
    expect(layers.map((l) => l.getAttribute('data-layer'))).toEqual(['2:2', '3:3']);
    expect(layers.map((l) => videoOf(l).getAttribute('src'))).toEqual([
      '/data/p/assets/a1',
      '/data/m2.mp4',
    ]);
  });

  it('shows only the layers under the playhead', () => {
    expect(stack(show(5.5).container).map((l) => l.getAttribute('data-layer'))).toEqual(['2:2']);
  });

  it('shows nothing where no layer is', () => {
    expect(stack(show(7).container)).toHaveLength(0);
  });

  it('insets a picture-in-picture 30% wide from its corner, and fills the frame otherwise', () => {
    const [full, pip] = stack(show(4).container) as [HTMLElement, HTMLElement];
    expect(full.getAttribute('data-frame')).toBe('full');
    expect(full.style.width).toBe('');
    expect(pip.getAttribute('data-frame')).toBe('pipTopRight');
    expect(pip.style.width).toBe('30%');
    expect(pip.style.top).toBe('4%');
    expect(pip.style.right).toBe('4%');
    expect(pip.style.left).toBe('');
    expect(pip.style.bottom).toBe('');
  });

  it('mutes a layer without sound, and plays one with sound at its level', () => {
    const [full, pip] = stack(show(4).container) as [HTMLElement, HTMLElement];
    expect(videoOf(full).muted).toBe(true);
    expect(videoOf(pip).muted).toBe(false);
    expect(videoOf(pip).volume).toBeCloseTo(gainToLinear(-6), 5);
  });

  it('ducks a layer’s sound under speech, as music ducks', () => {
    const words: Word[] = [{ id: 'w', text: 'hi', start: 3.8, end: 4.4 }];
    const [, pip] = stack(show(4, { words }).container) as [HTMLElement, HTMLElement];
    expect(videoOf(pip).volume).toBeCloseTo(gainToLinear(-6) * DUCK, 5);
  });

  it('keeps each layer on the playhead, the way B-roll did', () => {
    const [full, pip] = stack(show(4, { playing: true }).container) as [HTMLElement, HTMLElement];
    // Seconds into each file: offset + (t - start).
    expect(videoOf(full).currentTime).toBe(2);
    expect(videoOf(pip).currentTime).toBe(11);
    expect(play).toHaveBeenCalledTimes(2);
  });

  it('leaves a layer alone while it is within 0.3 s of the playhead', () => {
    const { container, rerender } = show(4);
    const pip = stack(container)[1] as HTMLElement;
    videoOf(pip).currentTime = 11.2;
    rerender(
      <Overlays
        edits={edits}
        assets={[upload]}
        sources={[take2]}
        words={[]}
        playback={playbackAt(4.1)}
      />,
    );
    expect(videoOf(pip).currentTime).toBe(11.2);
  });

  it('draws no layer, picture or sound, over a title card, as the export does', () => {
    const titling = { ...playbackAt(4), titling: { text: 'T' } } as unknown as Playback;
    const { container } = render(
      <Overlays edits={edits} assets={[upload]} sources={[take2]} words={[]} playback={titling} />,
    );
    expect(stack(container)).toHaveLength(0);
  });

  it('marks a layer whose file is missing', () => {
    const lost: Edit[] = [
      {
        kind: 'layer',
        track: 2,
        start: 2,
        end: 6,
        media: 'gone',
        offset: 0,
        frame: 'full',
        audio: null,
      },
    ];
    show(4, { edits: lost });
    expect(screen.getByText('Layer file missing')).toBeTruthy();
  });
});

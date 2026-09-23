import { describe, expect, it } from 'vitest';

import {
  assetTime,
  audiosAt,
  FRAMES,
  frameLabel,
  gainToLinear,
  layers,
  layersAt,
  layerTag,
  layerVolume,
  mediaName,
  mediaUrl,
  pipPlacement,
  speaking,
} from './overlays';
import type { Asset, Edit, LayerEdit, SourceView, Word } from './types';

const edits: Edit[] = [
  {
    kind: 'layer',
    track: 2,
    start: 2,
    end: 4,
    media: 'b',
    offset: 1.5,
    frame: 'full',
    audio: null,
  },
  {
    kind: 'layer',
    track: 3,
    start: 6,
    end: 8,
    media: 'c',
    offset: 0,
    frame: 'pipTopLeft',
    audio: -6,
  },
  { kind: 'audio', start: 0, end: 10, media: 'm', offset: 3, gain: -6, duck: true },
];
const words: Word[] = [{ id: 'w', text: 'x', start: 1, end: 1.5 }];

describe('overlays', () => {
  it('finds the overlays under a source time and maps into the asset', () => {
    expect(layersAt(3, edits).map((l) => l.media)).toEqual(['b']);
    expect(layersAt(4, edits)).toEqual([]);
    expect(layersAt(7, edits).map((l) => l.media)).toEqual(['c']);
    expect(audiosAt(9, edits)).toHaveLength(1);
    expect(assetTime(edits[0] as { start: number; offset: number }, 3)).toBe(2.5);
    expect(gainToLinear(-6)).toBeCloseTo(0.501, 3);
    expect(gainToLinear(0)).toBe(1);
    expect(speaking(1.2, words)).toBe(true);
    expect(speaking(1.7, words)).toBe(false);
  });

  it('lists layers on one track or on all of them', () => {
    expect(layers(edits).map((l) => l.media)).toEqual(['b', 'c']);
    expect(layers(edits, 2).map((l) => l.media)).toEqual(['b']);
    expect(layers(edits, 3).map((l) => l.media)).toEqual(['c']);
  });
});

const v2: LayerEdit = {
  kind: 'layer',
  track: 2,
  start: 2,
  end: 6,
  media: 'a1',
  offset: 0,
  frame: 'full',
  audio: null,
};
const v3: LayerEdit = {
  kind: 'layer',
  track: 3,
  start: 3,
  end: 5,
  media: 'm2',
  offset: 10,
  frame: 'pipTopRight',
  audio: -6,
};
const stacked: Edit[] = [
  v3,
  { kind: 'audio', start: 0, end: 10, media: 'mu', offset: 0, gain: 0, duck: true },
  v2,
];
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

describe('layers', () => {
  it('lists layers per track, and those under a time lower track first', () => {
    expect(layers(stacked)).toEqual([v3, v2]);
    expect(layers(stacked, 2)).toEqual([v2]);
    expect(layers(stacked, 3)).toEqual([v3]);
    expect(layersAt(4, stacked)).toEqual([v2, v3]);
    expect(layersAt(5, stacked)).toEqual([v2]);
    expect(layersAt(6, stacked)).toEqual([]);
  });

  it('labels a layer the way its transcript tag reads', () => {
    expect(FRAMES).toEqual([
      'full',
      'pipTopLeft',
      'pipTopRight',
      'pipBottomLeft',
      'pipBottomRight',
    ]);
    expect(frameLabel('full')).toBe('Full');
    expect(frameLabel('pipTopLeft')).toBe('PiP ↖');
    expect(frameLabel('pipTopRight')).toBe('PiP ↗');
    expect(frameLabel('pipBottomLeft')).toBe('PiP ↙');
    expect(frameLabel('pipBottomRight')).toBe('PiP ↘');
    expect(layerTag(v3, 'take2.mp4')).toBe('V3 · take2.mp4 · PiP ↗');
    expect(layerTag(v2, 'a1.mp4')).toBe('V2 · a1.mp4');
  });

  it('turns a level in dB into a preview volume, and no sound into muted', () => {
    expect(layerVolume(null)).toBeNull();
    expect(layerVolume(0)).toBe(1);
    expect(layerVolume(-6)).toBeCloseTo(0.501, 3);
    // HTMLMediaElement.volume tops out at 1; the export applies the boost.
    expect(layerVolume(6)).toBe(1);
  });

  it('places a picture-in-picture 30% wide, 4% in from its corner', () => {
    expect(pipPlacement('full')).toBeNull();
    expect(pipPlacement('pipTopLeft')).toEqual({ width: '30%', top: '4%', left: '4%' });
    expect(pipPlacement('pipTopRight')).toEqual({ width: '30%', top: '4%', right: '4%' });
    expect(pipPlacement('pipBottomLeft')).toEqual({ width: '30%', bottom: '4%', left: '4%' });
    expect(pipPlacement('pipBottomRight')).toEqual({ width: '30%', bottom: '4%', right: '4%' });
  });

  it('finds a layer’s file among the uploads or the project’s own videos', () => {
    expect(mediaName('a1', [upload], [take2])).toBe('a1.mp4');
    expect(mediaName('m2', [upload], [take2])).toBe('take2.mp4');
    expect(mediaName('zz', [upload], [take2])).toBeUndefined();
    expect(mediaUrl('a1', [upload], [take2])).toBe('/data/p/assets/a1');
    expect(mediaUrl('m2', [upload], [take2])).toBe('/data/m2.mp4');
  });
});

import { describe, expect, it } from 'vitest';

import { assetTime, audiosAt, gainToLinear, layerAt, layers, speaking } from './overlays';
import type { Edit, Word } from './types';

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
    expect(layerAt(3, edits, 2)?.media).toBe('b');
    expect(layerAt(4, edits, 2)).toBeUndefined();
    expect(layerAt(3, edits, 3)).toBeUndefined();
    expect(layerAt(7, edits, 3)?.media).toBe('c');
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

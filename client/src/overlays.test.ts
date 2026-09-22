import { describe, expect, it } from 'vitest';

import { assetTime, audiosAt, brollAt, gainToLinear, speaking } from './overlays';
import type { Edit, Word } from './types';

const edits: Edit[] = [
  { kind: 'broll', start: 2, end: 4, media: 'b', offset: 1.5 },
  { kind: 'audio', start: 0, end: 10, media: 'm', offset: 3, gain: -6, duck: true },
];
const words: Word[] = [{ id: 'w', text: 'x', start: 1, end: 1.5 }];

describe('overlays', () => {
  it('finds the overlays under a source time and maps into the asset', () => {
    expect(brollAt(3, edits)?.media).toBe('b');
    expect(brollAt(4, edits)).toBeUndefined();
    expect(audiosAt(9, edits)).toHaveLength(1);
    expect(assetTime(edits[0] as { start: number; offset: number }, 3)).toBe(2.5);
    expect(gainToLinear(-6)).toBeCloseTo(0.501, 3);
    expect(gainToLinear(0)).toBe(1);
    expect(speaking(1.2, words)).toBe(true);
    expect(speaking(1.7, words)).toBe(false);
  });
});

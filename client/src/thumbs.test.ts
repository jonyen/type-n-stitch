import { describe, expect, it } from 'vitest';

import { sheetFor, spriteStyle } from './thumbs';

describe('thumbnail sheet (engine cases)', () => {
  it('samples short sources every half second', () => {
    expect(sheetFor(20)).toEqual({
      count: 40,
      columns: 10,
      rows: 4,
      interval: 0.5,
      width: 160,
      height: 90,
    });
  });

  it('caps long sources at 120 frames', () => {
    const sheet = sheetFor(3600);
    expect(sheet.count).toBe(120);
    expect(sheet.interval).toBe(30);
    expect(sheet.rows).toBe(12);
  });

  it('gives a tiny source one cell', () => {
    expect(sheetFor(0.2)).toMatchObject({ count: 1, columns: 1, rows: 1 });
  });
});

describe('spriteStyle', () => {
  it('scales the sheet to the element and positions one cell by percentage', () => {
    const style = spriteStyle(sheetFor(20), '/data/m/thumbs-v1.jpg', 12);
    expect(style.backgroundImage).toBe('url(/data/m/thumbs-v1.jpg)');
    expect(style.backgroundSize).toBe('1000% 400%');
    // Cell 12 is column 2 of 10, row 1 of 4.
    expect(style.backgroundPosition).toBe(`${(2 / 9) * 100}% ${(1 / 3) * 100}%`);
  });
});

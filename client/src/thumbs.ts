// The scrubber sprite sheet's layout, computed the way the engine does
// (engine/src/thumbnails.rs `thumbnail_sheet`), so a cached sheet can be
// shown on the home screen without asking the server to render one.

import type { CSSProperties } from 'react';

import type { Thumbnails } from './api';

/** The cached sheet's file name inside a media directory (server/src/routes.rs `THUMBS_CACHE`). */
export const THUMBS_FILE = 'thumbs-v1.jpg';

const MAX_THUMBS = 120;
const MIN_INTERVAL = 0.5;
const COLUMNS = 10;

export function sheetFor(duration: number): Omit<Thumbnails, 'url'> {
  const d = Math.max(duration, 0);
  const interval = Math.max(d / MAX_THUMBS, MIN_INTERVAL);
  const count = Math.min(Math.max(Math.ceil(d / interval), 1), MAX_THUMBS);
  const columns = Math.min(count, COLUMNS);
  return { count, columns, rows: Math.ceil(count / columns), interval, width: 160, height: 90 };
}

/** Background styles that show cell `index` of the sheet, scaled to fill a 16:9 element. */
export function spriteStyle(
  sheet: Omit<Thumbnails, 'url'>,
  url: string,
  index: number,
): CSSProperties {
  const cell = Math.min(Math.max(index, 0), sheet.count - 1);
  const col = cell % sheet.columns;
  const row = Math.floor(cell / sheet.columns);
  const x = sheet.columns > 1 ? (col / (sheet.columns - 1)) * 100 : 0;
  const y = sheet.rows > 1 ? (row / (sheet.rows - 1)) * 100 : 0;
  return {
    backgroundImage: `url(${url})`,
    backgroundSize: `${sheet.columns * 100}% ${sheet.rows * 100}%`,
    backgroundPosition: `${x}% ${y}%`,
  };
}

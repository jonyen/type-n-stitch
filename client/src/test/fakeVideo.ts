import { vi } from 'vitest';

/**
 * Just enough of a <video> for the stitched clock: a src that loads paused at
 * 0 (no pause event, as in the HTML load algorithm), a settable clock,
 * play/pause and their events, and `loaded()` for a file's metadata arriving.
 */
export class FakeVideo extends EventTarget {
  private file = '';
  currentTime = 0;
  paused = true;
  ended = false;
  videoWidth = 0;
  videoHeight = 0;
  get src(): string {
    return this.file;
  }
  set src(url: string) {
    this.file = url;
    this.paused = true;
    this.ended = false;
    this.currentTime = 0;
  }
  play = vi.fn(() => {
    this.paused = false;
    this.dispatchEvent(new Event('play'));
    return Promise.resolve();
  });
  pause = vi.fn(() => {
    if (this.paused) return;
    this.paused = true;
    this.dispatchEvent(new Event('pause'));
  });
  loaded(width = 0, height = 0) {
    this.videoWidth = width;
    this.videoHeight = height;
    this.dispatchEvent(new Event('loadedmetadata'));
  }
}

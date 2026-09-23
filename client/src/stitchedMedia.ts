// A clock over the main track's files, played through one <video>. The
// playback hook reads and sets stitched seconds; this swaps the element's
// file when that time moves into another source, and holds the element's own
// events back while the new file loads, so the hook's piece tracking
// (`playIndex`, `ended`) never sees the swap.

import { useEffect, useState } from 'react';

import { locate } from './editlist';
import type { MediaKind } from './types';
import type { PlaybackMedia } from './usePlayback';

export interface PlayableSource {
  offset: number;
  duration: number;
  url: string;
  /** An audio-only file never supplies the canvas. `SourceView` carries it. */
  kind?: MediaKind;
}

/** Element events the playback hook listens for. */
const FORWARDED = ['play', 'pause', 'ended', 'timeupdate', 'seeked'] as const;

export class StitchedMedia extends EventTarget implements PlaybackMedia {
  private el: HTMLVideoElement | null = null;
  private list: PlayableSource[] = [];
  /** The source whose file the element holds (or is loading). */
  private index = 0;
  /** The url last given to the element. */
  private loadedUrl: string | null = null;
  /** The stitched instant a swap is heading for; null when no file is loading. */
  private pending: number | null = null;
  /** Play once the swap lands. */
  private resume = false;
  private preloader: HTMLVideoElement | null = null;
  /** Each file's width over height once its metadata is in (0: no picture), by url. */
  private shapes = new Map<string, number>();
  /**
   * The canvas: the first file with a picture, as the export picks it. Null
   * until that is known, and when no file has one; the Player then uses 16:9.
   */
  aspect: number | null = null;

  private readonly forward = (e: Event) => {
    if (this.pending !== null) return;
    this.dispatchEvent(new Event(e.type));
  };

  private readonly landed = () => {
    const el = this.el;
    if (!el) return;
    const url = this.list[this.index]?.url;
    if (url) this.learn(el, url);
    if (this.pending === null) return;
    const hit = locate(this.list, this.pending);
    const resume = this.resume;
    this.pending = null;
    this.resume = false;
    if (hit && hit.index === this.index) el.currentTime = hit.local;
    this.dispatchEvent(new Event('seeked'));
    if (resume)
      void el.play().catch(() => {
        // Autoplay refused: say so, so the transport shows paused.
        this.dispatchEvent(new Event('pause'));
      });
  };

  /** The callback ref for the Player's <video>. Stable for the clock's life. */
  readonly attach = (el: HTMLVideoElement | null): void => {
    if (el === this.el) return;
    if (this.el) {
      for (const type of FORWARDED) this.el.removeEventListener(type, this.forward);
      this.el.removeEventListener('loadedmetadata', this.landed);
    }
    this.el = el;
    this.loadedUrl = null;
    this.pending = null;
    this.resume = false;
    if (!el) return;
    for (const type of FORWARDED) el.addEventListener(type, this.forward);
    el.addEventListener('loadedmetadata', this.landed);
    const source = this.list[this.index];
    if (source) this.load(source.url);
  };

  /** The main track's files in stitched order. The loaded file stays if it is still there. */
  setSources(list: PlayableSource[]): void {
    this.list = list;
    if (this.index >= list.length) this.index = 0;
    const source = list[this.index];
    if (source && source.url !== this.loadedUrl) this.load(source.url);
    this.preloadNext();
    this.updateAspect();
  }

  /** The url being warmed for the next swap, if any. */
  get preloading(): string | null {
    return this.preloader?.getAttribute('src') ?? null;
  }

  get currentTime(): number {
    if (this.pending !== null) return this.pending;
    return (this.list[this.index]?.offset ?? 0) + (this.el?.currentTime ?? 0);
  }

  set currentTime(t: number) {
    const hit = locate(this.list, t);
    const el = this.el;
    if (!hit || !el) return;
    if (hit.index === this.index) {
      // Mid-swap to this file: just aim the landing elsewhere.
      if (this.pending !== null) this.pending = t;
      else el.currentTime = hit.local;
      return;
    }
    // A swap started while playing carries on playing.
    if (this.pending === null && !el.paused) this.resume = true;
    this.index = hit.index;
    this.pending = t;
    this.load((this.list[hit.index] as PlayableSource).url);
    this.preloadNext();
  }

  get paused(): boolean {
    if (this.pending !== null) return !this.resume;
    return this.el?.paused ?? true;
  }

  get ended(): boolean {
    return (
      this.pending === null && this.index === this.list.length - 1 && (this.el?.ended ?? false)
    );
  }

  play(): Promise<void> {
    if (!this.el) return Promise.resolve();
    if (this.pending !== null) {
      if (!this.resume) {
        this.resume = true;
        this.dispatchEvent(new Event('play'));
      }
      return Promise.resolve();
    }
    return this.el.play();
  }

  pause(): void {
    if (this.pending !== null) {
      if (this.resume) {
        this.resume = false;
        this.dispatchEvent(new Event('pause'));
      }
      return;
    }
    this.el?.pause();
  }

  private load(url: string) {
    if (!this.el) return;
    this.loadedUrl = url;
    this.el.src = url;
  }

  /** Record `url`'s picture shape from an element whose metadata just loaded. */
  private learn(el: HTMLVideoElement, url: string) {
    const shape = el.videoWidth > 0 && el.videoHeight > 0 ? el.videoWidth / el.videoHeight : 0;
    this.shapes.set(url, shape);
    this.updateAspect();
  }

  /** The first file with a picture, in stitched order, once every file before it is known. */
  private updateAspect() {
    let next: number | null = null;
    for (const s of this.list) {
      if (s.kind === 'audio') continue;
      const shape = this.shapes.get(s.url);
      // Not loaded yet: an earlier file may still turn out to have a picture.
      if (shape === undefined) break;
      if (shape > 0) {
        next = shape;
        break;
      }
    }
    if (next !== this.aspect) {
      this.aspect = next;
      this.dispatchEvent(new Event('aspect'));
    }
  }

  /** Point a detached, muted element at the next file so its swap starts warm. */
  private preloadNext() {
    const next = this.list[this.index + 1];
    if (!next || typeof document === 'undefined') return;
    if (!this.preloader) {
      const preloader = document.createElement('video');
      preloader.preload = 'auto';
      preloader.muted = true;
      // A warmed file's shape counts toward the canvas before it plays.
      preloader.addEventListener('loadedmetadata', () => {
        const url = preloader.getAttribute('src');
        if (url) this.learn(preloader, url);
      });
      this.preloader = preloader;
    }
    if (this.preloader.getAttribute('src') !== next.url)
      this.preloader.setAttribute('src', next.url);
  }
}

/** One clock per mounted editor, fed the current sources. */
export function useStitchedMedia(sources: PlayableSource[]): StitchedMedia {
  const [media] = useState(() => new StitchedMedia());
  useEffect(() => media.setSources(sources), [media, sources]);
  return media;
}

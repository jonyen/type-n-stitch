// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { StitchedMedia, type PlayableSource } from './stitchedMedia';
import { FakeVideo } from './test/fakeVideo';

const three: PlayableSource[] = [
  { offset: 0, duration: 10, url: '/a' },
  { offset: 10, duration: 5, url: '/b' },
  { offset: 15, duration: 2.5, url: '/c' },
];

// jsdom has no media loading: releasing the warmed file calls load().
beforeEach(() => {
  vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => undefined);
});

function setup(list = three) {
  const media = new StitchedMedia();
  media.setSources(list);
  const video = new FakeVideo();
  media.attach(video as unknown as HTMLVideoElement);
  const events: string[] = [];
  for (const type of ['play', 'pause', 'seeked', 'ended'])
    media.addEventListener(type, () => events.push(type));
  return { media, video, events };
}

describe('StitchedMedia', () => {
  it('reads and seeks stitched time through the file that holds it', () => {
    const { media, video } = setup();
    expect(video.src).toBe('/a');
    video.currentTime = 4;
    expect(media.currentTime).toBe(4);
    media.currentTime = 6.5;
    expect(video.src).toBe('/a');
    expect(video.currentTime).toBe(6.5);
  });

  it('swaps files when a seek lands in another source, holding the target until it loads', () => {
    const { media, video } = setup();
    media.currentTime = 12;
    expect(video.src).toBe('/b');
    expect(media.currentTime).toBe(12);
    video.loaded();
    expect(video.currentTime).toBe(2);
    expect(media.currentTime).toBe(12);
    // A join belongs to the later file; the exact end is the last file at its length.
    media.currentTime = 15;
    expect(video.src).toBe('/c');
    video.loaded();
    expect(video.currentTime).toBe(0);
    media.currentTime = 17.5;
    expect(video.currentTime).toBe(2.5);
  });

  it('keeps playing across a swap without telling the hook it paused', async () => {
    const { media, video, events } = setup();
    await media.play();
    expect(events).toEqual(['play']);
    media.currentTime = 10;
    // Whatever the element says while the new file loads is held back.
    video.dispatchEvent(new Event('pause'));
    video.dispatchEvent(new Event('timeupdate'));
    expect(media.paused).toBe(false);
    expect(events).toEqual(['play']);
    video.loaded();
    expect(video.play).toHaveBeenCalledTimes(2);
    expect(events).toEqual(['play', 'seeked', 'play']);
  });

  it('reports a play or pause made during a swap at once, and honours the last one', async () => {
    const { media, video, events } = setup();
    media.currentTime = 12;
    expect(media.paused).toBe(true);
    await media.play();
    expect(media.paused).toBe(false);
    expect(video.play).not.toHaveBeenCalled();
    media.pause();
    expect(events).toEqual(['play', 'pause']);
    video.loaded();
    expect(video.play).not.toHaveBeenCalled();
    expect(video.currentTime).toBe(2);
  });

  it('forwards a file ending, but is ended only at the last file', () => {
    const { media, video, events } = setup();
    video.ended = true;
    video.dispatchEvent(new Event('ended'));
    expect(events).toEqual(['ended']);
    expect(media.ended).toBe(false);
    media.currentTime = 16;
    video.loaded();
    video.ended = true;
    expect(media.ended).toBe(true);
  });

  it('takes the canvas from the first file with a picture, and keeps it', () => {
    const { media, video } = setup();
    const onAspect = vi.fn();
    media.addEventListener('aspect', onAspect);
    video.loaded(1920, 1080);
    expect(media.aspect).toBeCloseTo(16 / 9);
    media.currentTime = 12;
    video.loaded(640, 480);
    expect(media.aspect).toBeCloseTo(16 / 9);
    expect(onAspect).toHaveBeenCalledOnce();
  });

  it('skips a first file with no picture, as the export does', () => {
    const { media, video } = setup([
      { offset: 0, duration: 10, url: '/a', kind: 'audio' },
      { offset: 10, duration: 5, url: '/b', kind: 'video' },
    ]);
    video.loaded(0, 0);
    // The second file's shape is not known yet: no canvas, so the Player shows 16:9.
    expect(media.aspect).toBeNull();
    media.currentTime = 12;
    video.loaded(640, 480);
    expect(media.aspect).toBeCloseTo(4 / 3);
  });

  it('has no canvas while no file shows a picture', () => {
    const { media, video } = setup(three.slice(0, 1));
    video.loaded(0, 0);
    expect(media.aspect).toBeNull();
  });

  it('warms the next file', () => {
    const { media, video } = setup();
    expect(media.preloading).toBe('/b');
    media.currentTime = 11;
    video.loaded();
    expect(media.preloading).toBe('/c');
  });

  it('keeps the loaded file when a source is appended', () => {
    const { media, video } = setup(three.slice(0, 1));
    video.currentTime = 3;
    expect(media.preloading).toBeNull();
    media.setSources(three);
    expect(video.src).toBe('/a');
    expect(video.currentTime).toBe(3);
    expect(media.preloading).toBe('/b');
  });

  it('does nothing outside the stitched timeline', () => {
    const { media, video } = setup();
    media.currentTime = 99;
    expect(video.src).toBe('/a');
    expect(video.currentTime).toBe(0);
  });
});

describe('StitchedMedia when things go wrong', () => {
  it('gives up a swap whose file fails to load, and says it paused', async () => {
    const { media, video, events } = setup();
    await media.play();
    media.currentTime = 12;
    video.dispatchEvent(new Event('error'));
    expect(media.paused).toBe(true);
    expect(events).toEqual(['play', 'pause']);
    // The clock reads the element again, and a later seek still works.
    expect(media.currentTime).toBe(10);
    media.currentTime = 3;
    expect(video.src).toBe('/a');
    video.loaded();
    expect(video.currentTime).toBe(3);
    expect(video.play).toHaveBeenCalledOnce();
  });

  it('swallows a play() cut short by a swap or pause, but reports a refusal as a pause', async () => {
    const { media, video, events } = setup();
    video.play.mockImplementationOnce(() =>
      Promise.reject(new DOMException('interrupted', 'AbortError')),
    );
    await expect(media.play()).resolves.toBeUndefined();
    expect(events).toEqual([]);
    video.play.mockImplementationOnce(() =>
      Promise.reject(new DOMException('no autoplay', 'NotAllowedError')),
    );
    await expect(media.play()).resolves.toBeUndefined();
    expect(events).toEqual(['pause']);
  });

  it('lets go of the warmed file when there is no next one', () => {
    const { media } = setup();
    expect(media.preloading).toBe('/b');
    media.setSources(three.slice(0, 1));
    expect(media.preloading).toBeNull();
    media.setSources(three);
    expect(media.preloading).toBe('/b');
    media.setSources([]);
    expect(media.preloading).toBeNull();
  });
});

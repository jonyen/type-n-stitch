// Playback that honours the edit list live: cuts are skipped as the
// playhead reaches them, and overdubs freeze the picture on their first
// frame while the synthesized audio plays through a second element.

import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';

import { cutRanges, overdubAt, skipTarget, wordIndexAt } from './editlist';
import type { Edit, OverdubEdit, Word } from './types';

export interface Playback {
  playing: boolean;
  currentTime: number;
  /** Index of the word under the playhead, or -1. */
  activeWord: number;
  /** The overdub whose audio is currently playing, if any. */
  overdubbing: OverdubEdit | null;
  toggle: () => void;
  seek: (t: number) => void;
}

export function usePlayback(
  mediaRef: RefObject<HTMLMediaElement | null>,
  words: Word[],
  edits: Edit[],
  duration: number,
  /** Media URL. The element mounts after the hook, so listeners re-attach when it changes. */
  src: string | undefined,
): Playback {
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [activeWord, setActiveWord] = useState(-1);
  const [overdubbing, setOverdubbing] = useState<OverdubEdit | null>(null);

  // Latest props for the animation-frame loop without re-subscribing.
  const editsRef = useRef(edits);
  const wordsRef = useRef(words);
  editsRef.current = edits;
  wordsRef.current = words;

  const wantPlaying = useRef(false);
  const activeOverdub = useRef<OverdubEdit | null>(null);
  const overdubAudio = useRef<HTMLAudioElement | null>(null);
  const frame = useRef(0);

  const audio = useCallback(() => {
    overdubAudio.current ??= new Audio();
    return overdubAudio.current;
  }, []);

  const sync = useCallback(() => {
    const media = mediaRef.current;
    if (!media) return;
    const t = media.currentTime;
    setCurrentTime(Math.round(t * 10) / 10);
    setActiveWord(wordIndexAt(t, wordsRef.current));
  }, [mediaRef]);

  const leaveOverdub = useCallback(
    (resume: boolean) => {
      const od = activeOverdub.current;
      const media = mediaRef.current;
      activeOverdub.current = null;
      setOverdubbing(null);
      audio().pause();
      if (!od || !media) return;
      media.currentTime = od.end;
      if (resume && wantPlaying.current) void media.play();
    },
    [audio, mediaRef],
  );

  const enterOverdub = useCallback(
    (od: OverdubEdit) => {
      const media = mediaRef.current;
      if (!media) return;
      activeOverdub.current = od;
      setOverdubbing(od);
      media.pause();
      media.currentTime = od.start;
      const a = audio();
      a.src = od.audioUrl;
      a.onended = () => leaveOverdub(true);
      a.onerror = () => leaveOverdub(true);
      void a.play();
    },
    [audio, leaveOverdub, mediaRef],
  );

  // One step of the playhead watchdog.
  const tick = useCallback(() => {
    const media = mediaRef.current;
    if (!media) return;
    const od = activeOverdub.current;
    if (od) {
      // The overdub was undone while it was playing.
      if (!editsRef.current.includes(od)) leaveOverdub(true);
    } else {
      const t = media.currentTime;
      const next = overdubAt(t, editsRef.current);
      if (next) {
        enterOverdub(next);
      } else {
        const skip = skipTarget(t, cutRanges(editsRef.current));
        if (skip !== null) {
          if (skip >= duration - 0.01) {
            media.pause();
            media.currentTime = duration;
          } else {
            media.currentTime = skip;
          }
        }
      }
    }
    sync();
  }, [duration, enterOverdub, leaveOverdub, mediaRef, sync]);

  useEffect(() => {
    if (!playing) return;
    const loop = () => {
      tick();
      frame.current = requestAnimationFrame(loop);
    };
    frame.current = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(frame.current);
  }, [playing, tick]);

  useEffect(() => {
    const media = mediaRef.current;
    if (!src || !media) return;
    const onPlay = () => {
      wantPlaying.current = true;
      setPlaying(true);
    };
    const onPause = () => {
      // Pausing to enter an overdub is not a user pause.
      if (!activeOverdub.current) setPlaying(false);
    };
    const onEnded = () => {
      wantPlaying.current = false;
      setPlaying(false);
    };
    media.addEventListener('play', onPlay);
    media.addEventListener('pause', onPause);
    media.addEventListener('ended', onEnded);
    media.addEventListener('timeupdate', sync);
    media.addEventListener('seeked', sync);
    return () => {
      media.removeEventListener('play', onPlay);
      media.removeEventListener('pause', onPause);
      media.removeEventListener('ended', onEnded);
      media.removeEventListener('timeupdate', sync);
      media.removeEventListener('seeked', sync);
      setPlaying(false);
      wantPlaying.current = false;
    };
  }, [mediaRef, src, sync]);

  // Stop everything when the source changes or the hook unmounts.
  useEffect(() => {
    return () => {
      overdubAudio.current?.pause();
      activeOverdub.current = null;
    };
  }, []);

  const toggle = useCallback(() => {
    const media = mediaRef.current;
    if (!media) return;
    if (activeOverdub.current) {
      const a = audio();
      if (a.paused) {
        wantPlaying.current = true;
        setPlaying(true);
        void a.play();
      } else {
        wantPlaying.current = false;
        setPlaying(false);
        a.pause();
      }
      return;
    }
    if (media.paused) {
      if (media.ended || media.currentTime >= duration - 0.01) media.currentTime = 0;
      void media.play();
    } else {
      wantPlaying.current = false;
      media.pause();
    }
  }, [audio, duration, mediaRef]);

  const seek = useCallback(
    (t: number) => {
      const media = mediaRef.current;
      if (!media) return;
      if (activeOverdub.current) {
        activeOverdub.current = null;
        setOverdubbing(null);
        audio().pause();
      }
      media.currentTime = Math.max(0, Math.min(t, duration));
      if (wantPlaying.current && media.paused) void media.play();
      sync();
    },
    [audio, duration, mediaRef, sync],
  );

  return { playing, currentTime, activeWord, overdubbing, toggle, seek };
}

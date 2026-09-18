// Playback that honours the edit list live: cuts are skipped as the
// playhead reaches them, and overdubs freeze the picture on their first
// frame while the synthesized audio plays through a second element.

import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';

import { cutRanges, overdubAt, skipTarget, titles, wordIndexAt } from './editlist';
import type { Edit, OverdubEdit, TitleEdit, Word } from './types';

/** The earliest title whose instant lies in (prev, now]; null when none or when moving backwards. */
export function titleCrossed(list: TitleEdit[], prev: number, now: number): TitleEdit | null {
  if (now <= prev) return null;
  let best: TitleEdit | null = null;
  for (const title of list) {
    if (title.at > prev && title.at <= now && (best === null || title.at < best.at)) best = title;
  }
  return best;
}

/**
 * The next title at instant `at` that has not been shown yet, in edit order.
 * Several titles may share an instant; they play one after another.
 */
export function nextTitleAt(
  list: TitleEdit[],
  at: number,
  shown: readonly TitleEdit[],
): TitleEdit | null {
  return list.find((title) => title.at === at && !shown.includes(title)) ?? null;
}

/**
 * The far end of the span the playhead traverses this tick: a cut skip or an
 * overdub jump carries it past `t` in the same tick, and a title inside that
 * span must still be seen.
 */
export function tickSpan(t: number, skip: number | null, overdubEnd: number | null): number {
  return Math.max(t, skip ?? t, overdubEnd ?? t);
}

export interface Playback {
  playing: boolean;
  currentTime: number;
  /** Index of the word under the playhead, or -1. */
  activeWord: number;
  /** The overdub whose audio is currently playing, if any. */
  overdubbing: OverdubEdit | null;
  /** The title card the preview is paused on, if any. */
  titling: TitleEdit | null;
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
  const [titling, setTitling] = useState<TitleEdit | null>(null);

  // Latest props for the animation-frame loop without re-subscribing.
  const editsRef = useRef(edits);
  const wordsRef = useRef(words);
  editsRef.current = edits;
  wordsRef.current = words;

  const wantPlaying = useRef(false);
  const activeOverdub = useRef<OverdubEdit | null>(null);
  const overdubAudio = useRef<HTMLAudioElement | null>(null);
  const frame = useRef(0);
  const activeTitle = useRef<TitleEdit | null>(null);
  const titleTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** The cards already played at the current instant, so the rest still follow. */
  const shownTitles = useRef<{ at: number; list: TitleEdit[] }>({ at: Number.NaN, list: [] });
  /** `enterTitle`, so `leaveTitle` can chain into the next card at the same instant. */
  const enterTitleRef = useRef<((title: TitleEdit) => void) | null>(null);
  /**
   * The playhead at the previous tick, so a title instant is crossed only once.
   * It starts behind zero so a title at 0 plays when playback first starts.
   */
  const lastTime = useRef(-1);

  const audio = useCallback(() => {
    overdubAudio.current ??= new Audio();
    return overdubAudio.current;
  }, []);

  const sync = useCallback(() => {
    const media = mediaRef.current;
    if (!media) return;
    const t = media.currentTime;
    lastTime.current = t;
    setCurrentTime(Math.round(t * 10) / 10);
    setActiveWord(wordIndexAt(t, wordsRef.current));
  }, [mediaRef]);

  const clearTitleTimer = useCallback(() => {
    if (titleTimer.current !== null) {
      clearTimeout(titleTimer.current);
      titleTimer.current = null;
    }
  }, []);

  const leaveTitle = useCallback(
    (resume: boolean) => {
      clearTitleTimer();
      const title = activeTitle.current;
      const media = mediaRef.current;
      activeTitle.current = null;
      setTitling(null);
      if (!title || !media) return;
      if (resume) {
        // Another card at the same instant plays straight after this one.
        const seen = shownTitles.current.at === title.at ? shownTitles.current.list : [];
        const next = nextTitleAt(titles(editsRef.current), title.at, seen);
        if (next) {
          enterTitleRef.current?.(next);
          return;
        }
      }
      // Park the crossing behind us so the cards do not re-trigger.
      lastTime.current = title.at;
      shownTitles.current = { at: Number.NaN, list: [] };
      if (resume && wantPlaying.current) void media.play();
    },
    [clearTitleTimer, mediaRef],
  );

  const enterTitle = useCallback(
    (title: TitleEdit) => {
      const media = mediaRef.current;
      if (!media) return;
      clearTitleTimer();
      if (shownTitles.current.at === title.at) shownTitles.current.list.push(title);
      else shownTitles.current = { at: title.at, list: [title] };
      activeTitle.current = title;
      setTitling(title);
      media.pause();
      media.currentTime = title.at;
      titleTimer.current = setTimeout(() => leaveTitle(true), title.duration * 1000);
    },
    [clearTitleTimer, leaveTitle, mediaRef],
  );

  useEffect(() => {
    enterTitleRef.current = enterTitle;
  }, [enterTitle]);

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
    const title = activeTitle.current;
    const od = activeOverdub.current;
    if (title) {
      // The title was undone while its card was showing.
      if (!editsRef.current.includes(title)) leaveTitle(true);
    } else if (od) {
      // The overdub was undone while it was playing.
      if (!editsRef.current.includes(od)) leaveOverdub(true);
    } else {
      const t = media.currentTime;
      const next = overdubAt(t, editsRef.current);
      const skip = next ? null : skipTarget(t, cutRanges(editsRef.current));
      // The jump is decided first but taken last: a title inside the cut (or
      // the overdubbed range) would otherwise be stepped over in this tick
      // and never seen, because the crossing is checked against the time
      // `sync` recorded *after* the jump.
      const after = tickSpan(t, skip, next?.end ?? null);
      const crossed = titleCrossed(titles(editsRef.current), lastTime.current, after);
      if (crossed) {
        enterTitle(crossed);
      } else if (next) {
        enterOverdub(next);
      } else if (skip !== null) {
        if (skip >= duration - 0.01) {
          media.pause();
          media.currentTime = duration;
        } else {
          media.currentTime = skip;
        }
      }
    }
    sync();
  }, [duration, enterOverdub, enterTitle, leaveOverdub, leaveTitle, mediaRef, sync]);

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
      // Pausing to enter an overdub or a title card is not a user pause.
      if (!activeOverdub.current && !activeTitle.current) setPlaying(false);
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
      // A card must not outlive the media it belongs to.
      if (titleTimer.current !== null) clearTimeout(titleTimer.current);
      titleTimer.current = null;
      activeTitle.current = null;
      setTitling(null);
      shownTitles.current = { at: Number.NaN, list: [] };
      lastTime.current = -1;
    };
  }, [mediaRef, src, sync]);

  // Stop everything when the source changes or the hook unmounts.
  useEffect(() => {
    return () => {
      overdubAudio.current?.pause();
      activeOverdub.current = null;
      activeTitle.current = null;
      shownTitles.current = { at: Number.NaN, list: [] };
      if (titleTimer.current !== null) clearTimeout(titleTimer.current);
      titleTimer.current = null;
    };
  }, []);

  const toggle = useCallback(() => {
    const media = mediaRef.current;
    if (!media) return;
    // Clicking through a title card dismisses it and plays on.
    if (activeTitle.current) {
      leaveTitle(true);
      return;
    }
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
  }, [audio, duration, leaveTitle, mediaRef]);

  const seek = useCallback(
    (t: number) => {
      const media = mediaRef.current;
      if (!media) return;
      if (activeTitle.current) leaveTitle(false);
      shownTitles.current = { at: Number.NaN, list: [] };
      if (activeOverdub.current) {
        activeOverdub.current = null;
        setOverdubbing(null);
        audio().pause();
      }
      const to = Math.max(0, Math.min(t, duration));
      media.currentTime = to;
      if (wantPlaying.current && media.paused) void media.play();
      sync();
      // Titles before the destination must not fire on the next tick.
      lastTime.current = to;
    },
    [audio, duration, leaveTitle, mediaRef, sync],
  );

  return { playing, currentTime, activeWord, overdubbing, titling, toggle, seek };
}

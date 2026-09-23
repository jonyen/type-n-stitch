// Playback that honours the edit list live: cuts are skipped as the
// playhead reaches them, and overdubs freeze the picture on their first
// frame while the synthesized audio plays through a second element.

import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';

import { EPS, overdubAt, owner, titles, wordIndexAt } from './editlist';
import { outputToSource, pieceOutputTime, timelineLength, type Segment } from './timeline';
import type { Edit, OverdubEdit, Range, TitleEdit, Word } from './types';

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

/** Where play should begin: the first output piece when the media sits at its start or end (or has ended); null to continue from `t`. */
export function restartAt(
  t: number,
  ended: boolean,
  duration: number,
  ordered: Range[],
): number | null {
  const first = ordered[0]?.start ?? 0;
  if (ended || t <= 0 || t >= duration - 0.01) return first;
  return null;
}

/** Index of the output piece containing `t`, else the next piece at or after `t`, else the last piece. 0 when `ordered` is empty. */
export function pieceIndexAt(t: number, ordered: Range[]): number {
  if (ordered.length === 0) return 0;
  const inside = ordered.findIndex((p) => t >= p.start && t < p.end);
  if (inside !== -1) return inside;
  let best = -1;
  ordered.forEach((p, i) => {
    if (p.start >= t && (best === -1 || p.start < (ordered[best] as Range).start)) best = i;
  });
  return best === -1 ? ordered.length - 1 : best;
}

export type PlayStep =
  { kind: 'play' } | { kind: 'seek'; to: number; index: number } | { kind: 'stop' };

/**
 * What the playhead does next, tracking the output piece it is on (`index`)
 * rather than re-deriving it from source time alone: a source-time playhead
 * cannot tell which output piece it is playing when a piece's source range
 * overlaps another piece earlier in the same order (a reversed pair, say),
 * so leaving a piece always advances to the *next output piece*, never back
 * to whichever piece happens to contain the current source time.
 */
export function playStep(t: number, index: number, ordered: Range[]): PlayStep {
  if (ordered.length === 0) return { kind: 'play' };
  const piece = ordered[index];
  if (piece && t < piece.end - EPS) return { kind: 'play' };
  const next = ordered[index + 1];
  if (next) return { kind: 'seek', to: next.start, index: index + 1 };
  return { kind: 'stop' };
}

/**
 * What playback drives: a media element, or a `StitchedMedia` clock over
 * several files. Every time is in stitched seconds.
 */
export interface PlaybackMedia {
  currentTime: number;
  readonly paused: boolean;
  readonly ended: boolean;
  play(): Promise<void>;
  pause(): void;
  addEventListener(type: string, listener: () => void): void;
  removeEventListener(type: string, listener: () => void): void;
}

export interface Playback {
  playing: boolean;
  /** The playhead in source seconds, to 0.1 s (exact once stopped at the end). */
  currentTime: number;
  /**
   * The playhead in output seconds, to 0.1 s: mapped through the output piece
   * that is playing, so it is right where a reorder makes a source instant
   * ambiguous. The output length once stopped at the end.
   */
  outputTime: number;
  /** Stopped at the end of the output: play starts over from the first piece. */
  atEnd: boolean;
  /** Index of the word under the playhead, or -1. */
  activeWord: number;
  /** The overdub whose audio is currently playing, if any. */
  overdubbing: OverdubEdit | null;
  /** The title card the preview is paused on, if any. */
  titling: TitleEdit | null;
  toggle: () => void;
  /** Seek to a source time. */
  seek: (t: number) => void;
  /** Seek to an output time; the output length parks at the end. */
  seekOutput: (t: number) => void;
}

export function usePlayback(
  mediaRef: RefObject<PlaybackMedia | null>,
  words: Word[],
  edits: Edit[],
  duration: number,
  /** Media URL. The element mounts after the hook, so listeners re-attach when it changes. */
  src: string | undefined,
  ordered: Range[],
  /** The edit in output time, laid out from the same `ordered` pieces. */
  segments: Segment[],
): Playback {
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [outputTime, setOutputTime] = useState(0);
  const [atEnd, setAtEnd] = useState(false);
  const [activeWord, setActiveWord] = useState(-1);
  const [overdubbing, setOverdubbing] = useState<OverdubEdit | null>(null);
  const [titling, setTitling] = useState<TitleEdit | null>(null);

  // Latest props for the animation-frame loop without re-subscribing.
  const editsRef = useRef(edits);
  const wordsRef = useRef(words);
  const orderedRef = useRef(ordered);
  const segmentsRef = useRef(segments);
  editsRef.current = edits;
  wordsRef.current = words;
  orderedRef.current = ordered;
  segmentsRef.current = segments;

  const wantPlaying = useRef(false);
  /** The output piece currently playing, by index into `ordered`. */
  const playIndex = useRef(0);
  /**
   * Stopped at the end of the output. The media then sits at the source end,
   * which is not where the output ends once pieces are reordered, so the end
   * is a state of its own rather than a source time.
   */
  const ended = useRef(false);
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
    const end = ended.current;
    // Both are rounded to keep renders to a few a second, the output time only
    // after mapping: a rounded source time can land in a different piece.
    setCurrentTime(end ? t : Math.round(t * 10) / 10);
    setOutputTime(
      end
        ? timelineLength(segmentsRef.current)
        : Math.round(
            pieceOutputTime(t, playIndex.current, orderedRef.current, segmentsRef.current) * 10,
          ) / 10,
    );
    setAtEnd(end);
    setActiveWord(wordIndexAt(t, wordsRef.current));
  }, [mediaRef]);

  // `ordered` changing (a split, cut, reorder or undo, possibly a peer's,
  // mid-playback) can leave `playIndex` pointing at a piece that no longer
  // occupies that slot; resync it from where the media actually sits rather
  // than pausing or seeking, so an edit never interrupts the person watching.
  // The output time moves with the layout, so it is re-read too.
  useEffect(() => {
    const media = mediaRef.current;
    if (!media) return;
    if (!ended.current) playIndex.current = pieceIndexAt(media.currentTime, ordered);
    sync();
  }, [mediaRef, ordered, segments, sync]);

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
      const step = next ? null : playStep(t, playIndex.current, orderedRef.current);
      const skipTo = step?.kind === 'seek' ? step.to : null;
      // The jump is decided first but taken last: a title inside the cut (or
      // the overdubbed range) would otherwise be stepped over in this tick
      // and never seen, because the crossing is checked against the time
      // `sync` recorded *after* the jump.
      const after = tickSpan(t, skipTo, next?.end ?? null);
      const crossed = titleCrossed(titles(editsRef.current), lastTime.current, after);
      if (crossed) {
        enterTitle(crossed);
      } else if (next) {
        enterOverdub(next);
      } else if (step?.kind === 'seek') {
        media.currentTime = step.to;
        playIndex.current = step.index;
      } else if (step?.kind === 'stop') {
        wantPlaying.current = false;
        ended.current = true;
        media.pause();
        media.currentTime = duration;
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
      ended.current = false;
      setPlaying(true);
    };
    const onPause = () => {
      // Pausing to enter an overdub or a title card is not a user pause.
      if (!activeOverdub.current && !activeTitle.current) setPlaying(false);
    };
    const onEnded = () => {
      const step = playStep(duration, playIndex.current, orderedRef.current);
      if (step.kind === 'seek') {
        media.currentTime = step.to;
        playIndex.current = step.index;
        void media.play();
        return;
      }
      wantPlaying.current = false;
      ended.current = true;
      setPlaying(false);
      sync();
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
  }, [duration, mediaRef, src, sync]);

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
      const at = ended.current
        ? (orderedRef.current[0]?.start ?? 0)
        : restartAt(media.currentTime, media.ended, duration, orderedRef.current);
      const startAt = at ?? media.currentTime;
      playIndex.current = pieceIndexAt(startAt, orderedRef.current);
      ended.current = false;
      if (at !== null) media.currentTime = at;
      void media.play();
      sync();
    } else {
      wantPlaying.current = false;
      media.pause();
    }
  }, [audio, duration, leaveTitle, mediaRef, sync]);

  /** Seek to source time `t`, played as output piece `index` (by default, the one holding `t`). */
  const seekTo = useCallback(
    (t: number, index?: number) => {
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
      playIndex.current = index ?? pieceIndexAt(to, orderedRef.current);
      ended.current = false;
      media.currentTime = to;
      if (wantPlaying.current && media.paused) void media.play();
      sync();
      // Titles before the destination must not fire on the next tick.
      lastTime.current = to;
    },
    [audio, duration, leaveTitle, mediaRef, sync],
  );

  const seek = useCallback((t: number) => seekTo(t), [seekTo]);

  const seekOutput = useCallback(
    (t: number) => {
      const media = mediaRef.current;
      if (!media) return;
      const segs = segmentsRef.current;
      const list = orderedRef.current;
      if (list.length > 0 && t >= timelineLength(segs) - EPS) {
        // The end of the output: park there, as playing to the end does. Its
        // source instant would read back as the start of another piece.
        wantPlaying.current = false;
        media.pause();
        seekTo(duration, list.length - 1);
        ended.current = true;
        sync();
        return;
      }
      const seg = segs.find((s) => t >= s.output.start && t < s.output.end);
      const index = seg ? Math.min(owner(list, seg.source.start), list.length - 1) : undefined;
      seekTo(outputToSource(t, segs), index);
    },
    [duration, mediaRef, seekTo, sync],
  );

  return {
    playing,
    currentTime,
    outputTime,
    atEnd,
    activeWord,
    overdubbing,
    titling,
    toggle,
    seek,
    seekOutput,
  };
}

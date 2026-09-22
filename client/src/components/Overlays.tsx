import { useEffect, useRef, useState } from 'react';

import { assetTime, audiosAt, brollAt, DUCK, gainToLinear, speaking } from '../overlays';
import type { Asset, AudioEdit, Edit, Word } from '../types';
import type { Playback } from '../usePlayback';
import styles from './Overlays.module.css';

interface Props {
  edits: Edit[];
  assets: Asset[];
  words: Word[];
  playback: Playback;
}

/** A muted second video over the frame while a B-roll range plays, and one audio element per music edit under the playhead. */
export function Overlays({ edits, assets, words, playback }: Props) {
  const t = playback.currentTime;
  const url = (id: string) => assets.find((a) => a.id === id)?.url;
  const broll = brollAt(t, edits);
  const beds = audiosAt(t, edits);
  return (
    <>
      {broll && (
        <BrollVideo
          key={broll.start}
          src={url(broll.media)}
          edit={broll}
          t={t}
          playing={playback.playing}
        />
      )}
      {beds.map((a) => (
        <AudioBed
          key={a.start}
          src={url(a.media)}
          edit={a}
          t={t}
          playing={playback.playing}
          ducked={a.duck && speaking(t, words)}
        />
      ))}
    </>
  );
}

function BrollVideo({
  src,
  edit,
  t,
  playing,
}: {
  src: string | undefined;
  edit: { start: number; offset: number };
  t: number;
  playing: boolean;
}) {
  const ref = useRef<HTMLVideoElement>(null);
  const [missing, setMissing] = useState(src === undefined);
  useEffect(() => {
    // Clears the placeholder once `listAssets` resolves and the file shows up.
    setMissing(src === undefined);
  }, [src]);
  useEffect(() => {
    const v = ref.current;
    if (!v) return;
    const want = assetTime(edit, t);
    if (Math.abs(v.currentTime - want) > 0.3) v.currentTime = want;
    if (playing && v.paused)
      void v.play().catch(() => {
        // Autoplay can be blocked; the muted overlay just stays paused.
      });
    if (!playing && !v.paused) v.pause();
  }, [edit, t, playing]);
  // Explicit stop when this overlay unmounts (the B-roll range ended, or the
  // whole player did), rather than relying on removal from the document to
  // pause it. A plain cleanup on the sync effect above would fire every
  // tick (it depends on `t`), so this is its own mount-only effect.
  useEffect(
    () => () => {
      ref.current?.pause();
    },
    [],
  );
  if (missing) return <div className={styles.missing}>B-roll file missing</div>;
  return (
    <video
      ref={ref}
      className={styles.broll}
      src={src}
      muted
      playsInline
      onError={() => setMissing(true)}
    />
  );
}

function AudioBed({
  src,
  edit,
  t,
  playing,
  ducked,
}: {
  src: string | undefined;
  edit: AudioEdit;
  t: number;
  playing: boolean;
  ducked: boolean;
}) {
  const ref = useRef<HTMLAudioElement>(null);
  useEffect(() => {
    const a = ref.current;
    if (!a) return;
    const want = assetTime(edit, t);
    if (Math.abs(a.currentTime - want) > 0.3) a.currentTime = want;
    // `HTMLMediaElement.volume` tops out at 1, so a positive gain cannot be
    // previewed: the clamp keeps the value legal. The export honours the full
    // −30..+12 dB range, and `AudioDialog` says so when the gain is a boost.
    a.volume = Math.min(1, gainToLinear(edit.gain) * (ducked ? DUCK : 1));
    if (playing && a.paused)
      void a.play().catch(() => {
        // Autoplay can be blocked; the bed just stays paused.
      });
    if (!playing && !a.paused) a.pause();
  }, [edit, t, playing, ducked]);
  // Explicit stop when this overlay unmounts, rather than relying on removal
  // from the document to pause it; see the matching note in `BrollVideo`.
  useEffect(
    () => () => {
      ref.current?.pause();
    },
    [],
  );
  if (!src) return null;
  return <audio ref={ref} src={src} preload="auto" />;
}

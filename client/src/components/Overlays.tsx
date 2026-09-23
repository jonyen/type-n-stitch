import { useEffect, useRef, useState } from 'react';

import { cx } from '../cx';
import {
  assetTime,
  audiosAt,
  DUCK,
  gainToLinear,
  layersAt,
  layerVolume,
  mediaUrl,
  pipPlacement,
  speaking,
} from '../overlays';
import type { Asset, AudioEdit, Edit, LayerEdit, SourceView, Word } from '../types';
import type { Playback } from '../usePlayback';
import styles from './Overlays.module.css';

interface Props {
  edits: Edit[];
  assets: Asset[];
  /** The project's own videos: a layer can show a stretch of one of them. */
  sources: SourceView[];
  words: Word[];
  playback: Playback;
}

/**
 * The layer stack and the music under the playhead. One video per visible
 * layer, lower tracks first: siblings paint in DOM order, so V3 covers V2
 * covers the main picture, and captions and title cards (after this in the
 * Player) stay on top. One audio element per music edit.
 */
export function Overlays({ edits, assets, sources, words, playback }: Props) {
  const t = playback.currentTime;
  const url = (id: string) => mediaUrl(id, assets, sources);
  const talking = speaking(t, words);
  // Like the export, a layer's picture and sound are both absent over a title card.
  const shown = playback.titling ? [] : layersAt(t, edits);
  return (
    <>
      {shown.map((l) => (
        <LayerVideo
          key={`${l.track}:${l.start}`}
          src={url(l.media)}
          edit={l}
          t={t}
          playing={playback.playing}
          ducked={talking}
        />
      ))}
      {audiosAt(t, edits).map((a) => (
        <AudioBed
          key={a.start}
          src={url(a.media)}
          edit={a}
          t={t}
          playing={playback.playing}
          ducked={a.duck && talking}
        />
      ))}
    </>
  );
}

function LayerVideo({
  src,
  edit,
  t,
  playing,
  ducked,
}: {
  src: string | undefined;
  edit: LayerEdit;
  t: number;
  playing: boolean;
  ducked: boolean;
}) {
  const ref = useRef<HTMLVideoElement>(null);
  const [missing, setMissing] = useState(src === undefined);
  useEffect(() => {
    // Clears the placeholder once `listAssets` resolves and the file shows up.
    setMissing(src === undefined);
  }, [src]);
  const volume = layerVolume(edit.audio);
  useEffect(() => {
    const v = ref.current;
    if (!v) return;
    const want = assetTime(edit, t);
    if (Math.abs(v.currentTime - want) > 0.3) v.currentTime = want;
    // Muted unless the layer's sound is on; then its level, ducked under
    // speech like music, as the export mixes it.
    v.muted = volume === null;
    if (volume !== null) v.volume = volume * (ducked ? DUCK : 1);
    if (playing && v.paused)
      void v.play().catch(() => {
        // Autoplay can be blocked; the layer just stays paused.
      });
    if (!playing && !v.paused) v.pause();
  }, [edit, t, playing, volume, ducked]);
  // Explicit stop when this layer unmounts (its range ended, or the whole
  // player did), rather than relying on removal from the document to pause
  // it. A cleanup on the sync effect above would fire every tick (it depends
  // on `t`), so this is its own mount-only effect.
  useEffect(
    () => () => {
      ref.current?.pause();
    },
    [],
  );
  const place = pipPlacement(edit.frame);
  return (
    <div
      className={cx(styles.layer, place && styles.pip)}
      style={place ?? undefined}
      data-layer={`${edit.track}:${edit.start}`}
      data-frame={edit.frame}
    >
      {missing ? (
        <span className={styles.missing}>Layer file missing</span>
      ) : (
        <video
          ref={ref}
          className={styles.video}
          src={src}
          muted={volume === null}
          playsInline
          preload="auto"
          onError={() => setMissing(true)}
        />
      )}
    </div>
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
  // from the document to pause it; see the matching note in `LayerVideo`.
  useEffect(
    () => () => {
      ref.current?.pause();
    },
    [],
  );
  if (!src) return null;
  return <audio ref={ref} src={src} preload="auto" />;
}

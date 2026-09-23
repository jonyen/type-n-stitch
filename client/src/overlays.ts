// Overlay helpers for layers (V2/V3), background audio, gain, and the
// preview's speech envelope. Mirrors the engine's overlay handling for the client.

import type {
  Asset,
  AudioEdit,
  Edit,
  Frame,
  LayerEdit,
  LayerTrack,
  SourceView,
  Word,
} from './types';

/** A stacked video track (2 or 3). V1 is the main track and never holds a layer. */
export type { LayerTrack };

/** How much a ducked track attenuates under speech, as a linear factor. */
export const DUCK = 0.25;

/** Layer edits on one track, or on every track when `track` is omitted. */
export function layers(edits: Edit[], track?: LayerTrack): LayerEdit[] {
  return edits.filter(
    (e): e is LayerEdit => e.kind === 'layer' && (track === undefined || e.track === track),
  );
}

export function audios(edits: Edit[]): AudioEdit[] {
  return edits.filter((e): e is AudioEdit => e.kind === 'audio');
}

/** The background audio clips covering source time `t`. */
export function audiosAt(t: number, edits: Edit[]): AudioEdit[] {
  return audios(edits).filter((a) => t >= a.start && t < a.end);
}

/** Seconds into the asset at source time `t`. */
export function assetTime(edit: { start: number; offset: number }, t: number): number {
  return edit.offset + Math.max(0, t - edit.start);
}

export function gainToLinear(db: number): number {
  return Math.pow(10, db / 20);
}

/** True while a word is under the playhead (the preview's speech envelope). */
export function speaking(t: number, words: Word[]): boolean {
  return words.some((w) => t >= w.start && t < w.end);
}

/** The layers covering source time `t`, lower tracks first: the order they paint in. */
export function layersAt(t: number, edits: Edit[]): LayerEdit[] {
  return layers(edits)
    .filter((l) => t >= l.start && t < l.end)
    .sort((a, b) => a.track - b.track);
}

export const FRAMES: readonly Frame[] = [
  'full',
  'pipTopLeft',
  'pipTopRight',
  'pipBottomLeft',
  'pipBottomRight',
];

const ARROW: Record<Exclude<Frame, 'full'>, string> = {
  pipTopLeft: '↖',
  pipTopRight: '↗',
  pipBottomLeft: '↙',
  pipBottomRight: '↘',
};

export function frameLabel(frame: Frame): string {
  return frame === 'full' ? 'Full' : `PiP ${ARROW[frame]}`;
}

/** "V2 · clip.mp4", plus "· PiP ↗" for a picture-in-picture. */
export function layerTag(layer: LayerEdit, name: string): string {
  const parts = [`V${layer.track}`, name];
  if (layer.frame !== 'full') parts.push(frameLabel(layer.frame));
  return parts.join(' · ');
}

/**
 * A layer's preview volume: null when its sound is off (the element stays
 * muted), else its level as a linear factor. `HTMLMediaElement.volume` tops
 * out at 1, so a boost is capped here; the export applies the full level.
 */
export function layerVolume(audio: number | null): number | null {
  return audio === null ? null : Math.min(1, gainToLinear(audio));
}

/** Where a picture-in-picture sits in the frame, as CSS lengths. */
export interface PipPlacement {
  width: string;
  top?: string;
  bottom?: string;
  left?: string;
  right?: string;
}

/**
 * A picture-in-picture is 30% of the frame's width, 4% in from its corner
 * (4% of the width from the side, 4% of the height from the top or bottom).
 * The export uses the same numbers. Null for a full-frame layer.
 */
export function pipPlacement(frame: Frame): PipPlacement | null {
  if (frame === 'full') return null;
  const place: PipPlacement = { width: '30%' };
  if (frame === 'pipTopLeft' || frame === 'pipTopRight') place.top = '4%';
  else place.bottom = '4%';
  if (frame === 'pipTopLeft' || frame === 'pipBottomLeft') place.left = '4%';
  else place.right = '4%';
  return place;
}

/** A layer's or music bed's file name: an uploaded asset, else one of the project's own videos. */
export function mediaName(id: string, assets: Asset[], sources: SourceView[]): string | undefined {
  return assets.find((a) => a.id === id)?.name ?? sources.find((s) => s.mediaId === id)?.filename;
}

export function mediaUrl(id: string, assets: Asset[], sources: SourceView[]): string | undefined {
  return assets.find((a) => a.id === id)?.url ?? sources.find((s) => s.mediaId === id)?.url;
}

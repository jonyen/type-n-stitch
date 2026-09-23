// JSON shapes shared with the Rust engine (engine/src/types.rs). Times are
// seconds in the source media; ranges are half-open [start, end).

export interface Word {
  id: string;
  text: string;
  start: number;
  end: number;
}

export interface Range {
  start: number;
  end: number;
}

/** How two output pieces meet. `crossfade` is reserved: the server rejects it. */
export type Transition = 'none' | 'dip' | 'crossfade';

export type TitleStyle = 'dark' | 'light' | 'accent';

export type CaptionPos = 'bottomLeft' | 'bottomCenter' | 'topLeft';

export interface CutEdit extends Range {
  kind: 'cut';
  /** Overrides the project transition where this cut joins its neighbours. */
  transition?: Transition;
}

export interface OverdubEdit extends Range {
  kind: 'overdub';
  text: string;
  audioUrl: string;
  audioDuration: number;
}

/** A card inserted at source instant `at`; the output grows by `duration`. */
export interface TitleEdit {
  kind: 'title';
  at: number;
  duration: number;
  text: string;
  subtitle: string | null;
  style: TitleStyle;
}

/** Text drawn over the picture for `[start, end)`; the output length is unchanged. */
export interface CaptionEdit extends Range {
  kind: 'caption';
  text: string;
  position: CaptionPos;
}

/** A video on the main track, placed at a fixed offset on the stitched timeline. */
export interface Source {
  media: string;
  offset: number;
  duration: number;
}

/** Where a layer's picture sits: full frame, or a corner picture-in-picture. */
export type Frame = 'full' | 'pipTopLeft' | 'pipTopRight' | 'pipBottomLeft' | 'pipBottomRight';

/** The video tracks above the main one: V2 and V3. */
export type LayerTrack = 2 | 3;

/**
 * `media` from `offset` on track `track` over main-track `[start, end)`,
 * placed by `frame`. `audio` is null when muted, else the layer's level in dB.
 */
export interface LayerEdit extends Range {
  kind: 'layer';
  track: LayerTrack;
  media: string;
  offset: number;
  frame: Frame;
  audio: number | null;
}

export interface AudioEdit extends Range {
  kind: 'audio';
  media: string;
  offset: number;
  gain: number;
  duck: boolean;
}

export type Edit = CutEdit | OverdubEdit | TitleEdit | CaptionEdit | LayerEdit | AudioEdit;

export type MediaKind = 'audio' | 'video';

export interface Media {
  id: string;
  filename: string;
  ext: string;
  duration: number;
  kind: MediaKind;
  url: string;
}

export type TranscriptStatus = 'pending' | 'running' | 'ready' | 'error';

/**
 * One file on the main track as GET /api/projects/:id lists it, in stitched
 * order; index 0 is the project's own media.
 */
export interface SourceView {
  index: number;
  mediaId: string;
  url: string;
  filename: string;
  kind: MediaKind;
  offset: number;
  duration: number;
  transcript: TranscriptStatus;
}

/** A starter clip from samples/library.json (GET /api/library). */
export interface LibraryItem {
  slug: string;
  title: string;
  blurb: string;
  author: string;
  sourceTitle: string;
  sourceUrl: string | null;
  kind: MediaKind;
  duration: number | null;
  poster: string | null;
  available: boolean;
}

export interface User {
  id: string;
  email: string;
  displayName: string;
  color: string;
  bot?: boolean;
}

/** An API token minted from the "Connect an agent" dialog (POST /api/tokens). */
export interface TokenInfo {
  id: string;
  label: string;
  createdAt: number;
  lastUsedAt: number | null;
}

export type Role = 'owner' | 'editor' | 'commenter' | 'viewer';

export interface ProjectSummary {
  id: string;
  title: string;
  role: Role;
  media: Media;
  /** Every file on the main track. Sent by GET /api/projects/:id; absent in the list. */
  sources?: SourceView[];
  createdAt: number;
}

/** A project's uploaded layer or music file (GET/POST/DELETE /api/projects/:id/assets). */
export interface Asset {
  id: string;
  kind: MediaKind;
  name: string;
  ext: string;
  duration: number;
  width: number | null;
  height: number | null;
  createdAt: number;
  url: string;
  poster: string | null;
}

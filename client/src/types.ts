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

export interface CutEdit extends Range {
  kind: 'cut';
}

export interface OverdubEdit extends Range {
  kind: 'overdub';
  text: string;
  audioUrl: string;
  audioDuration: number;
}

export type Edit = CutEdit | OverdubEdit;

export type MediaKind = 'audio' | 'video';

export interface Media {
  id: string;
  filename: string;
  ext: string;
  duration: number;
  kind: MediaKind;
  url: string;
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
}

export type Role = 'owner' | 'editor' | 'commenter' | 'viewer';

export interface ProjectSummary {
  id: string;
  title: string;
  role: Role;
  media: Media;
  createdAt: number;
}

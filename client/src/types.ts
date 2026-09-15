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

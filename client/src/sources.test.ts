import { describe, expect, it } from 'vitest';

import {
  isTranscribing,
  playableSources,
  readyKey,
  sourceViewsOf,
  unlistedSources,
} from './sources';
import type { Media, ProjectSummary, Source, SourceView } from './types';

const media: Media = {
  id: 'm0',
  filename: 'take1.mp4',
  ext: 'mp4',
  duration: 10,
  kind: 'video',
  url: '/data/m0/source.mp4',
};
const summary: ProjectSummary = { id: 'p', title: 'take1', role: 'owner', media, createdAt: 0 };

const view = (
  index: number,
  mediaId: string,
  offset: number,
  duration: number,
  transcript: SourceView['transcript'] = 'ready',
): SourceView => ({
  index,
  mediaId,
  url: `/data/${mediaId}/source.mp4`,
  filename: `${mediaId}.mp4`,
  kind: 'video',
  offset,
  duration,
  transcript,
});

describe('sourceViewsOf', () => {
  it('builds the one source from `media` for a server that does not list sources', () => {
    expect(sourceViewsOf(summary)).toEqual([
      {
        index: 0,
        mediaId: 'm0',
        url: '/data/m0/source.mp4',
        filename: 'take1.mp4',
        kind: 'video',
        offset: 0,
        duration: 10,
        transcript: 'ready',
      },
    ]);
  });

  it("uses the server's list when there is one", () => {
    const sources = [view(0, 'm0', 0, 10), view(1, 'm1', 10, 5, 'running')];
    expect(sourceViewsOf({ ...summary, sources })).toBe(sources);
  });
});

describe('playableSources', () => {
  const views = [view(0, 'm0', 0, 10), view(1, 'm1', 10, 5, 'running'), view(2, 'm2', 15, 4)];
  const fold: Source[] = [
    { media: 'm0', offset: 0, duration: 10 },
    { media: 'm1', offset: 10, duration: 5 },
  ];

  it("lays the fold's sources over the listed files; an undone source is not played", () => {
    expect(playableSources(fold, views).map((v) => [v.index, v.mediaId, v.transcript])).toEqual([
      [0, 'm0', 'ready'],
      [1, 'm1', 'running'],
    ]);
  });

  it('leaves out, and reports, a source the list does not know yet', () => {
    const more = [...fold, { media: 'm9', offset: 15, duration: 1 }];
    expect(playableSources(more, views)).toHaveLength(2);
    expect(unlistedSources(more, views)).toEqual(['m9']);
    expect(unlistedSources(fold, views)).toEqual([]);
  });

  it('knows which transcripts are still coming', () => {
    expect(isTranscribing(view(1, 'm1', 0, 1, 'pending'))).toBe(true);
    expect(isTranscribing(view(1, 'm1', 0, 1, 'running'))).toBe(true);
    expect(isTranscribing(view(1, 'm1', 0, 1, 'ready'))).toBe(false);
    expect(isTranscribing(view(1, 'm1', 0, 1, 'error'))).toBe(false);
  });
});

describe('readyKey (which ready files the current words cover)', () => {
  const s = (media: string, offset: number, duration: number): Source => ({
    media,
    offset,
    duration,
  });

  it('follows undo, then a new file, then its transcript arriving', () => {
    const v0 = view(0, 'm0', 0, 10);
    const v1 = view(1, 'm1', 10, 5);
    // Loaded with two ready files: the words cover both.
    let covered = readyKey([v0, v1]);
    expect(readyKey([v0, v1], [s('m0', 0, 10), s('m1', 10, 5)])).toBe(covered);
    // Undo the second: the fold drops it before the list does. The words are stale.
    expect(readyKey([v0, v1], [s('m0', 0, 10)])).not.toBe(covered);
    // Refetched: the reply lists only the first file.
    covered = readyKey([v0]);
    expect(readyKey([v0], [s('m0', 0, 10)])).toBe(covered);
    // A new file lands and starts transcribing: nothing to fetch yet.
    const v2 = view(1, 'm2', 10, 3, 'running');
    expect(readyKey([v0, v2], [s('m0', 0, 10), s('m2', 10, 3)])).toBe(covered);
    // Its transcript is ready: the count is back where it was before the undo, but the set differs.
    const ready = { ...v2, transcript: 'ready' as const };
    expect(readyKey([v0, ready], [s('m0', 0, 10), s('m2', 10, 3)])).not.toBe(covered);
  });
});

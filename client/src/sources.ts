// The project's files as the server lists them, joined onto the fold's
// sources. The fold decides what is on the timeline (an undo can drop a
// source); the list, which can be a poll behind, supplies names, urls and
// transcript status.

import type { ProjectSummary, Source, SourceView } from './types';

/** The listed sources, or one built from `media` for a server that predates sources. */
export function sourceViewsOf(project: ProjectSummary): SourceView[] {
  if (project.sources && project.sources.length > 0) return project.sources;
  const m = project.media;
  return [
    {
      index: 0,
      mediaId: m.id,
      url: m.url,
      filename: m.filename,
      kind: m.kind,
      offset: 0,
      duration: m.duration,
      transcript: 'ready',
    },
  ];
}

/**
 * The fold's sources with their file details, in stitched order. A source the
 * list does not know yet (a peer's, just added) is left out until refetched.
 */
export function playableSources(sources: Source[], views: SourceView[]): SourceView[] {
  const out: SourceView[] = [];
  sources.forEach((s, index) => {
    const listed = views.find((v) => v.mediaId === s.media);
    if (listed) out.push({ ...listed, index, offset: s.offset, duration: s.duration });
  });
  return out;
}

/** Media ids on the timeline that the list does not name yet. */
export function unlistedSources(sources: Source[], views: SourceView[]): string[] {
  return sources.map((s) => s.media).filter((id) => !views.some((v) => v.mediaId === id));
}

export function isTranscribing(view: SourceView): boolean {
  return view.transcript === 'pending' || view.transcript === 'running';
}

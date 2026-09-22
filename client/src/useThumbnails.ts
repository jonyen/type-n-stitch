import { useEffect, useState } from 'react';

import { fetchThumbnails, type Thumbnails } from './api';
import type { MediaKind } from './types';

/** Sprite sheet for the scrubber preview; null for audio or until it loads. */
export function useThumbnails(
  projectId: string | null,
  kind: MediaKind | undefined,
): Thumbnails | null {
  const [thumbs, setThumbs] = useState<Thumbnails | null>(null);
  useEffect(() => {
    setThumbs(null);
    if (!projectId || kind !== 'video') return;
    let cancelled = false;
    fetchThumbnails(projectId)
      .then((t) => {
        if (!cancelled) setThumbs(t);
      })
      .catch(() => {
        // No preview frames; the time label still shows.
      });
    return () => {
      cancelled = true;
    };
  }, [projectId, kind]);
  return thumbs;
}

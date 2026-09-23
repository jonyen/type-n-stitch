// Importing several files as one project: order them, then upload one at a
// time. The first upload creates the project; the rest are appended to its
// main track. A file that fails is marked with the server's reason and the
// others carry on.

import type { ProjectSummary, SourceView } from './types';

/** What the pickers accept; the server probes and rejects anything unreadable. */
export const MEDIA_ACCEPT = '.mp3,.wav,.m4a,.mp4,.mov,audio/*,video/*';

export type ImportStatus = 'queued' | 'uploading' | 'done' | 'error';

export interface ImportItem {
  id: string;
  name: string;
  status: ImportStatus;
  /** Fraction of the file uploaded, 0 to 1. */
  progress: number;
  error: string | null;
}

export interface ImportDeps {
  /** Make a new project from `file`. Absent when appending to an open project. */
  create?: (file: File, onProgress: (fraction: number) => void) => Promise<ProjectSummary>;
  append: (
    projectId: string,
    file: File,
    onProgress: (fraction: number) => void,
  ) => Promise<SourceView>;
}

/** By file name, with natural numbering (take2 before take10), ignoring case. */
export function byName(files: File[]): File[] {
  return [...files].sort((a, b) =>
    a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }),
  );
}

/** `list` with the item at `from` moved to `to`; unchanged when either is out of range. */
export function moveItem<T>(list: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length)
    return [...list];
  const next = [...list];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item as T);
  return next;
}

export function itemsFor(files: File[]): ImportItem[] {
  return files.map((f, i) => ({
    id: `${i}-${f.name}`,
    name: f.name,
    status: 'queued',
    progress: 0,
    error: null,
  }));
}

/**
 * Upload `files` in order. With no `projectId`, the first file that uploads
 * creates the project. `onChange` gets a new list on every status or progress
 * change.
 */
export async function runImport(
  files: File[],
  deps: ImportDeps,
  onChange: (items: ImportItem[]) => void,
  projectId: string | null = null,
): Promise<{ projectId: string | null; project: ProjectSummary | null; items: ImportItem[] }> {
  let items = itemsFor(files);
  const set = (i: number, patch: Partial<ImportItem>) => {
    items = items.map((item, k) => (k === i ? { ...item, ...patch } : item));
    onChange(items);
  };
  onChange(items);
  let id = projectId;
  let project: ProjectSummary | null = null;
  for (const [i, file] of files.entries()) {
    set(i, { status: 'uploading', progress: 0 });
    const onProgress = (fraction: number) => set(i, { progress: fraction });
    try {
      if (id === null) {
        if (!deps.create) throw new Error('no project to add this file to');
        project = await deps.create(file, onProgress);
        id = project.id;
      } else {
        await deps.append(id, file, onProgress);
      }
      set(i, { status: 'done', progress: 1 });
    } catch (err) {
      set(i, { status: 'error', error: err instanceof Error ? err.message : String(err) });
    }
  }
  return { projectId: id, project, items };
}

/** One line naming every failed file and why, or null when none failed. */
export function importFailures(items: ImportItem[]): string | null {
  const failed = items.filter((i) => i.status === 'error');
  if (failed.length === 0) return null;
  return `Could not import ${failed.map((i) => `${i.name} (${i.error ?? 'failed'})`).join('; ')}`;
}

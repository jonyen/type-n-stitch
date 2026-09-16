// Thin fetch wrappers over the Rust server. Errors carry the server's message.

import type { CutEdit, Edit, LibraryItem, Media, Word } from './types';

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(url, init);
  } catch {
    throw new ApiError('Could not reach the server. Is `cargo run -p server` running?', 0);
  }
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    const message =
      body && typeof body === 'object' && 'error' in body && typeof body.error === 'string'
        ? body.error
        : `${response.status} ${response.statusText}`;
    throw new ApiError(message, response.status);
  }
  return body as T;
}

function postJson<T>(url: string, payload: unknown): Promise<T> {
  return request<T>(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
  });
}

export function uploadMedia(file: File): Promise<Media> {
  const form = new FormData();
  form.append('file', file, file.name);
  return request<Media>('/api/media', { method: 'POST', body: form });
}

export function listLibrary(): Promise<LibraryItem[]> {
  return request<LibraryItem[]>('/api/library');
}

export function openLibraryClip(slug: string): Promise<Media> {
  return request<Media>(`/api/library/${encodeURIComponent(slug)}`, { method: 'POST' });
}

export async function transcribeMedia(id: string): Promise<Word[]> {
  const { words } = await request<{ words: Word[] }>(`/api/media/${id}/transcribe`, {
    method: 'POST',
  });
  return words;
}

export interface Suggestions {
  fillers: CutEdit[];
  pauses: CutEdit[];
}

export function suggestEdits(id: string, twoWordFillers: boolean): Promise<Suggestions> {
  return postJson(`/api/media/${id}/suggest`, { twoWordFillers });
}

export function synthesizeOverdub(
  id: string,
  text: string,
): Promise<{ audioUrl: string; duration: number }> {
  return postJson(`/api/media/${id}/overdub`, { text });
}

export interface ExportStarted {
  jobId: string;
  planned: number;
}

export type ExportJob =
  | { status: 'running'; progress: number }
  | { status: 'done'; progress: number; url: string; duration: number; bytes: number }
  | { status: 'error'; message: string };

export function exportMedia(id: string, edits: Edit[]): Promise<ExportStarted> {
  return postJson(`/api/media/${id}/export`, { edits });
}

export function exportProgress(id: string, jobId: string): Promise<ExportJob> {
  return request<ExportJob>(`/api/media/${id}/export/${jobId}/progress`);
}

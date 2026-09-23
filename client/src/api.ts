// Thin fetch wrappers over the Rust server. Errors carry the server's message.

import type { ClientOp, DocState } from './ops';
import type {
  Asset,
  CutEdit,
  LibraryItem,
  ProjectSummary,
  SourceView,
  TokenInfo,
  User,
  Word,
} from './types';

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

/**
 * Called when the server answers 401 outside the auth routes, i.e. the session
 * expired mid-use. `useSession` registers a handler that drops the user so the
 * app routes back to the login screen instead of showing "sign in first".
 */
let onUnauthorized: (() => void) | null = null;

export function setUnauthorizedHandler(fn: (() => void) | null): void {
  onUnauthorized = fn;
}

const UNREACHABLE = 'Could not reach the server. Is `cargo run -p server` running?';

/** The server's `{ error }` text, else the status line. */
function errorMessage(body: unknown, status: number, statusText: string): string {
  return body && typeof body === 'object' && 'error' in body && typeof body.error === 'string'
    ? body.error
    : `${status} ${statusText}`;
}

export async function request<T>(url: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(url, init);
  } catch {
    throw new ApiError(UNREACHABLE, 0);
  }
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    // A failed sign-in is also a 401; it must not wipe an existing session.
    if (response.status === 401 && !url.startsWith('/api/auth/')) onUnauthorized?.();
    throw new ApiError(errorMessage(body, response.status, response.statusText), response.status);
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

/**
 * A multipart POST that reports how much of the body has gone up (fetch
 * cannot), with `request`'s errors. Progress ends at 1 once the server answers.
 */
export function upload<T>(
  url: string,
  form: FormData,
  onProgress?: (fraction: number) => void,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', url);
    if (onProgress) {
      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable && e.total > 0) onProgress(e.loaded / e.total);
      };
    }
    xhr.onerror = () => reject(new ApiError(UNREACHABLE, 0));
    xhr.onload = () => {
      let body: unknown = null;
      try {
        body = JSON.parse(xhr.responseText);
      } catch {
        body = null;
      }
      if (xhr.status < 200 || xhr.status >= 300) {
        if (xhr.status === 401) onUnauthorized?.();
        reject(new ApiError(errorMessage(body, xhr.status, xhr.statusText), xhr.status));
        return;
      }
      onProgress?.(1);
      resolve(body as T);
    };
    xhr.send(form);
  });
}

function fileForm(file: File): FormData {
  const form = new FormData();
  form.append('file', file, file.name);
  return form;
}

export function uploadMedia(
  file: File,
  onProgress?: (fraction: number) => void,
): Promise<ProjectSummary> {
  return upload<ProjectSummary>('/api/projects', fileForm(file), onProgress);
}

/** Append a file to the end of the project's main track (editors and owners). */
export function addSource(
  projectId: string,
  file: File,
  onProgress?: (fraction: number) => void,
): Promise<SourceView> {
  return upload<SourceView>(`/api/projects/${projectId}/sources`, fileForm(file), onProgress);
}

export function listLibrary(): Promise<LibraryItem[]> {
  return request<LibraryItem[]>('/api/library');
}

export function openLibraryClip(slug: string): Promise<ProjectSummary> {
  return request<ProjectSummary>(`/api/library/${encodeURIComponent(slug)}`, { method: 'POST' });
}

export function listProjects(): Promise<ProjectSummary[]> {
  return request<ProjectSummary[]>('/api/projects');
}

export function fetchProject(id: string): Promise<{ project: ProjectSummary; doc: DocState }> {
  return request(`/api/projects/${id}`);
}

export function submitOps(id: string, ops: ClientOp[]): Promise<DocState> {
  return postJson(`/api/projects/${id}/ops`, { ops });
}

/**
 * The stitched words of every source whose transcript is ready, and each
 * source's status. Waits for source 0 only; the server transcribes the
 * others in the background and starts any that is `pending`. `sources` is
 * absent from a server that predates sources.
 */
export function transcribeProject(id: string): Promise<{ words: Word[]; sources?: SourceView[] }> {
  return request(`/api/projects/${id}/transcribe`, { method: 'POST' });
}

export async function transcribeMedia(id: string): Promise<Word[]> {
  return (await transcribeProject(id)).words;
}

export interface Suggestions {
  fillers: CutEdit[];
  pauses: CutEdit[];
}

export function suggestEdits(id: string, twoWordFillers: boolean): Promise<Suggestions> {
  return postJson(`/api/projects/${id}/suggest`, { twoWordFillers });
}

export function synthesizeOverdub(
  id: string,
  text: string,
): Promise<{ audioUrl: string; duration: number }> {
  return postJson(`/api/projects/${id}/overdub`, { text });
}

export interface Speakers {
  /** Number of distinct speakers found. */
  count: number;
  /** Speaker index per transcript word, parallel to the word list. */
  words: (number | null)[];
}

export function fetchSpeakers(id: string): Promise<Speakers> {
  return request<Speakers>(`/api/projects/${id}/speakers`, { method: 'POST' });
}

export interface Thumbnails {
  url: string;
  count: number;
  columns: number;
  rows: number;
  /** Seconds between frames; cell i shows the frame at i * interval. */
  interval: number;
  width: number;
  height: number;
}

export function fetchThumbnails(id: string): Promise<Thumbnails> {
  return request<Thumbnails>(`/api/projects/${id}/thumbnails`, { method: 'POST' });
}

export interface ExportStarted {
  jobId: string;
  planned: number;
}

export type ExportJob =
  | { status: 'running'; progress: number }
  | { status: 'done'; progress: number; url: string; duration: number; bytes: number }
  | { status: 'error'; message: string };

export function exportMedia(id: string): Promise<ExportStarted> {
  return postJson(`/api/projects/${id}/export`, {});
}

export function exportProgress(id: string, jobId: string): Promise<ExportJob> {
  return request<ExportJob>(`/api/projects/${id}/export/${jobId}/progress`);
}

export function fetchMe(): Promise<User> {
  return request<User>('/api/me');
}

export function fetchSetup(): Promise<{ needsSetup: boolean }> {
  return request('/api/auth/setup');
}

export function register(email: string, password: string, displayName: string): Promise<User> {
  return postJson('/api/auth/register', { email, password, displayName });
}

export function login(email: string, password: string): Promise<User> {
  return postJson('/api/auth/login', { email, password });
}

export function logout(): Promise<void> {
  return request('/api/auth/logout', { method: 'POST' });
}

export function listTokens(): Promise<TokenInfo[]> {
  return request<TokenInfo[]>('/api/tokens');
}

export interface NewToken {
  token: string;
  id: string;
  label: string;
  createdAt: number;
}

export function createToken(label: string): Promise<NewToken> {
  return postJson('/api/tokens', { label });
}

export function revokeToken(id: string): Promise<{ ok: true }> {
  return request(`/api/tokens/${id}`, { method: 'DELETE' });
}

export function listAssets(id: string): Promise<Asset[]> {
  return request<Asset[]>(`/api/projects/${id}/assets`);
}

export function uploadAsset(id: string, file: File): Promise<Asset> {
  const form = new FormData();
  form.append('file', file, file.name);
  return request<Asset>(`/api/projects/${id}/assets`, { method: 'POST', body: form });
}

export function deleteAsset(id: string, assetId: string): Promise<{ ok: true }> {
  return request(`/api/projects/${id}/assets/${assetId}`, { method: 'DELETE' });
}

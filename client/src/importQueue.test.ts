import { describe, expect, it } from 'vitest';

import { ApiError } from './api';
import {
  byName,
  importFailures,
  itemsFor,
  moveItem,
  runImport,
  type ImportDeps,
  type ImportItem,
} from './importQueue';
import type { ProjectSummary, SourceView } from './types';

const file = (name: string) => new File(['x'], name, { type: 'video/mp4' });

const summary: ProjectSummary = {
  id: 'p',
  title: 'b',
  role: 'owner',
  media: { id: 'm0', filename: 'b.mp4', ext: 'mp4', duration: 10, kind: 'video', url: '/m0' },
  createdAt: 0,
};
const view: SourceView = {
  index: 1,
  mediaId: 'm1',
  url: '/m1',
  filename: 'c.mp4',
  kind: 'video',
  offset: 10,
  duration: 5,
  transcript: 'pending',
};

describe('byName and moveItem', () => {
  it('sorts by name with natural numbering, ignoring case', () => {
    const names = byName([file('take10.mp4'), file('take2.mp4'), file('Take1.mov')]).map(
      (f) => f.name,
    );
    expect(names).toEqual(['Take1.mov', 'take2.mp4', 'take10.mp4']);
  });

  it('moves one item and keeps the rest in order', () => {
    expect(moveItem(['a', 'b', 'c'], 0, 2)).toEqual(['b', 'c', 'a']);
    expect(moveItem(['a', 'b', 'c'], 2, 0)).toEqual(['c', 'a', 'b']);
    expect(moveItem(['a', 'b', 'c'], 1, 1)).toEqual(['a', 'b', 'c']);
    expect(moveItem(['a', 'b', 'c'], 0, 9)).toEqual(['a', 'b', 'c']);
  });

  it('starts every file queued', () => {
    expect(itemsFor([file('a.mp4')])).toEqual([
      { id: '0-a.mp4', name: 'a.mp4', status: 'queued', progress: 0, error: null },
    ]);
  });
});

describe('runImport', () => {
  function deps(calls: string[], failing: string[] = []): ImportDeps {
    return {
      create: async (f, onProgress) => {
        calls.push(`create ${f.name}`);
        if (failing.includes(f.name)) throw new ApiError('file too large', 413);
        onProgress(0.5);
        return summary;
      },
      append: async (id, f, onProgress) => {
        calls.push(`append ${id} ${f.name}`);
        if (failing.includes(f.name)) throw new ApiError('unreadable media', 400);
        onProgress(1);
        return view;
      },
    };
  }

  it('creates the project from the first file and appends the rest in order, one at a time', async () => {
    const calls: string[] = [];
    const seen: ImportItem[][] = [];
    const result = await runImport(
      [file('a.mp4'), file('b.mp4'), file('c.mp4')],
      deps(calls),
      (items) => seen.push(items),
    );
    expect(calls).toEqual(['create a.mp4', 'append p b.mp4', 'append p c.mp4']);
    expect(result.project).toBe(summary);
    expect(result.projectId).toBe('p');
    expect(result.items.map((i) => i.status)).toEqual(['done', 'done', 'done']);
    // a.mp4 reported its half-way mark, and no two files ever uploaded at once.
    expect(
      seen.some((items) => items[0]?.status === 'uploading' && items[0].progress === 0.5),
    ).toBe(true);
    expect(seen.some((items) => items.filter((i) => i.status === 'uploading').length > 1)).toBe(
      false,
    );
  });

  it('marks a failed file and carries on; the next file creates the project if the first fails', async () => {
    const calls: string[] = [];
    const result = await runImport(
      [file('bad.mp4'), file('b.mp4'), file('worse.mov'), file('c.mp4')],
      deps(calls, ['bad.mp4', 'worse.mov']),
      () => undefined,
    );
    expect(calls).toEqual([
      'create bad.mp4',
      'create b.mp4',
      'append p worse.mov',
      'append p c.mp4',
    ]);
    expect(result.items.map((i) => [i.status, i.error])).toEqual([
      ['error', 'file too large'],
      ['done', null],
      ['error', 'unreadable media'],
      ['done', null],
    ]);
    expect(importFailures(result.items)).toBe(
      'Could not import bad.mp4 (file too large); worse.mov (unreadable media)',
    );
    expect(importFailures(result.items.slice(1, 2))).toBeNull();
  });

  it('appends to an open project without creating one', async () => {
    const calls: string[] = [];
    const { append } = deps(calls);
    const result = await runImport([file('d.mp4')], { append }, () => undefined, 'p');
    expect(calls).toEqual(['append p d.mp4']);
    expect(result.project).toBeNull();
    expect(result.projectId).toBe('p');
  });
});

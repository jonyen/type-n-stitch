import { describe, expect, it } from 'vitest';

import { splitTurns, tokenize } from './tokens';
import type { Edit, Word } from './types';

const words: Word[] = [
  { id: 'w0', text: 'thankful', start: 0, end: 0.91 },
  { id: 'w1', text: 'for', start: 0.91, end: 1.25 },
  { id: 'w2', text: 'you', start: 1.25, end: 1.59 },
  { id: 'w3', text: 'as', start: 2.0, end: 2.3 },
];
const cut = (start: number, end: number): Edit => ({ kind: 'cut', start, end });
const overdub: Edit = {
  kind: 'overdub',
  start: 1.25,
  end: 2.0,
  text: 'to',
  audioUrl: '/data/x/overdub-0.wav',
  audioDuration: 0.4,
};

describe('tokenize', () => {
  it('emits one word token per word with no edits', () => {
    expect(tokenize(words, []).map((t) => t.kind)).toEqual(['word', 'word', 'word', 'word']);
  });

  it('collapses an overdubbed run into one token', () => {
    const tokens = tokenize(words, [{ ...overdub, end: 2.0, start: 0.91 }]);
    expect(tokens).toEqual([
      { kind: 'word', index: 0, word: words[0] },
      { kind: 'overdub', overdub: { ...overdub, start: 0.91 }, first: 1, last: 2 },
      { kind: 'word', index: 3, word: words[3] },
    ]);
  });

  it('keeps cut words as word tokens while cuts are shown', () => {
    expect(tokenize(words, [cut(0.91, 2.0)], true).map((t) => t.kind)).toEqual([
      'word',
      'word',
      'word',
      'word',
    ]);
  });

  it('collapses a run of cut words into one gap when cuts are hidden', () => {
    expect(tokenize(words, [cut(0.91, 2.0)], false)).toEqual([
      { kind: 'word', index: 0, word: words[0] },
      { kind: 'gap', first: 1, last: 2 },
      { kind: 'word', index: 3, word: words[3] },
    ]);
  });

  it('does not swallow an overdub into a neighbouring gap', () => {
    const tokens = tokenize(words, [cut(0.91, 1.25), overdub, cut(2.0, 20)], false);
    expect(tokens.map((t) => t.kind)).toEqual(['word', 'gap', 'overdub', 'gap']);
  });
});

describe('splitTurns', () => {
  it('keeps everything in one unlabeled turn without speakers', () => {
    const turns = splitTurns(tokenize(words, []), null);
    expect(turns).toHaveLength(1);
    expect(turns[0]?.speaker).toBeNull();
    expect(turns[0]?.tokens).toHaveLength(4);
  });

  it('starts a new turn whenever the speaker changes', () => {
    const turns = splitTurns(tokenize(words, []), [0, 0, 1, 0]);
    expect(turns.map((t) => [t.speaker, t.tokens.length])).toEqual([
      [0, 2],
      [1, 1],
      [0, 1],
    ]);
  });

  it('gives a collapsed token the speaker of its first word', () => {
    const tokens = tokenize(words, [{ ...overdub, start: 0.91, end: 2.3 }]);
    const turns = splitTurns(tokens, [0, 1, 0, 0]);
    expect(turns.map((t) => [t.speaker, t.tokens.map((k) => k.kind)])).toEqual([
      [0, ['word']],
      [1, ['overdub']],
    ]);
  });

  it('carries the previous speaker over unlabeled words', () => {
    const turns = splitTurns(tokenize(words, []), [1, null, null, 1]);
    expect(turns).toHaveLength(1);
    expect(turns[0]?.speaker).toBe(1);
  });

  it('returns no turns for an empty transcript', () => {
    expect(splitTurns([], [])).toEqual([]);
  });
});

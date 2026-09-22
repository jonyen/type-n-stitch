import { describe, expect, it } from 'vitest';

import { clipRuns, splitTurns, tokenize, tokenStart, turnContains } from './tokens';
import type { Edit, TitleEdit, Word } from './types';

const words: Word[] = [
  { id: 'w0', text: 'thankful', start: 0, end: 0.91 },
  { id: 'w1', text: 'for', start: 0.91, end: 1.25 },
  { id: 'w2', text: 'you', start: 1.25, end: 1.59 },
  { id: 'w3', text: 'as', start: 2.0, end: 2.3 },
];
const cut = (start: number, end: number): Edit => ({ kind: 'cut', start, end });
const cutEdit = cut;
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

describe('turnContains', () => {
  it('spans from the first token to the last word of the last token', () => {
    const tokens = tokenize(words, [{ ...overdub, start: 1.25, end: 2.3 }]);
    const [a, b] = splitTurns(tokens, [0, 0, 1, 1]);
    expect(a && [0, 1, 2].map((i) => turnContains(a, i))).toEqual([true, true, false]);
    expect(b && [1, 2, 3].map((i) => turnContains(b, i))).toEqual([false, true, true]);
  });

  it('is false for -1, the index before any word plays', () => {
    const [turn] = splitTurns(tokenize(words, []), null);
    expect(turn && turnContains(turn, -1)).toBe(false);
  });
});

describe('title tokens', () => {
  it('places a title token before the first word at or after its instant', () => {
    const t: TitleEdit = {
      kind: 'title',
      at: 1.0,
      duration: 2,
      text: 'T',
      subtitle: null,
      style: 'dark',
    };
    const kinds = tokenize(words, [t]).map((k) =>
      k.kind === 'title' ? `title@${k.before}` : k.kind,
    );
    expect(kinds).toEqual(['word', 'title@1', 'word', 'word', 'word']);
    const late: TitleEdit = { ...t, at: 19 };
    expect(tokenize(words, [late]).at(-1)).toMatchObject({ kind: 'title', before: 4 });
  });
});

describe('clipRuns', () => {
  it('groups tokens by owning piece and orders them', () => {
    const edits: Edit[] = [
      cutEdit(1.25, 2.0),
      { kind: 'title', at: 5, duration: 1, text: 'T', subtitle: null, style: 'dark' },
    ];
    const tokens = tokenize(words, edits, true);
    const ordered = [
      { start: 2, end: 20 },
      { start: 0, end: 1.25 },
    ];
    const clips = clipRuns(words, tokens, ordered);
    expect(clips.map((c) => c.piece.start)).toEqual([2, 0]);
    expect(clips[0]?.tokens.map(tokenStart)).toEqual([3, 4]); // word "as" then the trailing title
    expect(clips[1]?.tokens.map(tokenStart)).toEqual([0, 1, 2]); // cut word 2 stays with the first source piece
  });
});

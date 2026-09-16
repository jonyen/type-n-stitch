// Turning the word list plus the edit list into what the transcript draws.

import { overdubs, wordStatus } from './editlist';
import type { Edit, OverdubEdit, Word } from './types';

export type Token =
  | { kind: 'word'; index: number; word: Word }
  | { kind: 'overdub'; overdub: OverdubEdit; first: number; last: number }
  | { kind: 'gap'; first: number; last: number };

/**
 * Words, with each overdubbed run collapsed into one token showing its new
 * text and, when cuts are hidden, each run of cut words collapsed into a gap.
 */
export function tokenize(words: Word[], edits: Edit[], showCuts = true): Token[] {
  const tokens: Token[] = [];
  const dubs = overdubs(edits);
  const covered = (word: Word | undefined, od: OverdubEdit) =>
    word !== undefined && wordStatus(word, [od]) === 'overdub';
  const isCut = (word: Word | undefined) => word !== undefined && wordStatus(word, edits) === 'cut';

  let i = 0;
  while (i < words.length) {
    const word = words[i];
    if (!word) break;
    const od = dubs.find((d) => covered(word, d));
    if (od) {
      let last = i;
      while (covered(words[last + 1], od)) last++;
      tokens.push({ kind: 'overdub', overdub: od, first: i, last });
      i = last + 1;
    } else if (!showCuts && isCut(word)) {
      let last = i;
      while (isCut(words[last + 1]) && !dubs.some((d) => covered(words[last + 1], d))) last++;
      tokens.push({ kind: 'gap', first: i, last });
      i = last + 1;
    } else {
      tokens.push({ kind: 'word', index: i, word });
      i++;
    }
  }
  return tokens;
}

/** Index of the first transcript word a token stands for. */
export function tokenStart(token: Token): number {
  return token.kind === 'word' ? token.index : token.first;
}

/** Index of the last transcript word a token stands for. */
export function tokenEnd(token: Token): number {
  return token.kind === 'word' ? token.index : token.last;
}

/** Whether word `index` falls inside `turn`. */
export function turnContains(turn: Turn, index: number): boolean {
  const first = turn.tokens[0];
  const last = turn.tokens[turn.tokens.length - 1];
  return (
    first !== undefined &&
    last !== undefined &&
    index >= tokenStart(first) &&
    index <= tokenEnd(last)
  );
}

export interface Turn {
  /** null when speakers are unknown; the whole transcript is then one turn. */
  speaker: number | null;
  tokens: Token[];
}

/**
 * Tokens split into consecutive runs by speaker. A token belongs to the
 * speaker of its first word. With no speaker labels, returns a single turn.
 */
export function splitTurns(tokens: Token[], speakers: (number | null)[] | null): Turn[] {
  if (!speakers) return tokens.length ? [{ speaker: null, tokens }] : [];
  const turns: Turn[] = [];
  let current: Turn | undefined;
  for (const token of tokens) {
    const speaker = speakers[tokenStart(token)] ?? current?.speaker ?? null;
    if (!current || speaker !== current.speaker) {
      current = { speaker, tokens: [] };
      turns.push(current);
    }
    current.tokens.push(token);
  }
  return turns;
}

/** Display name for a speaker index: the user's name for them, or "Speaker n". */
export function speakerLabel(speaker: number, names: string[]): string {
  return names[speaker]?.trim() || `Speaker ${speaker + 1}`;
}

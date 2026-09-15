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

// Turning the word list plus the edit list into what the transcript draws.

import { EPS, overdubs, titles, wordStatus } from './editlist';
import type { Edit, OverdubEdit, Range, TitleEdit, Word } from './types';

export type Token =
  | { kind: 'word'; index: number; word: Word }
  | { kind: 'overdub'; overdub: OverdubEdit; first: number; last: number }
  | { kind: 'gap'; first: number; last: number }
  /** A title card, drawn before word `before` (`words.length` past the end). */
  | { kind: 'title'; title: TitleEdit; before: number };

/**
 * Words, with each overdubbed run collapsed into one token showing its new
 * text and, when cuts are hidden, each run of cut words collapsed into a gap.
 */
export function tokenize(words: Word[], edits: Edit[], showCuts = true): Token[] {
  const tokens: Token[] = [];
  const dubs = overdubs(edits);
  // Title cards, earliest first, each emitted before the word its instant
  // falls in — so a title at a word's exact start reads before that word.
  const cards = titles(edits).sort((a, b) => a.at - b.at);
  let card = 0;
  const covered = (word: Word | undefined, od: OverdubEdit) =>
    word !== undefined && wordStatus(word, [od]) === 'overdub';
  const isCut = (word: Word | undefined) => word !== undefined && wordStatus(word, edits) === 'cut';

  let i = 0;
  while (i < words.length) {
    const word = words[i];
    if (!word) break;
    while (card < cards.length) {
      const title = cards[card];
      if (!title || title.at >= word.end) break;
      tokens.push({ kind: 'title', title, before: i });
      card++;
    }
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
  for (; card < cards.length; card++) {
    const title = cards[card];
    if (title) tokens.push({ kind: 'title', title, before: words.length });
  }
  return tokens;
}

/** Index of the first transcript word a token stands for. */
export function tokenStart(token: Token): number {
  if (token.kind === 'word') return token.index;
  if (token.kind === 'title') return token.before;
  return token.first;
}

/** Index of the last transcript word a token stands for. */
export function tokenEnd(token: Token): number {
  if (token.kind === 'word') return token.index;
  // A title stands between words rather than over one, so it starts and ends
  // at the word it precedes; past the last word that index is `words.length`,
  // which no word ever equals, so it claims nothing.
  if (token.kind === 'title') return token.before;
  return token.last;
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

export interface Clip {
  piece: Range;
  tokens: Token[];
}

/**
 * Tokens grouped by the piece owning their first word, in `ordered` order.
 * Cut words between pieces stay with the preceding piece; trailing titles
 * with the last.
 */
export function clipRuns(words: Word[], tokens: Token[], ordered: Range[]): Clip[] {
  const source = [...ordered].sort((a, b) => a.start - b.start);
  // First word index each source piece owns; run k covers [firstIdx[k], firstIdx[k+1]).
  // firstIdx[0] is hardcoded to 0 so any words before the first kept piece
  // (e.g. a leading cut) attach to the first clip rather than being orphaned.
  const firstIdx = source.map((p, k) =>
    k === 0 ? 0 : words.findIndex((w) => w.start >= p.start - EPS),
  );
  const runOf = (index: number) => {
    let k = 0;
    for (let i = 1; i < source.length; i++)
      if ((firstIdx[i] ?? -1) !== -1 && index >= (firstIdx[i] as number)) k = i;
    return k;
  };
  const buckets: Token[][] = source.map(() => []);
  for (const token of tokens) {
    const start = tokenStart(token);
    const k = start >= words.length ? source.length - 1 : runOf(start);
    buckets[k]?.push(token);
  }
  return ordered.map((piece) => {
    const k = source.findIndex((p) => Math.abs(p.start - piece.start) < EPS);
    return { piece, tokens: buckets[k] ?? [] };
  });
}

import { overdubs, wordStatus } from '../editlist';
import type { Edit, OverdubEdit, Word } from '../types';

interface Props {
  words: Word[];
  edits: Edit[];
  /** Inclusive index range of the selection. */
  selected: [number, number] | null;
  activeWord: number;
  onWordClick: (index: number, extend: boolean) => void;
  onOverdubClick: (overdub: OverdubEdit) => void;
}

type Token =
  | { kind: 'word'; index: number; word: Word }
  | { kind: 'overdub'; overdub: OverdubEdit; first: number; last: number };

/** Words, with each overdubbed run collapsed into one token showing its new text. */
function tokenize(words: Word[], edits: Edit[]): Token[] {
  const tokens: Token[] = [];
  const dubs = overdubs(edits);
  const covered = (word: Word | undefined, od: OverdubEdit) =>
    word !== undefined && wordStatus(word, [od]) === 'overdub';

  let i = 0;
  while (i < words.length) {
    const word = words[i];
    if (!word) break;
    const od = dubs.find((d) => covered(word, d));
    if (!od) {
      tokens.push({ kind: 'word', index: i, word });
      i++;
      continue;
    }
    let last = i;
    while (covered(words[last + 1], od)) last++;
    tokens.push({ kind: 'overdub', overdub: od, first: i, last });
    i = last + 1;
  }
  return tokens;
}

export function Transcript({
  words,
  edits,
  selected,
  activeWord,
  onWordClick,
  onOverdubClick,
}: Props) {
  const inSelection = (i: number) => selected !== null && i >= selected[0] && i <= selected[1];

  return (
    <p className="transcript" aria-label="Transcript">
      {tokenize(words, edits).map((token) => {
        if (token.kind === 'overdub') {
          const { overdub, first, last } = token;
          const active = activeWord >= first && activeWord <= last;
          const original = words
            .slice(first, last + 1)
            .map((w) => w.text)
            .join(' ');
          return (
            <span key={`od-${first}`}>
              <button
                type="button"
                className={`token overdub${active ? ' active' : ''}${inSelection(first) ? ' selected' : ''}`}
                title={`Overdub replacing “${original}”`}
                onClick={(e) => {
                  onWordClick(first, e.shiftKey);
                  if (!e.shiftKey) onOverdubClick(overdub);
                }}
              >
                {overdub.text}
              </button>{' '}
            </span>
          );
        }
        const { index, word } = token;
        const status = wordStatus(word, edits);
        const classes = ['token', status];
        if (inSelection(index)) classes.push('selected');
        if (index === activeWord) classes.push('active');
        return (
          <span key={word.id}>
            <button
              type="button"
              className={classes.join(' ')}
              data-index={index}
              onClick={(e) => onWordClick(index, e.shiftKey)}
            >
              {word.text}
            </button>{' '}
          </span>
        );
      })}
    </p>
  );
}

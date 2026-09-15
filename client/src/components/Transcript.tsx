import { useEffect, useRef, type MouseEvent } from 'react';

import { wordStatus } from '../editlist';
import { tokenize } from '../tokens';
import type { Edit, OverdubEdit, Word } from '../types';

interface Props {
  words: Word[];
  edits: Edit[];
  /** Inclusive index range of the selection. */
  selected: [number, number] | null;
  activeWord: number;
  /** When false, cut words collapse into a "…" marker so the text reads as the output. */
  showCuts: boolean;
  onWordClick: (index: number, extend: boolean) => void;
  /** The pointer, held down since a word, has reached another word. */
  onWordDrag: (index: number) => void;
  onOverdubClick: (overdub: OverdubEdit) => void;
}

export function Transcript({
  words,
  edits,
  selected,
  activeWord,
  showCuts,
  onWordClick,
  onWordDrag,
  onOverdubClick,
}: Props) {
  const inSelection = (i: number) => selected !== null && i >= selected[0] && i <= selected[1];

  // Mouse-down on a word starts a drag; every word the pointer enters while
  // the button is held extends the selection, like selecting text.
  const dragging = useRef(false);
  useEffect(() => {
    const stop = () => {
      dragging.current = false;
    };
    window.addEventListener('mouseup', stop);
    window.addEventListener('blur', stop);
    return () => {
      window.removeEventListener('mouseup', stop);
      window.removeEventListener('blur', stop);
    };
  }, []);

  const press = (index: number, e: MouseEvent<HTMLButtonElement>, after?: () => void) => {
    if (e.button !== 0) return;
    // No native text selection, but keep keyboard focus moving to the word so
    // the shortcuts work after clicking a checkbox or link.
    e.preventDefault();
    e.currentTarget.focus();
    dragging.current = true;
    onWordClick(index, e.shiftKey);
    if (!e.shiftKey) after?.();
  };
  const enter = (index: number) => {
    if (dragging.current) onWordDrag(index);
  };
  // Buttons also "click" from the keyboard (Enter/Space); detail is 0 then.
  const keyActivate = (index: number, e: MouseEvent) => {
    if (e.detail === 0) onWordClick(index, e.shiftKey);
  };

  return (
    <p className="transcript" aria-label="Transcript">
      {tokenize(words, edits, showCuts).map((token) => {
        if (token.kind === 'gap') {
          const n = token.last - token.first + 1;
          return (
            <span key={`gap-${token.first}`}>
              <span className="gap" title={`${n} word${n === 1 ? '' : 's'} cut`} aria-label="cut">
                …
              </span>{' '}
            </span>
          );
        }
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
                onMouseDown={(e) => press(first, e, () => onOverdubClick(overdub))}
                onMouseEnter={() => enter(first)}
                onClick={(e) => keyActivate(first, e)}
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
              onMouseDown={(e) => press(index, e)}
              onMouseEnter={() => enter(index)}
              onClick={(e) => keyActivate(index, e)}
            >
              {word.text}
            </button>{' '}
          </span>
        );
      })}
    </p>
  );
}

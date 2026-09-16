import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react';

import { wordStatus } from '../editlist';
import {
  speakerLabel,
  splitTurns,
  tokenize,
  tokenStart,
  turnContains,
  type Token,
} from '../tokens';
import type { Edit, OverdubEdit, Word } from '../types';

interface Props {
  words: Word[];
  edits: Edit[];
  /** Inclusive index range of the selection. */
  selected: [number, number] | null;
  activeWord: number;
  /** While playing, the transcript follows the active word. */
  playing: boolean;
  /** When false, cut words collapse into a "…" marker so the text reads as the output. */
  showCuts: boolean;
  onWordClick: (index: number, extend: boolean) => void;
  /** The pointer, held down since a word, has reached another word. */
  onWordDrag: (index: number) => void;
  onOverdubClick: (overdub: OverdubEdit) => void;
  /** Speaker per word, or null when unknown / a single speaker. */
  speakers: (number | null)[] | null;
  speakerNames: string[];
  onRenameSpeaker: (speaker: number, name: string) => void;
}

export function Transcript({
  words,
  edits,
  selected,
  activeWord,
  playing,
  showCuts,
  onWordClick,
  onWordDrag,
  onOverdubClick,
  speakers,
  speakerNames,
  onRenameSpeaker,
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

  // Follow playback: when the active word leaves the viewport, bring it back
  // to the middle. A wheel or touch scroll pauses following for a few
  // seconds so reading ahead isn't yanked back.
  const root = useRef<HTMLDivElement>(null);
  const userScrolledAt = useRef(0);
  useEffect(() => {
    const note = () => {
      userScrolledAt.current = Date.now();
    };
    window.addEventListener('wheel', note, { passive: true });
    window.addEventListener('touchmove', note, { passive: true });
    return () => {
      window.removeEventListener('wheel', note);
      window.removeEventListener('touchmove', note);
    };
  }, []);
  useEffect(() => {
    if (!playing || activeWord < 0 || Date.now() - userScrolledAt.current < 4000) return;
    const el = root.current?.querySelector<HTMLElement>('.token.active');
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const margin = 80;
    if (rect.top < margin || rect.bottom > window.innerHeight - margin) {
      el.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
  }, [activeWord, playing]);

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

  const renderToken = (token: Token): ReactNode => {
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
  };

  const turns = splitTurns(tokenize(words, edits, showCuts), speakers);
  return (
    <div ref={root} className="transcript" aria-label="Transcript">
      {turns.map((turn) => {
        const first = turn.tokens[0];
        const key = first ? `turn-${tokenStart(first)}` : 'turn';
        return (
          <div
            key={key}
            className={[
              'turn',
              turn.speaker !== null ? `speaker-${turn.speaker % 6}` : '',
              turnContains(turn, activeWord) ? 'speaking' : '',
            ]
              .filter(Boolean)
              .join(' ')}
          >
            {turn.speaker !== null && (
              <SpeakerTag
                speaker={turn.speaker}
                name={speakerLabel(turn.speaker, speakerNames)}
                onRename={(name) => onRenameSpeaker(turn.speaker as number, name)}
              />
            )}
            <p className={turn.speaker !== null ? `speech speaker-${turn.speaker % 6}` : 'speech'}>
              {turn.tokens.map(renderToken)}
            </p>
          </div>
        );
      })}
    </div>
  );
}

interface TagProps {
  speaker: number;
  name: string;
  onRename: (name: string) => void;
}

/** Speaker name above a turn; click to rename every turn by that speaker. */
function SpeakerTag({ speaker, name, onRename }: TagProps) {
  const [editing, setEditing] = useState(false);
  const className = `speaker-tag speaker-${speaker % 6}`;
  if (editing) {
    return (
      <input
        className={className}
        defaultValue={name}
        autoFocus
        aria-label="Speaker name"
        onFocus={(e) => e.currentTarget.select()}
        onBlur={(e) => {
          onRename(e.currentTarget.value);
          setEditing(false);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') e.currentTarget.blur();
          if (e.key === 'Escape') setEditing(false);
        }}
      />
    );
  }
  return (
    <button
      type="button"
      className={className}
      title="Rename this speaker"
      onClick={() => setEditing(true)}
    >
      {name}
    </button>
  );
}

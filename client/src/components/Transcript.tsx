import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent,
  type ReactNode,
} from 'react';

import {
  captions,
  cutTransitionAt,
  EPS,
  formatTime,
  nextCutTransition,
  wordStatus,
} from '../editlist';
import { audios, brolls } from '../overlays';
import type { Peer } from '../realtime';
import {
  clipRuns,
  speakerLabel,
  splitTurns,
  tokenize,
  tokenStart,
  turnContains,
  type Token,
} from '../tokens';
import type { Tool } from '../tools';
import type { Asset, Edit, OverdubEdit, Range, Transition, Word } from '../types';

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
  /** The `at` of the selected title card, or null. Exclusive with a word selection. */
  selectedTitle: number | null;
  onTitleClick: (at: number) => void;
  /** Double-click or Enter on a card: open it for editing. */
  onTitleOpen: (at: number) => void;
  /** Click a caption tag to remove that caption. */
  onCaptionClick: (start: number) => void;
  /** Cycle the transition override on the cut starting at `start`. */
  onCutTransition: (start: number, transition: Transition | null) => void;
  /** Speaker per word, or null when unknown / a single speaker. */
  speakers: (number | null)[] | null;
  speakerNames: string[];
  onRenameSpeaker: (speaker: number, name: string) => void;
  /** Viewers and commenters read the transcript; they cannot rename speakers. */
  readOnly: boolean;
  peers: Peer[];
  /** Output pieces, in output order. */
  ordered: Range[];
  /** Instants where the output is split into clips. */
  splits: number[];
  /** The `start` of the selected clip's piece, or null. Exclusive with a word/title selection. */
  selectedClip: number | null;
  onClipClick: (start: number) => void;
  assets: Asset[];
  onBrollClick: (start: number) => void;
  onAudioClick: (start: number) => void;
  /** The active timeline tool; the transcript's cursor follows it. */
  tool: Tool;
  /** Fired on mouseup after a press that started on a word (the end of a drag or a click). */
  onWordDragEnd?: () => void;
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
  selectedTitle,
  onTitleClick,
  onTitleOpen,
  onCaptionClick,
  onCutTransition,
  speakers,
  speakerNames,
  onRenameSpeaker,
  readOnly,
  peers,
  ordered,
  splits,
  selectedClip,
  onClipClick,
  assets,
  onBrollClick,
  onAudioClick,
  tool,
  onWordDragEnd,
}: Props) {
  const inSelection = (i: number) => selected !== null && i >= selected[0] && i <= selected[1];

  // Which peer (first wins) covers each word, and which peers sit before a word.
  const peerFor = (i: number) =>
    peers.find(
      (p) => p.state.selection !== null && i >= p.state.selection[0] && i <= p.state.selection[1],
    );
  const caretsAt = (i: number) => peers.filter((p) => p.state.caret === i);

  // Mouse-down on a word starts a drag; every word the pointer enters while
  // the button is held extends the selection, like selecting text.
  const dragging = useRef(false);
  const dragEnd = useRef(onWordDragEnd);
  useEffect(() => {
    dragEnd.current = onWordDragEnd;
  }, [onWordDragEnd]);
  useEffect(() => {
    const end = () => {
      if (dragging.current) dragEnd.current?.();
      dragging.current = false;
    };
    // Losing focus mid-drag abandons it rather than completing it.
    const cancel = () => {
      dragging.current = false;
    };
    window.addEventListener('mouseup', end);
    window.addEventListener('blur', cancel);
    return () => {
      window.removeEventListener('mouseup', end);
      window.removeEventListener('blur', cancel);
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

  // The item from `list` that begins at word `i`, so its tag is drawn once,
  // after the first word it covers. An item's range starts at that word's
  // start.
  const startingAt = <T extends Range>(list: T[], i: number): T | undefined => {
    const word = words[i];
    if (!word) return undefined;
    return list.find(
      (c) =>
        word.start >= c.start - EPS &&
        word.start < c.end - EPS &&
        (i === 0 || (words[i - 1]?.start ?? -1) < c.start - EPS),
    );
  };

  const captionList = captions(edits);
  const captionTag = (i: number): ReactNode => {
    const caption = startingAt(captionList, i);
    if (!caption) return null;
    return (
      <span
        className="caption-tag"
        title={readOnly ? 'Caption' : 'Click to remove'}
        onClick={readOnly ? undefined : () => onCaptionClick(caption.start)}
      >
        {caption.text}
      </span>
    );
  };

  const nameOf = (id: string) => assets.find((a) => a.id === id)?.name ?? 'missing asset';
  const overlayTags = (i: number): ReactNode => {
    const b = startingAt(brolls(edits), i);
    const a = startingAt(audios(edits), i);
    return (
      <>
        {b && (
          <span
            className="overlay-tag broll"
            title={readOnly ? 'B-roll' : 'Click to remove'}
            onClick={readOnly ? undefined : () => onBrollClick(b.start)}
          >
            ▣ {nameOf(b.media)}
          </span>
        )}
        {a && (
          <span
            className="overlay-tag audio"
            title={readOnly ? 'Music' : 'Click to edit'}
            onClick={readOnly ? undefined : () => onAudioClick(a.start)}
          >
            ♪ {nameOf(a.media)} {a.gain}dB
          </span>
        )}
      </>
    );
  };

  // `at` is not unique — two cards may share an instant — so a title's key
  // comes from where the token sits in the turn instead.
  const renderToken = (token: Token, tokenIndex: number): ReactNode => {
    if (token.kind === 'gap') {
      const n = token.last - token.first + 1;
      // The cut op's start is the first cut word's start, which is how
      // `deleteSelection` builds it, so that instant names this cut.
      const start = words[token.first]?.start;
      const override = start === undefined ? null : cutTransitionAt(start, edits);
      return (
        <span key={`gap-${token.first}`}>
          <span className="gap" title={`${n} word${n === 1 ? '' : 's'} cut`} aria-label="cut">
            …
          </span>
          {!readOnly && start !== undefined && (
            <button
              type="button"
              className="gap-transition"
              title="Transition for this cut"
              aria-label="Transition for this cut"
              onClick={() => onCutTransition(start, nextCutTransition(override))}
            >
              {override ?? '·'}
            </button>
          )}{' '}
        </span>
      );
    }
    if (token.kind === 'title') {
      const { title } = token;
      const selected = selectedTitle !== null && Math.abs(selectedTitle - title.at) < EPS;
      return (
        <button
          key={`title-${token.before}-${tokenIndex}`}
          type="button"
          data-at={title.at}
          className={`title-token ${title.style}${selected ? ' selected' : ''}`}
          title={readOnly ? title.text : 'Click to select · double-click to edit'}
          onClick={readOnly ? undefined : () => onTitleClick(title.at)}
          onDoubleClick={readOnly ? undefined : () => onTitleOpen(title.at)}
          onKeyDown={(e) => {
            if (!readOnly && e.key === 'Enter') onTitleOpen(title.at);
          }}
        >
          <span className="title-token-text">{title.text}</span>
          <span className="muted">{title.duration}s</span>
        </button>
      );
    }
    if (token.kind === 'overdub') {
      const { overdub, first, last } = token;
      const active = activeWord >= first && activeWord <= last;
      const original = words
        .slice(first, last + 1)
        .map((w) => w.text)
        .join(' ');
      const odPeer = peerFor(first);
      const odClasses = ['token', 'overdub'];
      if (active) odClasses.push('active');
      if (inSelection(first)) odClasses.push('selected');
      if (odPeer) odClasses.push('peer-selected');
      return (
        <span key={`od-${first}`}>
          {caretsAt(first).map((p) => (
            <span
              key={p.connId}
              className="peer-caret"
              style={{ background: p.user.color }}
              data-name={p.user.displayName}
              aria-hidden
            />
          ))}
          <button
            type="button"
            className={odClasses.join(' ')}
            data-index={first}
            style={odPeer ? ({ '--peer': odPeer.user.color } as CSSProperties) : undefined}
            title={`Overdub replacing “${original}”`}
            onMouseDown={(e) => press(first, e, () => onOverdubClick(overdub))}
            onMouseEnter={() => enter(first)}
            onClick={(e) => keyActivate(first, e)}
          >
            {overdub.text}
          </button>
          {captionTag(first)}
          {overlayTags(first)}{' '}
        </span>
      );
    }
    const { index, word } = token;
    const status = wordStatus(word, edits);
    const classes = ['token', status];
    if (inSelection(index)) classes.push('selected');
    if (index === activeWord) classes.push('active');
    const peer = peerFor(index);
    if (peer) classes.push('peer-selected');
    return (
      <span key={word.id}>
        {caretsAt(index).map((p) => (
          <span
            key={p.connId}
            className="peer-caret"
            style={{ background: p.user.color }}
            data-name={p.user.displayName}
            aria-hidden
          />
        ))}
        <button
          type="button"
          className={classes.join(' ')}
          data-index={index}
          style={peer ? ({ '--peer': peer.user.color } as CSSProperties) : undefined}
          onMouseDown={(e) => press(index, e)}
          onMouseEnter={() => enter(index)}
          onClick={(e) => keyActivate(index, e)}
        >
          {word.text}
        </button>
        {captionTag(index)}
        {overlayTags(index)}{' '}
      </span>
    );
  };

  const tokens = tokenize(words, edits, showCuts);
  const clips = clipRuns(words, tokens, ordered);
  const isSplit = (start: number) => splits.some((s) => Math.abs(s - start) < EPS);
  return (
    <div ref={root} className="transcript" data-tool={tool} aria-label="Transcript">
      {clips.map((clip, k) => (
        <section key={`clip-${clip.piece.start}`} className="clip" data-start={clip.piece.start}>
          {clips.length > 1 &&
            (() => {
              const split = isSplit(clip.piece.start);
              // Only a split's divider can show "selected": that's the only
              // one Delete acts on. A cut-derived boundary still jumps there
              // on click, but never carries a selection Delete can't use.
              const selected =
                split && selectedClip !== null && Math.abs(selectedClip - clip.piece.start) < EPS;
              return (
                <button
                  type="button"
                  className={`clip-divider${selected ? ' selected' : ''}${split ? ' split' : ''}`}
                  title={
                    split
                      ? 'Click to select · Delete joins it to the clip before'
                      : 'Clip boundary from a cut — click to jump here'
                  }
                  onClick={() => onClipClick(clip.piece.start)}
                >
                  Clip {k + 1} · {formatTime(clip.piece.end - clip.piece.start)}
                </button>
              );
            })()}
          {splitTurns(clip.tokens, speakers).map((turn) => {
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
                    readOnly={readOnly}
                  />
                )}
                <p
                  className={
                    turn.speaker !== null ? `speech speaker-${turn.speaker % 6}` : 'speech'
                  }
                >
                  {turn.tokens.map(renderToken)}
                </p>
              </div>
            );
          })}
        </section>
      ))}
    </div>
  );
}

interface TagProps {
  speaker: number;
  name: string;
  onRename: (name: string) => void;
  readOnly: boolean;
}

/** Speaker name above a turn; click to rename every turn by that speaker. */
function SpeakerTag({ speaker, name, onRename, readOnly }: TagProps) {
  const [editing, setEditing] = useState(false);
  const className = `speaker-tag speaker-${speaker % 6}`;
  if (readOnly) return <span className={className}>{name}</span>;
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

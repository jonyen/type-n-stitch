import { useRef, useState, type PointerEvent } from 'react';
import type { Thumbnails } from '../api';
import { dropSlot, firstWords, moveFor } from '../clipstrip';
import { EPS, formatTime } from '../editlist';
import type { Range, Word } from '../types';

interface Props {
  ordered: Range[];
  words: Word[];
  thumbs: Thumbnails | null;
  selected: number | null;
  readOnly: boolean;
  onSelect: (start: number) => void;
  onMove: (piece: number, before: number | null) => void;
}

export function ClipStrip({ ordered, words, thumbs, selected, readOnly, onSelect, onMove }: Props) {
  const strip = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<{ from: number; slot: number } | null>(null);
  if (ordered.length < 2) return null;

  const centres = () =>
    Array.from(strip.current?.querySelectorAll<HTMLElement>('.clip-card') ?? []).map((el) => {
      const r = el.getBoundingClientRect();
      return r.left + r.width / 2;
    });
  const down = (i: number, e: PointerEvent<HTMLButtonElement>) => {
    if (readOnly || e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ from: i, slot: i });
  };
  const move = (e: PointerEvent<HTMLButtonElement>) => {
    if (drag) setDrag({ ...drag, slot: dropSlot(e.clientX, centres()) });
  };
  const up = (i: number) => {
    if (!drag) return;
    const op = moveFor(ordered, drag.from, drag.slot);
    setDrag(null);
    if (op) onMove(op.piece, op.before);
    else if (drag.slot === drag.from || drag.slot === drag.from + 1)
      onSelect((ordered[i] as Range).start);
  };
  const frame = (t: number) =>
    thumbs && {
      backgroundImage: `url(${thumbs.url})`,
      backgroundSize: `${thumbs.columns * thumbs.width}px ${thumbs.rows * thumbs.height}px`,
      backgroundPosition: (() => {
        const index = Math.min(Math.floor(t / thumbs.interval), thumbs.count - 1);
        return `-${(index % thumbs.columns) * thumbs.width}px -${Math.floor(index / thumbs.columns) * thumbs.height}px`;
      })(),
    };

  return (
    <div ref={strip} className="clip-strip" role="list" aria-label="Clips">
      {ordered.map((piece, i) => (
        <span key={piece.start} className="clip-slot">
          {drag && drag.slot === i && drag.slot !== drag.from && drag.slot !== drag.from + 1 && (
            <span className="drop-indicator" />
          )}
          <button
            type="button"
            role="listitem"
            className={`clip-card${selected !== null && Math.abs(selected - piece.start) < EPS ? ' selected' : ''}${drag?.from === i ? ' dragging' : ''}`}
            onPointerDown={(e) => down(i, e)}
            onPointerMove={move}
            onPointerUp={() => up(i)}
            onPointerCancel={() => setDrag(null)}
            title={readOnly ? 'Clip' : 'Drag to reorder · click to select'}
          >
            <span className="clip-poster" style={frame(piece.start) ?? undefined}>
              <span className="length">{formatTime(piece.end - piece.start)}</span>
            </span>
            <span className="clip-words">{firstWords(words, piece) || '…'}</span>
          </button>
        </span>
      ))}
      {drag && drag.slot === ordered.length && drag.from !== ordered.length - 1 && (
        <span className="drop-indicator" />
      )}
    </div>
  );
}

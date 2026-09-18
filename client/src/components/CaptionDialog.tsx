import { useEffect, useRef, useState, type FormEvent } from 'react';

import type { CaptionPos } from '../types';

interface Props {
  /** The selected words, which the caption starts out as. */
  original: string;
  onSubmit: (text: string, position: CaptionPos) => void;
  onCancel: () => void;
}

const POSITIONS: { value: CaptionPos; label: string }[] = [
  { value: 'bottomLeft', label: 'Bottom left' },
  { value: 'bottomCenter', label: 'Bottom center' },
  { value: 'topLeft', label: 'Top left' },
];

export function CaptionDialog({ original, onSubmit, onCancel }: Props) {
  const [text, setText] = useState(original);
  const [position, setPosition] = useState<CaptionPos>('bottomCenter');
  const first = useRef<HTMLInputElement>(null);

  useEffect(() => {
    first.current?.focus();
    first.current?.select();
  }, []);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!text.trim()) return;
    onSubmit(text.trim(), position);
  };

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <form
        className="modal"
        onClick={(e) => e.stopPropagation()}
        onSubmit={submit}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.stopPropagation();
            onCancel();
          }
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) e.currentTarget.requestSubmit();
        }}
      >
        <h2>Add caption</h2>
        <p className="muted">Text drawn over the picture while the selected words play.</p>
        <label className="field">
          Caption
          <input
            ref={first}
            type="text"
            value={text}
            maxLength={200}
            required
            onChange={(e) => setText(e.target.value)}
          />
        </label>
        <label className="field">
          Position
          <select value={position} onChange={(e) => setPosition(e.target.value as CaptionPos)}>
            {POSITIONS.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
              </option>
            ))}
          </select>
        </label>
        <div className="actions">
          <span className="spacer" />
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={!text.trim()}>
            Add caption
          </button>
        </div>
      </form>
    </div>
  );
}

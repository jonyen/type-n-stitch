import { useEffect, useRef, useState, type FormEvent } from 'react';

import { cx } from '../cx';
import type { TitleEdit, TitleStyle } from '../types';
import { DialogFrame } from './DialogFrame';
import frame from './DialogFrame.module.css';
import ui from '../styles/ui.module.css';
import titleStyles from './TitleDialog.module.css';

/** Everything about a title card except where it sits. */
export type TitleFields = Omit<TitleEdit, 'kind' | 'at'>;

interface Props {
  /** When editing an existing card, the card; absent when adding one. */
  initial?: TitleEdit | undefined;
  /** The instant the card plays at, for the prompt. */
  at: number;
  onSubmit: (fields: TitleFields) => void;
  onCancel: () => void;
}

const STYLES: { value: TitleStyle; label: string }[] = [
  { value: 'dark', label: 'Dark' },
  { value: 'light', label: 'Light' },
  { value: 'accent', label: 'Accent' },
];

const CHIP: Record<TitleStyle, string | undefined> = {
  dark: titleStyles.dark,
  light: titleStyles.light,
  accent: titleStyles.accent,
};

export function TitleDialog({ initial, at, onSubmit, onCancel }: Props) {
  const [text, setText] = useState(initial?.text ?? '');
  const [subtitle, setSubtitle] = useState(initial?.subtitle ?? '');
  const [style, setStyle] = useState<TitleStyle>(initial?.style ?? 'dark');
  const [duration, setDuration] = useState(String(initial?.duration ?? 3));
  const first = useRef<HTMLInputElement>(null);

  useEffect(() => {
    first.current?.focus();
    first.current?.select();
  }, []);

  const seconds = Number(duration);
  const validDuration = Number.isFinite(seconds) && seconds >= 0.5 && seconds <= 30;
  const ready = text.trim().length > 0 && validDuration;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!ready) return;
    onSubmit({
      duration: seconds,
      text: text.trim(),
      subtitle: subtitle.trim() ? subtitle.trim() : null,
      style,
    });
  };

  return (
    <DialogFrame
      title={initial ? 'Edit title' : 'Add title'}
      description={`A full-frame card at ${at.toFixed(1)}s. The output grows by its duration.`}
      onClose={onCancel}
    >
      <form
        className={frame.form}
        onSubmit={submit}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) e.currentTarget.requestSubmit();
        }}
      >
        <label className={ui.field}>
          Title
          <input
            ref={first}
            type="text"
            value={text}
            maxLength={200}
            required
            placeholder="Chapter one"
            onChange={(e) => setText(e.target.value)}
          />
        </label>
        <label className={ui.field}>
          Subtitle <span className={ui.muted}>(optional)</span>
          <input
            type="text"
            value={subtitle}
            maxLength={200}
            placeholder="A line underneath"
            onChange={(e) => setSubtitle(e.target.value)}
          />
        </label>
        <fieldset className={cx(ui.field, titleStyles.styles)}>
          <legend>Style</legend>
          {STYLES.map((s) => (
            <label
              key={s.value}
              className={cx(titleStyles.chip, CHIP[s.value], style === s.value && titleStyles.on)}
            >
              <input
                type="radio"
                name="title-style"
                value={s.value}
                checked={style === s.value}
                onChange={() => setStyle(s.value)}
              />
              {s.label}
            </label>
          ))}
        </fieldset>
        <label className={cx(ui.field, titleStyles.duration)}>
          Duration (seconds)
          <input
            type="number"
            min={0.5}
            max={30}
            step={0.5}
            value={duration}
            onChange={(e) => setDuration(e.target.value)}
          />
        </label>
        <div className={frame.actions}>
          <span className={frame.spacer} />
          <button type="button" className={ui.button} onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className={cx(ui.button, ui.primary)} disabled={!ready}>
            {initial ? 'Save' : 'Add title'}
          </button>
        </div>
      </form>
    </DialogFrame>
  );
}

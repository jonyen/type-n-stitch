import { useEffect, useRef, useState, type FormEvent } from 'react';

import { cx } from '../cx';
import { DialogFrame } from './DialogFrame';
import frame from './DialogFrame.module.css';
import ui from '../styles/ui.module.css';

interface Props {
  /** The words being replaced, for the prompt. */
  original: string;
  onSubmit: (text: string) => Promise<void>;
  onCancel: () => void;
}

export function OverdubDialog({ original, onSubmit, onCancel }: Props) {
  const [text, setText] = useState(original);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const textarea = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    textarea.current?.focus();
    textarea.current?.select();
  }, []);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit(text.trim());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setBusy(false);
    }
  };

  return (
    <DialogFrame
      title="Overdub"
      description={
        <>
          Replacing <q>{original}</q>. Type what should be said instead; VoiceStudio will speak it
          in the cloned voice.
        </>
      }
      busy={busy}
      onClose={onCancel}
    >
      <form className={frame.form} onSubmit={submit}>
        <label className={ui.field}>
          Say instead
          <textarea
            ref={textarea}
            rows={3}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey))
                e.currentTarget.form?.requestSubmit();
            }}
            disabled={busy}
          />
        </label>
        {error && <p className={ui.error}>{error}</p>}
        <div className={frame.actions}>
          <span className={frame.spacer} />
          <button type="button" className={ui.button} onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button
            type="submit"
            className={cx(ui.button, ui.primary)}
            disabled={busy || !text.trim()}
          >
            {busy ? (
              <>
                <span className={ui.spinnerSmall} aria-hidden /> Synthesizing…
              </>
            ) : (
              'Generate'
            )}
          </button>
        </div>
      </form>
    </DialogFrame>
  );
}

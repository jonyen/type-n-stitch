import { useEffect, useRef, useState, type FormEvent } from 'react';

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
    <div className="modal-backdrop" onClick={busy ? undefined : onCancel}>
      <form className="modal" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <h2>Overdub</h2>
        <p className="muted">
          Replacing <q>{original}</q>. Type what should be said instead; VoiceStudio will speak it
          in the cloned voice.
        </p>
        <textarea
          ref={textarea}
          rows={3}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey))
              e.currentTarget.form?.requestSubmit();
            if (e.key === 'Escape' && !busy) onCancel();
          }}
          disabled={busy}
        />
        {error && <p className="error">{error}</p>}
        <div className="actions">
          <span className="spacer" />
          <button type="button" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={busy || !text.trim()}>
            {busy ? (
              <>
                <span className="spinner small" aria-hidden /> Synthesizing…
              </>
            ) : (
              'Generate'
            )}
          </button>
        </div>
      </form>
    </div>
  );
}

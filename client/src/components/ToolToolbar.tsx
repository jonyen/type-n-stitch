import { ToggleGroup } from 'radix-ui';
import { useEffect } from 'react';

import { handledUpstream, shouldIgnoreGlobalKey } from '../keyboardGuard';
import { TOOLS, toolForKey, type Tool } from '../tools';
import { Tip } from './Tip';
import styles from './ToolToolbar.module.css';

interface Props {
  tool: Tool;
  onChange: (tool: Tool) => void;
  /** Viewers get Select only. */
  readOnly: boolean;
  /** Off while a dialog is open, so typing there never switches tools. */
  shortcuts: boolean;
}

/** The tool picker across the top of the timeline, with V / C / X / Escape. */
export function ToolToolbar({ tool, onChange, readOnly, shortcuts }: Props) {
  useEffect(() => {
    if (!shortcuts) return;
    const onKey = (e: KeyboardEvent) => {
      // Form fields, open Radix menus and handled keys keep their own V / C / X / Escape.
      if (shouldIgnoreGlobalKey(e.target as Element | null, handledUpstream(e))) return;
      if (e.key === 'Escape') {
        onChange('select');
        return;
      }
      const next = toolForKey(e);
      if (!next || (readOnly && next !== 'select')) return;
      e.preventDefault();
      onChange(next);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [shortcuts, readOnly, onChange]);

  const active = TOOLS.find((t) => t.id === tool);
  return (
    <div className={styles.bar}>
      <ToggleGroup.Root
        type="single"
        value={tool}
        onValueChange={(v) => {
          if (v) onChange(v as Tool);
        }}
        aria-label="Tools"
        className={styles.group}
      >
        {TOOLS.map((t) => (
          <Tip key={t.id} label={`${t.label} (${t.key})`}>
            <ToggleGroup.Item
              value={t.id}
              className={styles.tool}
              disabled={readOnly && t.id !== 'select'}
              aria-label={`${t.label} (${t.key})`}
            >
              <span aria-hidden>{t.icon}</span> {t.label}
              <kbd className={styles.key}>{t.key}</kbd>
            </ToggleGroup.Item>
          </Tip>
        ))}
      </ToggleGroup.Root>
      {active && <span className={styles.hint}>{active.hint}</span>}
    </div>
  );
}

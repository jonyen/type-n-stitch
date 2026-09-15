import { useCallback, useEffect, useReducer, useRef, useState } from 'react';

import {
  exportMedia,
  suggestEdits,
  synthesizeOverdub,
  transcribeMedia,
  uploadMedia,
  type Suggestions,
} from './api';
import { Dropzone } from './components/Dropzone';
import { OverdubDialog } from './components/OverdubDialog';
import { Player } from './components/Player';
import { Toolbar, type ExportState } from './components/Toolbar';
import { Transcript } from './components/Transcript';
import { editorReducer, initialEditor, selectedRange } from './editor';
import { defaultSuggestOptions, fillerCuts, pauseCuts, pending } from './suggest';
import type { Media } from './types';
import { usePlayback } from './usePlayback';

export function App() {
  const [media, setMedia] = useState<Media | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editor, dispatch] = useReducer(editorReducer, initialEditor);
  const [overdubOpen, setOverdubOpen] = useState(false);
  const [exportState, setExportState] = useState<ExportState>({ status: 'idle' });
  const [twoWordFillers, setTwoWordFillers] = useState(false);
  const [showCuts, setShowCuts] = useState(true);
  const [suggestions, setSuggestions] = useState<Suggestions>({ fillers: [], pauses: [] });

  const mediaRef = useRef<HTMLVideoElement>(null);
  const playback = usePlayback(mediaRef, editor.words, editor.edits, editor.duration);

  const selected = selectedRange(editor.selection);
  const fillers = pending(suggestions.fillers, editor.edits);
  const pauses = pending(suggestions.pauses, editor.edits);

  // Ask the engine for suggestions; fall back to the local mirror if the
  // server is unreachable so the buttons still work.
  const { words, duration } = editor;
  useEffect(() => {
    if (!media || words.length === 0) return;
    const opts = { ...defaultSuggestOptions, twoWordFillers };
    let cancelled = false;
    suggestEdits(media.id, twoWordFillers)
      .catch(() => ({ fillers: fillerCuts(words, duration, opts), pauses: pauseCuts(words, opts) }))
      .then((s) => {
        if (!cancelled) setSuggestions(s);
      });
    return () => {
      cancelled = true;
    };
  }, [media, words, duration, twoWordFillers]);

  const onFile = useCallback(async (file: File) => {
    setLoadError(null);
    setExportState({ status: 'idle' });
    try {
      setBusy(`Uploading ${file.name}`);
      const uploaded = await uploadMedia(file);
      setBusy('Transcribing');
      const words = await transcribeMedia(uploaded.id);
      dispatch({ type: 'load', words, duration: uploaded.duration });
      setMedia(uploaded);
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }, []);

  const onWordClick = useCallback(
    (index: number, extend: boolean) => {
      dispatch({ type: 'select', index, extend });
      const word = editor.words[index];
      if (word && !extend) playback.seek(word.start);
    },
    [editor.words, playback],
  );

  const onWordDrag = useCallback((index: number) => {
    dispatch({ type: 'select', index, extend: true });
  }, []);

  const onExport = useCallback(async () => {
    if (!media) return;
    setExportState({ status: 'rendering' });
    try {
      const result = await exportMedia(media.id, editor.edits);
      setExportState({ status: 'done', ...result });
    } catch (err) {
      setExportState({
        status: 'error',
        message: err instanceof Error ? err.message : String(err),
      });
    }
  }, [editor.edits, media]);

  const onOverdubSubmit = useCallback(
    async (text: string) => {
      if (!media) return;
      const { audioUrl, duration } = await synthesizeOverdub(media.id, text);
      dispatch({ type: 'overdub', text, audioUrl, audioDuration: duration });
      setOverdubOpen(false);
    },
    [media],
  );

  // Keyboard: Delete cuts, ⌘Z undoes, Space plays, Esc clears, arrows move.
  useEffect(() => {
    if (!media) return;
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (overdubOpen || target?.closest('input, textarea, [contenteditable]')) return;
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        dispatch({ type: 'deleteSelection' });
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z' && !e.shiftKey) {
        e.preventDefault();
        dispatch({ type: 'undo' });
      } else if (e.key === ' ') {
        e.preventDefault();
        playback.toggle();
      } else if (e.key === 'Escape') {
        dispatch({ type: 'clearSelection' });
      } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
        e.preventDefault();
        dispatch({
          type: 'move',
          delta: e.key === 'ArrowLeft' ? -1 : 1,
          extend: e.shiftKey,
          skipCut: !showCuts,
        });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [media, overdubOpen, playback, showCuts]);

  const selectedText = selected
    ? editor.words
        .slice(selected[0], selected[1] + 1)
        .map((w) => w.text)
        .join(' ')
    : '';

  return (
    <div className="app">
      <header>
        <h1>
          <span className="logo" aria-hidden />
          type-n-stitch
        </h1>
        <span className="tagline muted">edit media by editing its words</span>
        <span className="spacer" />
        {media && (
          <button
            type="button"
            className="ghost"
            onClick={() => {
              mediaRef.current?.pause();
              setMedia(null);
              dispatch({ type: 'load', words: [], duration: 0 });
              setExportState({ status: 'idle' });
            }}
          >
            New file
          </button>
        )}
      </header>

      {!media ? (
        <Dropzone onFile={onFile} busy={busy} error={loadError} />
      ) : (
        <main className="editor">
          <section className="stage">
            <Player media={media} mediaRef={mediaRef} edits={editor.edits} playback={playback} />
            <Toolbar
              hasSelection={selected !== null}
              canUndo={editor.past.length > 0}
              fillerCount={fillers.length}
              pauseCount={pauses.length}
              twoWordFillers={twoWordFillers}
              showCuts={showCuts}
              exportState={exportState}
              onDelete={() => dispatch({ type: 'deleteSelection' })}
              onRemoveFillers={() => dispatch({ type: 'applyCuts', cuts: fillers })}
              onTightenPauses={() => dispatch({ type: 'applyCuts', cuts: pauses })}
              onTwoWordFillers={setTwoWordFillers}
              onShowCuts={setShowCuts}
              onOverdub={() => setOverdubOpen(true)}
              onUndo={() => dispatch({ type: 'undo' })}
              onExport={onExport}
            />
          </section>
          <section className="script">
            <Transcript
              words={editor.words}
              edits={editor.edits}
              selected={selected}
              activeWord={playback.activeWord}
              showCuts={showCuts}
              onWordClick={onWordClick}
              onWordDrag={onWordDrag}
              onOverdubClick={(od) => playback.seek(od.start)}
            />
          </section>
        </main>
      )}

      {overdubOpen && selected && (
        <OverdubDialog
          original={selectedText}
          onSubmit={onOverdubSubmit}
          onCancel={() => setOverdubOpen(false)}
        />
      )}
    </div>
  );
}

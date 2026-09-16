import { useCallback, useEffect, useReducer, useRef, useState } from 'react';

import {
  exportMedia,
  exportProgress,
  fetchSpeakers,
  openLibraryClip,
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
import type { LibraryItem, Media } from './types';
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

  const [speakers, setSpeakers] = useState<(number | null)[] | null>(null);
  const [speakerNames, setSpeakerNames] = useState<string[]>([]);

  const mediaRef = useRef<HTMLVideoElement>(null);
  const playback = usePlayback(mediaRef, editor.words, editor.edits, editor.duration, media?.url);

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

  // Back to the start screen. Edits live only in this state, so they are dropped.
  const goHome = useCallback(() => {
    mediaRef.current?.pause();
    setMedia(null);
    setLoadError(null);
    dispatch({ type: 'load', words: [], duration: 0 });
    setExportState({ status: 'idle' });
  }, []);

  // Opening media pushes a history entry, so the browser's Back button (and
  // the header's back link) return to the start screen.
  useEffect(() => {
    if (!media) return;
    if (history.state?.media !== media.id) history.pushState({ media: media.id }, '');
    const onPop = () => goHome();
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, [media, goHome]);

  const onBack = useCallback(() => {
    if (history.state?.media) history.back();
    else goHome();
  }, [goHome]);

  const load = useCallback(async (label: string, fetchMedia: () => Promise<Media>) => {
    setLoadError(null);
    setExportState({ status: 'idle' });
    try {
      setBusy(label);
      const loaded = await fetchMedia();
      setBusy('Transcribing');
      const words = await transcribeMedia(loaded.id);
      dispatch({ type: 'load', words, duration: loaded.duration });
      setMedia(loaded);
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }, []);

  const onFile = useCallback(
    (file: File) => load(`Uploading ${file.name}`, () => uploadMedia(file)),
    [load],
  );

  const onLibraryClip = useCallback(
    (item: LibraryItem) => load(`Opening ${item.title}`, () => openLibraryClip(item.slug)),
    [load],
  );

  // Speaker labels arrive after the transcript; the editor is usable before.
  // One speaker means nothing to split, so labels stay hidden.
  useEffect(() => {
    setSpeakers(null);
    if (!media) return;
    setSpeakerNames(readSpeakerNames(media.id));
    let cancelled = false;
    fetchSpeakers(media.id)
      .then((s) => {
        if (!cancelled && s.count > 1) setSpeakers(s.words);
      })
      .catch(() => {
        // Diarization not set up or failed: keep the plain transcript.
      });
    return () => {
      cancelled = true;
    };
  }, [media]);

  const onRenameSpeaker = useCallback(
    (speaker: number, name: string) => {
      if (!media) return;
      setSpeakerNames((names) => {
        const next = [...names];
        next[speaker] = name.trim();
        writeSpeakerNames(media.id, next);
        return next;
      });
    },
    [media],
  );

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
    setExportState({ status: 'rendering', progress: 0 });
    try {
      const { jobId } = await exportMedia(media.id, editor.edits);
      for (;;) {
        await new Promise((r) => setTimeout(r, 300));
        const job = await exportProgress(media.id, jobId);
        if (job.status === 'running') {
          setExportState({ status: 'rendering', progress: job.progress });
        } else if (job.status === 'done') {
          setExportState({ status: 'rendering', progress: 1 });
          await new Promise((r) => setTimeout(r, 250));
          setExportState({
            status: 'done',
            url: job.url,
            duration: job.duration,
            bytes: job.bytes,
          });
          return;
        } else {
          setExportState({ status: 'error', message: job.message });
          return;
        }
      }
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
        {media && (
          <button type="button" className="ghost back" onClick={onBack} title="Back to samples">
            <span aria-hidden>←</span> Home
          </button>
        )}
        <h1>
          {media ? (
            <a
              href="/"
              className="home-link"
              onClick={(e) => {
                e.preventDefault();
                onBack();
              }}
            >
              <span className="logo" aria-hidden />
              type-n-stitch
            </a>
          ) : (
            <>
              <span className="logo" aria-hidden />
              type-n-stitch
            </>
          )}
        </h1>
        <span className="tagline muted">edit media by editing its words</span>
        <span className="spacer" />
        {media && <span className="header-file muted">{media.filename}</span>}
      </header>

      {!media ? (
        <Dropzone onFile={onFile} onLibraryClip={onLibraryClip} busy={busy} error={loadError} />
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
              playing={playback.playing}
              showCuts={showCuts}
              onWordClick={onWordClick}
              onWordDrag={onWordDrag}
              onOverdubClick={(od) => playback.seek(od.start)}
              speakers={speakers}
              speakerNames={speakerNames}
              onRenameSpeaker={onRenameSpeaker}
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

// Speaker names are a per-browser convenience, keyed by media id.
function readSpeakerNames(id: string): string[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(`speakers:${id}`) ?? '[]');
    return Array.isArray(parsed) ? parsed.map((n) => (typeof n === 'string' ? n : '')) : [];
  } catch {
    return [];
  }
}

function writeSpeakerNames(id: string, names: string[]) {
  try {
    localStorage.setItem(`speakers:${id}`, JSON.stringify(names));
  } catch {
    // Storage unavailable; names last for this session only.
  }
}

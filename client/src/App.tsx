import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';

import {
  exportMedia,
  exportProgress,
  fetchProject,
  fetchSetup,
  fetchSpeakers,
  listAssets,
  listProjects,
  openLibraryClip,
  submitOps,
  suggestEdits,
  synthesizeOverdub,
  transcribeMedia,
  uploadAsset,
  uploadMedia,
  type Suggestions,
} from './api';
import { AgentDialog } from './components/AgentDialog';
import { AudioDialog } from './components/AudioDialog';
import { Avatar } from './components/Avatar';
import { BrollDialog } from './components/BrollDialog';
import { CaptionDialog } from './components/CaptionDialog';
import { ClipStrip } from './components/ClipStrip';
import { Dropzone } from './components/Dropzone';
import { Login } from './components/Login';
import { OverdubDialog } from './components/OverdubDialog';
import { Player } from './components/Player';
import { Presence } from './components/Presence';
import { Projects } from './components/Projects';
import { TitleDialog, type TitleFields } from './components/TitleDialog';
import { Toolbar, type ExportState } from './components/Toolbar';
import { Transcript } from './components/Transcript';
import { EPS, orderedPieces, rangeForWords, titles } from './editlist';
import { editorReducer, initialEditor, selectedRange, type EditorAction } from './editor';
import { createOpQueue, type OpQueue } from './opQueue';
import { newOpId, opForAction, type ClientOp, type DocState } from './ops';
import { type PresenceState } from './realtime';
import { useSession } from './session';
import { defaultSuggestOptions, fillerCuts, pauseCuts, pending } from './suggest';
import type {
  Asset,
  AudioEdit,
  CaptionPos,
  LibraryItem,
  ProjectSummary,
  TitleEdit,
  Transition,
} from './types';
import { usePlayback } from './usePlayback';
import { holdOrApply, useRealtime, type RemoteDoc } from './useRealtime';
import { useThumbnails } from './useThumbnails';

export function App() {
  const { user, setUser, signOut } = useSession();
  const [needsSetup, setNeedsSetup] = useState(false);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const canEdit = project?.role === 'owner' || project?.role === 'editor';

  const [busy, setBusy] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editor, dispatch] = useReducer(editorReducer, initialEditor);
  const [overdubOpen, setOverdubOpen] = useState(false);
  // The title dialog, adding at `at` or editing `initial`.
  const [titleDialog, setTitleDialog] = useState<{ at: number; initial?: TitleEdit } | null>(null);
  const [agentDialogOpen, setAgentDialogOpen] = useState(false);
  // The word range the caption dialog was opened on. Holding it here keeps
  // the dialog mounted (and the caption anchored) when a peer's `sync` clears
  // the selection while someone is still typing.
  const [captionRange, setCaptionRange] = useState<[number, number] | null>(null);
  // The word range the B-roll dialog was opened on.
  const [brollRange, setBrollRange] = useState<[number, number] | null>(null);
  // The music dialog: adding over `range` (null means the whole edit), or
  // editing an existing `edit`.
  const [audioDialog, setAudioDialog] = useState<
    { range: [number, number] | null } | { edit: AudioEdit } | null
  >(null);
  // A selected title card, by its instant. Exclusive with the word selection.
  const [selectedTitle, setSelectedTitle] = useState<number | null>(null);
  // A selected clip, by its piece's start. Exclusive with the word/title selection.
  const [selectedClip, setSelectedClip] = useState<number | null>(null);
  const [assets, setAssets] = useState<Asset[]>([]);
  const [exportState, setExportState] = useState<ExportState>({ status: 'idle' });
  const [twoWordFillers, setTwoWordFillers] = useState(false);
  const [showCuts, setShowCuts] = useState(true);
  const [suggestions, setSuggestions] = useState<Suggestions>({ fillers: [], pauses: [] });

  const [speakers, setSpeakers] = useState<(number | null)[] | null>(null);

  // The output layout for the current edit list, so playback can jump
  // between output pieces and the transcript can draw clip boundaries.
  const ordered = useMemo(
    () => orderedPieces(editor.duration, editor.edits, editor.splits, editor.order),
    [editor.duration, editor.edits, editor.splits, editor.order],
  );

  const mediaRef = useRef<HTMLVideoElement>(null);
  const playback = usePlayback(
    mediaRef,
    editor.words,
    editor.edits,
    editor.duration,
    project?.media.url,
    ordered,
  );
  const thumbs = useThumbnails(project?.id ?? null, project?.media.kind);

  // The server's last confirmed document, for rolling back a rejected op.
  const confirmed = useRef<DocState | null>(null);

  // One serial queue per open project; ops go out in order, one at a time.
  const queue = useRef<OpQueue | null>(null);

  // A peer's fold that arrived while our own edits were still in flight; see
  // `holdOrApply`. Applied as soon as the queue drains.
  const heldRemote = useRef<RemoteDoc | null>(null);

  /** Once nothing is in flight, the fold we held back can land. */
  const releaseHeldRemote = useCallback(() => {
    const held = heldRemote.current;
    if (!held || queue.current?.pending !== 0) return;
    heldRemote.current = null;
    dispatch({ type: 'remote', ...held });
  }, []);

  /** Accept the server's fold as the truth: remember it and show it. */
  const settle = useCallback(
    (doc: DocState) => {
      confirmed.current = doc;
      setLoadError(null);
      dispatch({ type: 'sync', doc });
      releaseHeldRemote();
    },
    [releaseHeldRemote],
  );

  /**
   * Apply an editing action locally, send its operation, and settle on the
   * server's fold. A rejected operation rolls back to the last confirmed doc.
   */
  const edit = useCallback(
    (action: EditorAction) => {
      if (!project || !canEdit) return;
      const op = opForAction(editor, action);
      dispatch(action);
      if (!op) return;
      const clientOp: ClientOp = { ...op, opId: newOpId() };
      queue.current
        ?.push(clientOp)
        .then(settle)
        .catch((err: unknown) => {
          setLoadError(err instanceof Error ? err.message : String(err));
          if (confirmed.current) dispatch({ type: 'sync', doc: confirmed.current });
          releaseHeldRemote();
        });
    },
    [project, editor, canEdit, settle, releaseHeldRemote],
  );

  // Undo and redo are server round-trips: the fold decides what they mean.
  const undoRedo = useCallback(
    (kind: 'undo' | 'redo') => {
      if (!project || !canEdit) return;
      const targetSeq = kind === 'undo' ? editor.undoable : editor.redoable;
      if (targetSeq === null) return;
      queue.current
        ?.push({ kind, targetSeq, opId: newOpId() })
        .then(settle)
        .catch((err: unknown) => {
          setLoadError(err instanceof Error ? err.message : String(err));
          releaseHeldRemote();
        });
    },
    [project, canEdit, editor.undoable, editor.redoable, settle, releaseHeldRemote],
  );

  const selected = selectedRange(editor.selection);
  const fillers = pending(suggestions.fillers, editor.edits);
  const pauses = pending(suggestions.pauses, editor.edits);

  useEffect(() => {
    fetchSetup()
      .then((s) => setNeedsSetup(s.needsSetup))
      .catch((err: unknown) => console.error(err));
  }, []);

  useEffect(() => {
    if (!user) {
      setProjects([]);
      return;
    }
    let cancelled = false;
    listProjects()
      .then((list) => {
        if (!cancelled) setProjects(list);
      })
      .catch((err: unknown) => console.error(err));
    return () => {
      cancelled = true;
    };
  }, [user, project]);

  // Ask the engine for suggestions; fall back to the local mirror if the
  // server is unreachable so the buttons still work.
  const { words, duration } = editor;
  const projectId = project?.id ?? null;
  useEffect(() => {
    if (!projectId || words.length === 0) return;
    const opts = { ...defaultSuggestOptions, twoWordFillers };
    let cancelled = false;
    suggestEdits(projectId, twoWordFillers)
      .catch(() => ({ fillers: fillerCuts(words, duration, opts), pauses: pauseCuts(words, opts) }))
      .then((s) => {
        if (!cancelled) setSuggestions(s);
      });
    return () => {
      cancelled = true;
    };
  }, [projectId, words, duration, twoWordFillers]);

  // The project's uploaded B-roll/music assets.
  const refreshAssets = useCallback(() => {
    if (!projectId) return;
    listAssets(projectId)
      .then(setAssets)
      .catch(() => setAssets([]));
  }, [projectId]);
  useEffect(() => {
    setAssets([]);
    refreshAssets();
  }, [refreshAssets]);
  const onUploadAsset = useCallback(
    async (file: File) => {
      if (!projectId) throw new Error('no project');
      const asset = await uploadAsset(projectId, file);
      setAssets((list) => [...list, asset]);
      return asset;
    },
    [projectId],
  );

  useEffect(() => {
    if (!projectId) {
      queue.current = null;
      heldRemote.current = null;
      return;
    }
    const q = createOpQueue((ops) => submitOps(projectId, ops));
    queue.current = q;
    return () => {
      // Closing matters: a queue left retrying would POST to a project the
      // user has left, forever.
      q.close();
      queue.current = null;
      heldRemote.current = null;
    };
  }, [projectId]);

  // The socket delivers everyone's appends, including our own; `remote` keeps
  // only folds newer than ours, so it never fights the reply to our own POST.
  // A fold arriving while our own edit is un-acked waits instead of erasing it.
  const onRemoteDoc = useCallback((doc: RemoteDoc) => {
    const { apply, held } = holdOrApply(doc, heldRemote.current, queue.current?.pending ?? 0);
    heldRemote.current = held;
    if (apply) dispatch({ type: 'remote', ...apply });
  }, []);
  const onSocketOpen = useCallback(() => queue.current?.flush(), []);
  const { peers, status, lastError, sendPresence } = useRealtime(
    projectId,
    onRemoteDoc,
    onSocketOpen,
  );

  // Tell peers where we are: playhead, selection, caret. Coalesced by the socket client.
  const selection = editor.selection;
  useEffect(() => {
    if (!projectId) return;
    // `selected` is a fresh tuple every render, so this depends on the
    // selection itself and derives the range here.
    const range = selectedRange(selection);
    const state: PresenceState = {
      playhead: playback.currentTime,
      selection: range,
      caret: range && range[0] === range[1] ? range[0] : null,
      playing: playback.playing,
    };
    sendPresence(state);
  }, [projectId, playback.currentTime, playback.playing, selection, sendPresence]);

  // Back to the start screen. The document lives on the server, so nothing is lost.
  const goHome = useCallback(() => {
    mediaRef.current?.pause();
    setProject(null);
    setLoadError(null);
    confirmed.current = null;
    heldRemote.current = null;
    dispatch({ type: 'load', words: [], duration: 0 });
    setExportState({ status: 'idle' });
    setSelectedTitle(null);
    setSelectedClip(null);
    setTitleDialog(null);
    setCaptionRange(null);
    setBrollRange(null);
    setAudioDialog(null);
  }, []);

  // Opening a project pushes a history entry, so the browser's Back button
  // (and the header's back link) return to the start screen.
  useEffect(() => {
    if (!projectId) return;
    if (history.state?.project !== projectId) history.pushState({ project: projectId }, '');
    const onPop = () => goHome();
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, [projectId, goHome]);

  // A different account must not inherit the previous one's open document.
  const userId = user?.id ?? null;
  const lastUserId = useRef(userId);
  useEffect(() => {
    if (lastUserId.current === userId) return;
    lastUserId.current = userId;
    goHome();
  }, [userId, goHome]);

  const onBack = useCallback(() => {
    if (history.state?.project) history.back();
    else goHome();
  }, [goHome]);

  const load = useCallback(async (label: string, fetchSummary: () => Promise<ProjectSummary>) => {
    setLoadError(null);
    setExportState({ status: 'idle' });
    try {
      setBusy(label);
      const summary = await fetchSummary();
      setBusy('Transcribing');
      const [words, { doc }] = await Promise.all([
        transcribeMedia(summary.id),
        fetchProject(summary.id),
      ]);
      dispatch({ type: 'load', words, duration: summary.media.duration });
      dispatch({ type: 'sync', doc });
      confirmed.current = doc;
      setProject(summary);
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }, []);

  const onOpenProject = useCallback(
    (p: ProjectSummary) => load(`Opening ${p.title}`, () => Promise.resolve(p)),
    [load],
  );

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
    if (!projectId) return;
    let cancelled = false;
    fetchSpeakers(projectId)
      .then((s) => {
        if (!cancelled && s.count > 1) setSpeakers(s.words);
      })
      .catch(() => {
        // Diarization not set up or failed: keep the plain transcript.
      });
    return () => {
      cancelled = true;
    };
  }, [projectId]);

  const onRenameSpeaker = useCallback(
    (speaker: number, name: string) => edit({ type: 'renameSpeaker', speaker, name }),
    [edit],
  );

  const onWordClick = useCallback(
    (index: number, extend: boolean) => {
      setSelectedTitle(null);
      setSelectedClip(null);
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
    if (!projectId) return;
    setExportState({ status: 'rendering', progress: 0 });
    try {
      const { jobId } = await exportMedia(projectId);
      for (;;) {
        await new Promise((r) => setTimeout(r, 300));
        const job = await exportProgress(projectId, jobId);
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
  }, [projectId]);

  const onOverdubSubmit = useCallback(
    async (text: string) => {
      if (!projectId) return;
      const { audioUrl, duration } = await synthesizeOverdub(projectId, text);
      edit({ type: 'overdub', text, audioUrl, audioDuration: duration });
      setOverdubOpen(false);
    },
    [projectId, edit],
  );

  /** Where a new card goes: the end of the selected words, else the playhead. */
  const onAddTitle = useCallback(() => {
    const at = selected
      ? rangeForWords(editor.words, selected[0], selected[1], editor.duration).end
      : playback.currentTime;
    setTitleDialog({ at });
  }, [selected, editor.words, editor.duration, playback.currentTime]);

  const onTitleOpen = useCallback(
    (at: number) => {
      const initial = titles(editor.edits).find((t) => Math.abs(t.at - at) < EPS);
      if (initial) setTitleDialog({ at: initial.at, initial });
    },
    [editor.edits],
  );

  const onTitleSubmit = useCallback(
    (fields: TitleFields) => {
      if (!titleDialog) return;
      const { at, initial } = titleDialog;
      edit(initial ? { type: 'editTitle', at, ...fields } : { type: 'addTitle', at, ...fields });
      setTitleDialog(null);
    },
    [titleDialog, edit],
  );

  // Selecting a card and selecting words are exclusive: one clears the other.
  const onTitleClick = useCallback((at: number) => {
    setSelectedTitle(at);
    setSelectedClip(null);
    dispatch({ type: 'clearSelection' });
  }, []);

  // Clicking any divider jumps there; only a split's divider can be
  // "selected" (Delete then joins it to the clip before). A cut-derived
  // boundary just seeks, since there's no split for Delete to undo.
  const onClipClick = useCallback(
    (start: number) => {
      setSelectedClip(editor.splits.some((s) => Math.abs(s - start) < EPS) ? start : null);
      setSelectedTitle(null);
      dispatch({ type: 'clearSelection' });
      playback.seek(start);
      document
        .querySelector<HTMLElement>(`.clip[data-start="${start}"]`)
        ?.scrollIntoView({ block: 'start', behavior: 'smooth' });
    },
    [playback, editor.splits],
  );

  /** Where a new split goes: the start of the selected words, else the playhead. */
  const onSplit = useCallback(() => {
    const at = selected
      ? rangeForWords(editor.words, selected[0], selected[1], editor.duration).start
      : playback.currentTime;
    edit({ type: 'split', at });
  }, [selected, editor.words, editor.duration, playback.currentTime, edit]);

  const onCaptionSubmit = useCallback(
    (text: string, position: CaptionPos) => {
      if (!captionRange) return;
      edit({ type: 'addCaption', text, position, range: captionRange });
      setCaptionRange(null);
    },
    [captionRange, edit],
  );

  // A peer's edit (or an undo) can remove the card we had selected.
  useEffect(() => {
    if (selectedTitle === null) return;
    if (!titles(editor.edits).some((t) => Math.abs(t.at - selectedTitle) < EPS))
      setSelectedTitle(null);
  }, [editor.edits, selectedTitle]);

  // A peer's edit (or an undo) can remove the clip boundary we had selected.
  useEffect(() => {
    if (selectedClip === null) return;
    if (!ordered.some((p) => Math.abs(p.start - selectedClip) < EPS)) setSelectedClip(null);
  }, [ordered, selectedClip]);

  const hasClipSelection =
    selectedClip !== null && editor.splits.some((s) => Math.abs(s - selectedClip) < EPS);

  // Keyboard: Delete cuts, ⌘Z undoes, ⇧⌘Z redoes, Space plays, Esc clears, arrows move.
  useEffect(() => {
    if (!projectId) return;
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (
        overdubOpen ||
        titleDialog ||
        captionRange ||
        agentDialogOpen ||
        brollRange ||
        audioDialog
      )
        return;
      if (target?.closest('input, textarea, select, [contenteditable]')) return;
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        // A selected card, a selected clip and a word selection never
        // coexist, so this is unambiguous: Delete removes whichever one is
        // showing.
        if (selectedTitle !== null) {
          edit({ type: 'removeTitle', at: selectedTitle });
          setSelectedTitle(null);
        } else if (hasClipSelection && selectedClip !== null) {
          edit({ type: 'unsplit', at: selectedClip });
          setSelectedClip(null);
        } else {
          edit({ type: 'deleteSelection' });
        }
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z') {
        e.preventDefault();
        undoRedo(e.shiftKey ? 'redo' : 'undo');
      } else if (e.key === ' ') {
        e.preventDefault();
        playback.toggle();
      } else if (e.key === 'Escape') {
        dispatch({ type: 'clearSelection' });
        setSelectedTitle(null);
        setSelectedClip(null);
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
  }, [
    projectId,
    overdubOpen,
    titleDialog,
    captionRange,
    agentDialogOpen,
    brollRange,
    audioDialog,
    selectedTitle,
    selectedClip,
    hasClipSelection,
    playback,
    showCuts,
    edit,
    undoRedo,
  ]);

  const wordsIn = (range: [number, number] | null) =>
    range
      ? editor.words
          .slice(range[0], range[1] + 1)
          .map((w) => w.text)
          .join(' ')
      : '';
  const selectedText = wordsIn(selected);
  // The dialog keeps showing the words it was opened on, selection or not.
  const captionText = wordsIn(captionRange);

  if (user === undefined)
    return (
      <div className="app">
        <p className="muted">Loading…</p>
      </div>
    );
  if (!user)
    return (
      <div className="app">
        <Login needsSetup={needsSetup} onSignedIn={setUser} />
      </div>
    );

  return (
    <div className="app">
      <header>
        {project && (
          <button type="button" className="ghost back" onClick={onBack} title="Back to samples">
            <span aria-hidden>←</span> Home
          </button>
        )}
        <h1>
          {project ? (
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
        {project && <span className="header-file muted">{project.media.filename}</span>}
        {project && <Presence peers={peers} status={status} lastError={lastError} />}
        <Avatar user={user} withName size="sm" />
        <button type="button" className="ghost" onClick={() => setAgentDialogOpen(true)}>
          Connect an agent
        </button>
        <button type="button" className="ghost" onClick={signOut}>
          Sign out
        </button>
      </header>

      {!project ? (
        <Dropzone onFile={onFile} onLibraryClip={onLibraryClip} busy={busy} error={loadError}>
          <Projects items={projects} onOpen={onOpenProject} disabled={busy !== null} />
        </Dropzone>
      ) : (
        <main className="editor">
          {loadError && (
            <p className="error banner">
              {loadError}
              <button
                type="button"
                className="ghost"
                onClick={() => setLoadError(null)}
                aria-label="Dismiss"
                title="Dismiss"
              >
                ✕
              </button>
            </p>
          )}
          <section className="stage">
            <Player
              media={project.media}
              thumbs={thumbs}
              mediaRef={mediaRef}
              edits={editor.edits}
              assets={assets}
              words={editor.words}
              playback={playback}
              peers={peers}
              transition={editor.transition}
            />
            <Toolbar
              hasSelection={selected !== null}
              hasTitleSelection={selectedTitle !== null}
              hasClipSelection={hasClipSelection}
              canUndo={editor.undoable !== null}
              canRedo={editor.redoable !== null}
              readOnly={!canEdit}
              fillerCount={fillers.length}
              pauseCount={pauses.length}
              twoWordFillers={twoWordFillers}
              showCuts={showCuts}
              exportState={exportState}
              transition={editor.transition}
              onDelete={() => {
                if (selectedTitle !== null) {
                  edit({ type: 'removeTitle', at: selectedTitle });
                  setSelectedTitle(null);
                } else if (hasClipSelection && selectedClip !== null) {
                  edit({ type: 'unsplit', at: selectedClip });
                  setSelectedClip(null);
                } else {
                  edit({ type: 'deleteSelection' });
                }
              }}
              onAddTitle={onAddTitle}
              onAddCaption={() => {
                if (selected) setCaptionRange(selected);
              }}
              onSplit={onSplit}
              onAddBroll={() => {
                if (selected) setBrollRange(selected);
              }}
              onAddMusic={() => setAudioDialog({ range: selected })}
              onTransition={(transition: Transition) => edit({ type: 'setTransition', transition })}
              onRemoveFillers={() => edit({ type: 'applyCuts', cuts: fillers })}
              onTightenPauses={() => edit({ type: 'applyCuts', cuts: pauses })}
              onTwoWordFillers={setTwoWordFillers}
              onShowCuts={setShowCuts}
              onOverdub={() => setOverdubOpen(true)}
              onUndo={() => undoRedo('undo')}
              onRedo={() => undoRedo('redo')}
              onExport={onExport}
            />
          </section>
          <section className="script">
            <ClipStrip
              ordered={ordered}
              words={editor.words}
              thumbs={thumbs}
              selected={selectedClip}
              readOnly={!canEdit}
              onSelect={onClipClick}
              onMove={(piece, before) => edit({ type: 'moveClip', piece, before })}
            />
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
              selectedTitle={selectedTitle}
              onTitleClick={onTitleClick}
              onTitleOpen={onTitleOpen}
              onCaptionClick={(start) => edit({ type: 'removeCaption', start })}
              onCutTransition={(start, transition) =>
                edit({ type: 'setCutTransition', start, transition })
              }
              speakers={speakers}
              speakerNames={editor.speakerNames}
              onRenameSpeaker={onRenameSpeaker}
              readOnly={!canEdit}
              peers={peers}
              ordered={ordered}
              splits={editor.splits}
              selectedClip={selectedClip}
              onClipClick={onClipClick}
              assets={assets}
              onBrollClick={(start) => edit({ type: 'removeBroll', start })}
              onAudioClick={(start) => {
                const found = editor.edits.find(
                  (e): e is AudioEdit => e.kind === 'audio' && Math.abs(e.start - start) < EPS,
                );
                if (found) setAudioDialog({ edit: found });
              }}
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

      {titleDialog && (
        <TitleDialog
          at={titleDialog.at}
          initial={titleDialog.initial}
          onSubmit={onTitleSubmit}
          onCancel={() => setTitleDialog(null)}
        />
      )}

      {agentDialogOpen && <AgentDialog onCancel={() => setAgentDialogOpen(false)} />}

      {captionRange && (
        <CaptionDialog
          original={captionText}
          onSubmit={onCaptionSubmit}
          onCancel={() => setCaptionRange(null)}
        />
      )}

      {brollRange && (
        <BrollDialog
          assets={assets}
          original={wordsIn(brollRange)}
          rangeLength={(() => {
            const r = rangeForWords(editor.words, brollRange[0], brollRange[1], editor.duration);
            return r.end - r.start;
          })()}
          onUpload={onUploadAsset}
          onSubmit={(asset, offset) => {
            edit({ type: 'addBroll', media: asset.id, offset, range: brollRange });
            setBrollRange(null);
          }}
          onCancel={() => setBrollRange(null)}
        />
      )}
      {audioDialog && (
        <AudioDialog
          assets={assets}
          initial={'edit' in audioDialog ? audioDialog.edit : undefined}
          wholeEdit={'range' in audioDialog && audioDialog.range === null}
          onUpload={onUploadAsset}
          onSubmit={(asset, gain, duck) => {
            if ('edit' in audioDialog)
              edit({ type: 'editAudio', start: audioDialog.edit.start, gain, duck });
            else if (asset)
              edit({ type: 'addAudio', media: asset.id, gain, duck, range: audioDialog.range });
            setAudioDialog(null);
          }}
          onRemove={
            'edit' in audioDialog
              ? () => {
                  edit({ type: 'removeAudio', start: audioDialog.edit.start });
                  setAudioDialog(null);
                }
              : undefined
          }
          onCancel={() => setAudioDialog(null)}
        />
      )}
    </div>
  );
}

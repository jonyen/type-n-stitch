import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react';

import {
  addSource,
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
  transcribeProject,
  uploadAsset,
  uploadMedia,
  type Suggestions,
} from './api';
import { AgentDialog } from './components/AgentDialog';
import { AudioDialog } from './components/AudioDialog';
import { LayerDialog } from './components/LayerDialog';
import { CaptionDialog } from './components/CaptionDialog';
import { Dropzone } from './components/Dropzone';
import { ImportList } from './components/ImportList';
import { Login } from './components/Login';
import { OverdubDialog } from './components/OverdubDialog';
import { Player } from './components/Player';
import { Projects } from './components/Projects';
import { SelectionToolbar } from './components/SelectionToolbar';
import { Timeline, type OverlayRef } from './components/Timeline';
import { TitleDialog, type TitleFields } from './components/TitleDialog';
import { ToolToolbar } from './components/ToolToolbar';
import { TopBar, type ExportState } from './components/TopBar';
import { Transcript } from './components/Transcript';
import { cx } from './cx';
import { EPS, isJoin, orderedPieces, rangeForWords, titles } from './editlist';
import { editorReducer, initialEditor, selectedRange, type EditorAction } from './editor';
import { byName, importFailures, runImport, type ImportItem } from './importQueue';
import { handledUpstream, shouldIgnoreGlobalKey } from './keyboardGuard';
import { createOpQueue, type OpQueue } from './opQueue';
import { newOpId, opForAction, type ClientOp, type DocState } from './ops';
import { audios, layers, type LayerTrack } from './overlays';
import { type PresenceState } from './realtime';
import { deleteAction } from './selection';
import { useSession } from './session';
import {
  isTranscribing,
  playableSources,
  readyKey,
  sourceViewsOf,
  unlistedSources,
} from './sources';
import { useStitchedMedia } from './stitchedMedia';
import styles from './App.module.css';
import ui from './styles/ui.module.css';
import { defaultSuggestOptions, fillerCuts, pauseCuts, pending } from './suggest';
import { canSplitAt, timelineSegments } from './timeline';
import type { Tool } from './tools';
import type {
  Asset,
  AudioEdit,
  CaptionPos,
  LayerEdit,
  LibraryItem,
  ProjectSummary,
  SourceView,
  TitleEdit,
  Transition,
} from './types';
import { usePlayback } from './usePlayback';
import { holdOrApply, useRealtime, type RemoteDoc } from './useRealtime';
import { useTheme } from './useTheme';
import { useThumbnails } from './useThumbnails';

export function App() {
  const { user, setUser, signOut } = useSession();
  const [theme, setTheme] = useTheme();
  const [needsSetup, setNeedsSetup] = useState(false);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const canEdit = project?.role === 'owner' || project?.role === 'editor';

  const [busy, setBusy] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editor, dispatch] = useReducer(editorReducer, initialEditor);
  // The word range the overdub dialog was opened on; like `captionRange`, a
  // peer clearing the selection must not unmount the dialog under the user
  // (and leave the shortcuts off, since they wait for it to close).
  const [overdubRange, setOverdubRange] = useState<[number, number] | null>(null);
  // The title dialog, adding at `at` or editing `initial`.
  const [titleDialog, setTitleDialog] = useState<{ at: number; initial?: TitleEdit } | null>(null);
  const [agentDialogOpen, setAgentDialogOpen] = useState(false);
  // The word range the caption dialog was opened on. Holding it here keeps
  // the dialog mounted (and the caption anchored) when a peer's `sync` clears
  // the selection while someone is still typing.
  const [captionRange, setCaptionRange] = useState<[number, number] | null>(null);
  // The layer dialog: adding over the word range it was opened on, or
  // changing an existing layer's track, frame or sound.
  const [layerDialog, setLayerDialog] = useState<
    { range: [number, number] } | { edit: LayerEdit } | null
  >(null);
  // The music dialog: adding over `range` (null means the whole edit), or
  // editing an existing `edit`.
  const [audioDialog, setAudioDialog] = useState<
    { range: [number, number] | null } | { edit: AudioEdit } | null
  >(null);
  // A selected title card, by its instant. Exclusive with the word selection.
  const [selectedTitle, setSelectedTitle] = useState<number | null>(null);
  // A selected clip, by its piece's start. Exclusive with the word/title selection.
  const [selectedClip, setSelectedClip] = useState<number | null>(null);
  // A selected layer or music bar. Exclusive with every other selection.
  const [selectedOverlay, setSelectedOverlay] = useState<OverlayRef | null>(null);
  // The timeline's mouse tool, also used by the transcript. Persists until changed.
  const [tool, setTool] = useState<Tool>('select');
  // Viewers stay on Select.
  useEffect(() => {
    if (!canEdit) setTool('select');
  }, [canEdit]);
  const [assets, setAssets] = useState<Asset[]>([]);
  const [exportState, setExportState] = useState<ExportState>({ status: 'idle' });
  const [twoWordFillers, setTwoWordFillers] = useState(false);
  const [showCuts, setShowCuts] = useState(true);
  const [suggestions, setSuggestions] = useState<Suggestions>({ fillers: [], pauses: [] });

  const [speakers, setSpeakers] = useState<(number | null)[] | null>(null);
  // The project's files as the server lists them: names, urls, transcript status.
  const [sourceViews, setSourceViews] = useState<SourceView[]>([]);
  // The ready files the current words cover (a `readyKey`). When the timeline's
  // ready files differ, either way (one finished, or an undo took one away),
  // the stitched words are fetched again.
  const [wordsCover, setWordsCover] = useState('');
  // Bumped to retry a failed words fetch.
  const [wordsRetry, setWordsRetry] = useState(0);
  // The home screen's import, file by file, while it runs.
  const [imports, setImports] = useState<ImportItem[]>([]);
  // Insert → Add video…, file by file. Failed files stay until dismissed.
  const [uploads, setUploads] = useState<ImportItem[]>([]);

  // The output layout for the current edit list, so playback can jump
  // between output pieces and the transcript can draw clip boundaries.
  const ordered = useMemo(
    () => orderedPieces(editor.duration, editor.edits, editor.splits, editor.order),
    [editor.duration, editor.edits, editor.splits, editor.order],
  );
  // The edit in output time: what the timeline draws and the player's clock reads.
  const segments = useMemo(
    () => timelineSegments(editor.duration, editor.edits, editor.splits, editor.order),
    [editor.duration, editor.edits, editor.splits, editor.order],
  );

  // The fold's sources with their files: what the player plays, what the
  // timeline badges and the transcript greys out while it transcribes.
  const playable = useMemo(
    () => playableSources(editor.sources, sourceViews),
    [editor.sources, sourceViews],
  );
  // One <video> behind a stitched clock, so playback stays in stitched time.
  const stitched = useStitchedMedia(playable);
  const mediaRef = useMemo(() => ({ current: stitched }), [stitched]);
  const playback = usePlayback(
    mediaRef,
    editor.words,
    editor.edits,
    editor.duration,
    playable[0]?.url,
    ordered,
    segments,
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

  // The project's uploaded layer and music assets.
  const refreshAssets = useCallback(() => {
    if (!projectId) return;
    listAssets(projectId)
      .then(setAssets)
      .catch(() => setAssets([]));
  }, [projectId]);
  // Media ids we have already gone looking for and not found, so a layer or
  // music edit naming an id the server does not have cannot loop the fetch.
  const soughtAssets = useRef(new Set<string>());
  useEffect(() => {
    setAssets([]);
    soughtAssets.current.clear();
    refreshAssets();
  }, [refreshAssets]);
  // A peer can upload a clip and use it before we have ever listed the
  // project's assets, which would draw the missing-file placeholder until the
  // next reload. Any applied fold that names an id we do not hold refetches
  // the list once; ids still unknown after that are remembered, so the set of
  // unknown ids has to actually change before we ask again.
  useEffect(() => {
    const have = new Set([...assets.map((a) => a.id), ...playable.map((s) => s.mediaId)]);
    const unknown = [...layers(editor.edits), ...audios(editor.edits)]
      .map((e) => e.media)
      .filter((id) => !have.has(id) && !soughtAssets.current.has(id));
    if (unknown.length === 0) return;
    for (const id of unknown) soughtAssets.current.add(id);
    refreshAssets();
  }, [editor.edits, assets, playable, refreshAssets]);
  const onUploadAsset = useCallback(
    async (file: File) => {
      if (!projectId) throw new Error('no project');
      const asset = await uploadAsset(projectId, file);
      setAssets((list) => [...list, asset]);
      return asset;
    },
    [projectId],
  );

  // Insert → Add video…: append at the end, one file at a time. The timeline
  // grows as each upload lands; the fold (broadcast, or the refetch below) confirms it.
  const onAddVideos = useCallback(
    async (files: File[]) => {
      if (!projectId || !canEdit || files.length === 0) return;
      const append = async (id: string, file: File, onProgress: (f: number) => void) => {
        const view = await addSource(id, file, onProgress);
        dispatch({
          type: 'addSource',
          media: view.mediaId,
          offset: view.offset,
          duration: view.duration,
        });
        setSourceViews((list) =>
          [...list.filter((v) => v.index !== view.index), view].sort((a, b) => a.index - b.index),
        );
        return view;
      };
      const result = await runImport(byName(files), { append }, setUploads, projectId);
      setUploads(result.items.filter((item) => item.status === 'error'));
      try {
        const fetched = await fetchProject(projectId);
        setSourceViews(sourceViewsOf(fetched.project));
        // Only when nothing of ours is in flight: an older fold must not hide an optimistic edit.
        if (queue.current?.pending === 0) settle(fetched.doc);
      } catch (err) {
        setLoadError(err instanceof Error ? err.message : String(err));
      }
    },
    [projectId, canEdit, settle],
  );

  // While any file is still transcribing, poll the project's list (like the
  // export poll). Only the list is updated, so a poll never clears a selection.
  const waiting = sourceViews.some(isTranscribing);
  useEffect(() => {
    if (!projectId || !waiting) return;
    let cancelled = false;
    const timer = setInterval(() => {
      fetchProject(projectId)
        .then(({ project: fresh }) => {
          if (cancelled) return;
          const views = sourceViewsOf(fresh);
          setSourceViews(views);
          // `pending` means nothing has started it (the server restarted since
          // it was added). /transcribe starts it and answers without waiting.
          if (views.some((v) => v.transcript === 'pending'))
            void transcribeProject(projectId).catch(() => undefined);
        })
        .catch(() => {
          // Try again on the next tick.
        });
    }, 2000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [projectId, waiting]);

  // A peer's AddSource names a file we have not listed: fetch the list once.
  const unlisted = unlistedSources(editor.sources, sourceViews).join(',');
  useEffect(() => {
    if (!projectId || !unlisted) return;
    let cancelled = false;
    fetchProject(projectId)
      .then(({ project: fresh }) => {
        if (!cancelled) setSourceViews(sourceViewsOf(fresh));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [projectId, unlisted]);

  // The timeline's ready files are not the ones the words cover: fetch the
  // stitched words again. The server stitches its fold's ready files, so an
  // undone file's words go and a finished file's words arrive.
  const readyNow = readyKey(sourceViews, editor.sources);
  useEffect(() => {
    if (!projectId || readyNow === wordsCover) return;
    let cancelled = false;
    let retry: ReturnType<typeof setTimeout> | undefined;
    transcribeProject(projectId)
      .then(({ words: list, sources: fresh }) => {
        if (cancelled) return;
        // The words cover exactly the sources this reply calls ready.
        if (fresh) setSourceViews(fresh);
        setWordsCover(fresh ? readyKey(fresh) : readyNow);
        dispatch({ type: 'setWords', words: list });
      })
      .catch(() => {
        // A background refresh: the editor still works. Try again shortly.
        if (!cancelled) retry = setTimeout(() => setWordsRetry((n) => n + 1), 2000);
      });
    return () => {
      cancelled = true;
      clearTimeout(retry);
    };
  }, [projectId, readyNow, wordsCover, wordsRetry]);

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
    stitched.pause();
    setProject(null);
    setLoadError(null);
    confirmed.current = null;
    heldRemote.current = null;
    dispatch({ type: 'load', words: [], duration: 0 });
    setExportState({ status: 'idle' });
    setSelectedTitle(null);
    setSelectedClip(null);
    setSelectedOverlay(null);
    setTitleDialog(null);
    setOverdubRange(null);
    setCaptionRange(null);
    setLayerDialog(null);
    setAudioDialog(null);
    setSourceViews([]);
    setWordsCover('');
    setUploads([]);
    setTool('select');
  }, [stitched]);

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
      const [transcript, fetched] = await Promise.all([
        transcribeProject(summary.id),
        fetchProject(summary.id),
      ]);
      const { words } = transcript;
      dispatch({ type: 'load', words, duration: summary.media.duration, media: summary.media.id });
      dispatch({ type: 'sync', doc: fetched.doc });
      confirmed.current = fetched.doc;
      const views = sourceViewsOf(fetched.project);
      setSourceViews(views);
      // The words cover exactly the sources the transcribe reply calls ready.
      setWordsCover(readyKey(transcript.sources ?? views));
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

  // Several files make one project: the first creates it, the rest are
  // appended in the order chosen. Failures are reported once it opens.
  const onImport = useCallback(
    async (files: File[]) => {
      setLoadError(null);
      setBusy(
        files.length === 1
          ? `Uploading ${files[0]?.name ?? ''}`
          : `Importing ${files.length} files`,
      );
      const result = await runImport(files, { create: uploadMedia, append: addSource }, setImports);
      const failed = importFailures(result.items);
      setImports([]);
      const created = result.project;
      if (!created) {
        setBusy(null);
        setLoadError(failed ?? 'Nothing was imported.');
        return;
      }
      await load(`Opening ${created.title}`, () => Promise.resolve(created));
      if (failed) setLoadError(failed);
    },
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
  }, [projectId, words]);

  const onRenameSpeaker = useCallback(
    (speaker: number, name: string) => edit({ type: 'renameSpeaker', speaker, name }),
    [edit],
  );

  const onWordClick = useCallback(
    (index: number, extend: boolean) => {
      const word = editor.words[index];
      if (tool === 'razor') {
        if (word && canEdit && canSplitAt(word.start, editor.edits, editor.splits, editor.duration))
          edit({ type: 'split', at: word.start });
        return;
      }
      setSelectedTitle(null);
      setSelectedClip(null);
      setSelectedOverlay(null);
      dispatch({ type: 'select', index, extend });
      if (word && !extend) playback.seek(word.start);
    },
    [tool, canEdit, editor.words, editor.edits, editor.splits, editor.duration, edit, playback],
  );

  // The Range tool cuts the dragged words on release; a plain click cuts nothing.
  const onWordDragEnd = useCallback(() => {
    if (tool === 'range') edit({ type: 'deleteSelection' });
  }, [tool, edit]);

  // Razor splits on a press; dragging across words selects nothing.
  const onWordDrag = useCallback(
    (index: number) => {
      if (tool === 'razor') return;
      dispatch({ type: 'select', index, extend: true });
    },
    [tool],
  );

  const headSeq = editor.headSeq;
  const onExport = useCallback(async () => {
    if (!projectId) return;
    // The version being rendered, so a result can tell when the edit moved on.
    const seq = headSeq;
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
            seq,
          });
          return;
        } else {
          setExportState({ status: 'error', message: job.message, seq });
          return;
        }
      }
    } catch (err) {
      setExportState({
        status: 'error',
        message: err instanceof Error ? err.message : String(err),
        seq,
      });
    }
  }, [projectId, headSeq]);

  const onOverdubSubmit = useCallback(
    async (text: string) => {
      if (!projectId || !overdubRange) return;
      const { audioUrl, duration } = await synthesizeOverdub(projectId, text);
      edit({ type: 'overdub', text, audioUrl, audioDuration: duration, range: overdubRange });
      setOverdubRange(null);
    },
    [projectId, overdubRange, edit],
  );

  /** Where a new card goes: the end of the selected words, else the playhead. */
  const onAddTitle = useCallback(() => {
    const at = selected
      ? rangeForWords(editor.words, selected[0], selected[1], editor.duration, editor.sources).end
      : playback.currentTime;
    setTitleDialog({ at });
  }, [selected, editor.words, editor.duration, editor.sources, playback.currentTime]);

  // A music bar, from the transcript tag or the timeline, opens its dialog.
  const openAudio = useCallback(
    (start: number) => {
      const found = audios(editor.edits).find((a) => Math.abs(a.start - start) < EPS);
      if (found) setAudioDialog({ edit: found });
    },
    [editor.edits],
  );

  // A layer bar's double-click opens its dialog to change track, frame or sound.
  const openLayer = useCallback(
    (track: LayerTrack, start: number) => {
      const found = layers(editor.edits).find(
        (l) => l.track === track && Math.abs(l.start - start) < EPS,
      );
      if (found) setLayerDialog({ edit: found });
    },
    [editor.edits],
  );

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
    setSelectedOverlay(null);
    dispatch({ type: 'clearSelection' });
  }, []);

  const onSelectOverlay = useCallback((ref: OverlayRef | null) => {
    setSelectedOverlay(ref);
    if (ref) {
      setSelectedTitle(null);
      setSelectedClip(null);
      dispatch({ type: 'clearSelection' });
    }
  }, []);

  // Clicking any divider jumps there; only a split's divider can be
  // "selected" (Delete then joins it to the clip before). A cut-derived
  // boundary just seeks, since there's no split for Delete to undo.
  const onClipClick = useCallback(
    (start: number) => {
      // A join between two files is a boundary, never a split Delete can remove.
      const split =
        editor.splits.some((s) => Math.abs(s - start) < EPS) && !isJoin(start, editor.sources);
      setSelectedClip(split ? start : null);
      setSelectedTitle(null);
      setSelectedOverlay(null);
      dispatch({ type: 'clearSelection' });
      playback.seek(start);
      document
        .querySelector<HTMLElement>(`[data-clip-start="${start}"]`)
        ?.scrollIntoView({ block: 'start', behavior: 'smooth' });
    },
    [playback, editor.splits, editor.sources],
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

  // A peer's edit (or an undo) can remove the layer or music we had selected.
  useEffect(() => {
    if (!selectedOverlay) return;
    const still =
      selectedOverlay.kind === 'layer'
        ? layers(editor.edits).some(
            (l) =>
              l.track === selectedOverlay.track && Math.abs(l.start - selectedOverlay.start) < EPS,
          )
        : audios(editor.edits).some((a) => Math.abs(a.start - selectedOverlay.start) < EPS);
    if (!still) setSelectedOverlay(null);
  }, [editor.edits, selectedOverlay]);

  const hasClipSelection =
    selectedClip !== null &&
    editor.splits.some((s) => Math.abs(s - selectedClip) < EPS) &&
    !isJoin(selectedClip, editor.sources);

  // A word selection made any other way (the arrow keys, a peer's sync) ends
  // the title, clip and overlay selections, as clicking a word does.
  const hasWordSelection = selected !== null;
  useEffect(() => {
    if (!hasWordSelection) return;
    setSelectedTitle(null);
    setSelectedClip(null);
    setSelectedOverlay(null);
  }, [hasWordSelection]);

  /** Delete whatever is selected: words, a title card, a split, or a layer or music bar. */
  const deleteSelected = useCallback(() => {
    edit(
      deleteAction({
        hasWords: hasWordSelection,
        title: selectedTitle,
        clip: hasClipSelection ? selectedClip : null,
        overlay: selectedOverlay,
      }),
    );
    setSelectedTitle(null);
    setSelectedClip(null);
    setSelectedOverlay(null);
  }, [hasWordSelection, selectedTitle, hasClipSelection, selectedClip, selectedOverlay, edit]);

  /** Escape: no word, title, clip or overlay selection. The floating toolbar's dismiss too. */
  const clearAll = useCallback(() => {
    dispatch({ type: 'clearSelection' });
    setSelectedTitle(null);
    setSelectedClip(null);
    setSelectedOverlay(null);
  }, []);

  // Keyboard: Delete cuts, ⌘Z undoes, ⇧⌘Z redoes, Space plays, Esc clears, arrows move.
  // The window listener is added once per project and calls the latest
  // handler through a ref. Re-adding it on every render (`playback` is a new
  // object each time) dropped keys: a listener removed mid-dispatch is not
  // called, and a render can land between two listeners of one real keydown,
  // e.g. after the tool picker's own Escape switches back to Select.
  const keyHandler = useRef<((e: KeyboardEvent) => void) | null>(null);
  useEffect(() => {
    if (!projectId) return;
    const onKey = (e: KeyboardEvent) => keyHandler.current?.(e);
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [projectId]);
  useLayoutEffect(() => {
    keyHandler.current = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (
        overdubRange ||
        titleDialog ||
        captionRange ||
        agentDialogOpen ||
        layerDialog ||
        audioDialog
      )
        return;
      if (shouldIgnoreGlobalKey(target, handledUpstream(e))) return;
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        deleteSelected();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z') {
        e.preventDefault();
        undoRedo(e.shiftKey ? 'redo' : 'undo');
      } else if (e.key === ' ') {
        e.preventDefault();
        playback.toggle();
      } else if (e.key === 'Escape') {
        clearAll();
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
  }, [
    overdubRange,
    titleDialog,
    captionRange,
    agentDialogOpen,
    layerDialog,
    audioDialog,
    deleteSelected,
    clearAll,
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
  // The dialog keeps showing the words it was opened on, selection or not.
  const captionText = wordsIn(captionRange);
  // What the layer dialog describes: the words its layer covers, and their length.
  const layerWords = (() => {
    if (!layerDialog) return { text: '', length: 0 };
    if ('edit' in layerDialog) {
      const { start, end } = layerDialog.edit;
      const text = editor.words
        .filter((w) => w.start >= start - EPS && w.start < end - EPS)
        .map((w) => w.text)
        .join(' ');
      return { text, length: end - start };
    }
    const [from, to] = layerDialog.range;
    const r = rangeForWords(editor.words, from, to, editor.duration, editor.sources);
    return { text: wordsIn(layerDialog.range), length: r.end - r.start };
  })();

  if (user === undefined)
    return (
      <div className={styles.app}>
        <p className={ui.muted}>Loading…</p>
      </div>
    );
  if (!user) return <Login needsSetup={needsSetup} onSignedIn={setUser} />;

  const controls = project
    ? {
        readOnly: !canEdit,
        canUndo: editor.undoable !== null,
        canRedo: editor.redoable !== null,
        onUndo: () => undoRedo('undo'),
        onRedo: () => undoRedo('redo'),
        fillerCount: fillers.length,
        pauseCount: pauses.length,
        twoWordFillers,
        onRemoveFillers: () => edit({ type: 'applyCuts', cuts: fillers }),
        onTightenPauses: () => edit({ type: 'applyCuts', cuts: pauses }),
        onTwoWordFillers: setTwoWordFillers,
        hasSelection: selected !== null,
        onAddTitle,
        onAddCaption: () => {
          if (selected) setCaptionRange(selected);
        },
        onAddLayer: () => {
          if (selected) setLayerDialog({ range: selected });
        },
        onAddMusic: () => setAudioDialog({ range: selected }),
        onAddVideos: (files: File[]) => void onAddVideos(files),
        addingVideos: uploads.some((u) => u.status === 'queued' || u.status === 'uploading'),
        onOverdub: () => {
          if (selected) setOverdubRange(selected);
        },
        onSplit,
        transition: editor.transition,
        onTransition: (transition: Transition) => edit({ type: 'setTransition', transition }),
        peers,
        status,
        lastError,
        exportState,
        headSeq: editor.headSeq,
        onExport: () => void onExport(),
      }
    : null;

  return (
    <div className={styles.app}>
      <TopBar
        user={user}
        project={project}
        editor={controls}
        theme={theme}
        onTheme={setTheme}
        onHome={onBack}
        onAgent={() => setAgentDialogOpen(true)}
        onSignOut={signOut}
      />

      {!project ? (
        <div className={styles.home}>
          <Dropzone
            onImport={(files) => void onImport(files)}
            onLibraryClip={onLibraryClip}
            busy={busy}
            error={loadError}
            imports={imports}
          >
            <Projects items={projects} onOpen={onOpenProject} disabled={busy !== null} />
          </Dropzone>
        </div>
      ) : (
        <main className={styles.editor}>
          {loadError && (
            <p className={styles.banner} role="alert">
              {loadError}
              <button
                type="button"
                className={cx(ui.iconButton, ui.ghost)}
                onClick={() => setLoadError(null)}
                aria-label="Dismiss"
              >
                ✕
              </button>
            </p>
          )}
          <section className={styles.viewer} aria-label="Viewer">
            {uploads.length > 0 && (
              <div className={styles.uploads}>
                <ImportList items={uploads} />
                {uploads.every((u) => u.status === 'error') && (
                  <button
                    type="button"
                    className={cx(ui.button, ui.ghost)}
                    onClick={() => setUploads([])}
                  >
                    Dismiss
                  </button>
                )}
              </div>
            )}
            <Player
              sources={playable}
              stitched={stitched}
              edits={editor.edits}
              assets={assets}
              words={editor.words}
              playback={playback}
              segments={segments}
              transition={editor.transition}
            />
          </section>
          <section className={styles.script} aria-label="Transcript">
            <div className={styles.scriptHead}>
              <div className={styles.scriptHeadRow}>
                <span>Transcript</span>
                <label
                  className={ui.toggle}
                  title="Off: read the transcript as the output will sound"
                >
                  <input
                    type="checkbox"
                    checked={showCuts}
                    onChange={(e) => setShowCuts(e.target.checked)}
                  />
                  Show cuts
                </label>
              </div>
              <span className={styles.scriptHint}>
                click a word to seek · shift-click to extend · space plays
              </span>
            </div>
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
              onLayerClick={(track, start) => onSelectOverlay({ kind: 'layer', track, start })}
              onAudioClick={openAudio}
              sources={playable}
              tool={tool}
              onWordDragEnd={onWordDragEnd}
            />
            <SelectionToolbar
              anchorIndex={selected?.[0] ?? null}
              titleAt={selectedTitle}
              clipStart={hasClipSelection ? selectedClip : null}
              overlay={selectedOverlay}
              open={
                (selected !== null ||
                  selectedTitle !== null ||
                  hasClipSelection ||
                  selectedOverlay !== null) &&
                canEdit &&
                tool === 'select'
              }
              onDelete={deleteSelected}
              onOverdub={() => {
                if (selected) setOverdubRange(selected);
              }}
              onCaption={() => {
                if (selected) setCaptionRange(selected);
              }}
              onLayer={() => {
                if (selected) setLayerDialog({ range: selected });
              }}
              onDismiss={clearAll}
            />
          </section>
          <section className={styles.dock} aria-label="Timeline">
            <ToolToolbar
              tool={tool}
              onChange={setTool}
              readOnly={!canEdit}
              shortcuts={
                !(
                  overdubRange ||
                  titleDialog ||
                  captionRange ||
                  agentDialogOpen ||
                  layerDialog ||
                  audioDialog
                )
              }
            />
            <Timeline
              words={editor.words}
              edits={editor.edits}
              assets={assets}
              ordered={ordered}
              segments={segments}
              outputTime={playback.outputTime}
              peers={peers}
              thumbs={thumbs}
              readOnly={!canEdit}
              selectedClip={selectedClip}
              selectedOverlay={selectedOverlay}
              onSeek={playback.seekOutput}
              onSelectClip={onClipClick}
              onMoveClip={(piece, before) => edit({ type: 'moveClip', piece, before })}
              onSelectOverlay={onSelectOverlay}
              onOpenAudio={openAudio}
              onOpenLayer={openLayer}
              tool={tool}
              duration={editor.duration}
              splits={editor.splits}
              onSplit={(at) => edit({ type: 'split', at })}
              onCut={(ranges) =>
                edit({
                  type: 'applyCuts',
                  cuts: ranges.map((r) => ({ kind: 'cut' as const, ...r })),
                })
              }
              sources={playable}
            />
          </section>
        </main>
      )}

      {overdubRange && (
        <OverdubDialog
          original={wordsIn(overdubRange)}
          onSubmit={onOverdubSubmit}
          onCancel={() => setOverdubRange(null)}
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

      {layerDialog && (
        <LayerDialog
          assets={assets}
          sources={playable}
          initial={'edit' in layerDialog ? layerDialog.edit : undefined}
          original={layerWords.text}
          rangeLength={layerWords.length}
          onUpload={onUploadAsset}
          onSubmit={(choice) => {
            if ('edit' in layerDialog) {
              const { track, start } = layerDialog.edit;
              edit({
                type: 'setLayer',
                track,
                start,
                toTrack: choice.track,
                frame: choice.frame,
                audio: choice.audio,
              });
              // Keep the bar selected when it moves to the other track.
              if (
                selectedOverlay?.kind === 'layer' &&
                selectedOverlay.track === track &&
                Math.abs(selectedOverlay.start - start) < EPS
              )
                setSelectedOverlay({ kind: 'layer', track: choice.track, start });
            } else {
              edit({
                type: 'addLayer',
                track: choice.track,
                media: choice.media,
                offset: choice.offset,
                frame: choice.frame,
                audio: choice.audio,
                range: layerDialog.range,
              });
            }
            setLayerDialog(null);
          }}
          onRemove={
            'edit' in layerDialog
              ? () => {
                  edit({
                    type: 'removeLayer',
                    track: layerDialog.edit.track,
                    start: layerDialog.edit.start,
                  });
                  setLayerDialog(null);
                }
              : undefined
          }
          onCancel={() => setLayerDialog(null)}
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

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import ReactMarkdown from "react-markdown";
import { useLocation, useParams } from "react-router-dom";
import { apiFetch } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import {
  parseFrontmatter,
  stripBlockIds,
  stripVaultNoteLinks,
} from "../lib/markdown";
import { extractMarkdownHeadings, slugifyHeading } from "../lib/noteHeadings";
import { frontmatterLineOffset, linesMatch } from "../lib/sourceMap";
import { useNoteAutosave } from "../hooks/useNoteAutosave";
import { createEditHistory } from "../lib/editHistory";
import {
  createSearchHighlightPlugin,
  normalizeSearchQuery,
  setActiveSearchHit as setActiveSearchHitClass,
} from "../lib/noteSearch";
import { isNoteEqual, isNoteLinksEqual } from "../lib/stateCompare";
import type {
  ActiveNoteMeta,
  ExplorerNote,
  Note,
  NoteLinks,
  NoteLinksResponse,
} from "../types";
import {
  describeWriteOutcome,
  updateNote,
  uploadAttachment,
} from "../api/writeApi";
import {
  clearNoteDraft,
  loadNoteDraft,
  saveNoteDraft,
} from "../lib/writeDrafts";
import { NoteEditor } from "./NoteEditor";
import { NoteSkeleton, StateBlock, StatusBadge, UiButton } from "./ui";
import { SaveState } from "./note-page/SaveState";
import {
  attachmentRejection,
  insertEmbedAt,
  insertionLineForDrop,
  uploadNoteAttachment,
} from "./note-page/attachmentDrop";
import { InlineEditorProvider } from "./note-page/InlineEditorProvider";
import { jumpToHeading, scrollElementIntoView } from "./note-page/dom";
import { NotePreview } from "./note-page/NotePreview";
import { createNoteMarkdownComponents } from "./note-page/renderers";
import {
  NoteLinksPanel,
  NoteProperties,
  NoteTocDesktop,
  NoteTocMobile,
  SearchHitNavigator,
} from "./note-page/sections";
import { useResolvedWikilinks } from "./note-page/wikilinks";

export function NotePage({
  onActiveNoteChange,
  onTagSelect,
  propertiesCollapsedStorageKey,
  vaultRevision,
  writeEnabled,
  editRequestId,
  onWriteNotice,
  noteCandidates = [],
}: {
  onActiveNoteChange: (meta: ActiveNoteMeta | null) => void;
  onTagSelect: (tag: string) => void;
  propertiesCollapsedStorageKey: string;
  vaultRevision: number;
  writeEnabled: boolean;
  editRequestId: number;
  onWriteNotice?: (message: string | null) => void;
  noteCandidates?: ExplorerNote[];
}) {
  const params = useParams<{ slug: string }>();
  const location = useLocation();
  const slug = params.slug ?? "";
  const [note, setNote] = useState<Note | null>(null);
  const [noteLinks, setNoteLinks] = useState<NoteLinks | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isEditing, setIsEditing] = useState(false);
  const [draftContent, setDraftContent] = useState("");
  const [editBaseHash, setEditBaseHash] = useState("");
  const [draftNotice, setDraftNotice] = useState<string | null>(null);
  const [draftStale, setDraftStale] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [conflictNote, setConflictNote] = useState<Note | null>(null);
  const [noteChangedOnDisk, setNoteChangedOnDisk] = useState(false);
  const [editorError, setEditorError] = useState<string | null>(null);
  const [inlineDirty, setInlineDirty] = useState(false);
  const [activeUnit, setActiveUnit] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [propertiesCollapsed, setPropertiesCollapsed] = useState<boolean>(
    () => {
      return window.localStorage.getItem(propertiesCollapsedStorageKey) !== "0";
    },
  );
  const [searchHitCount, setSearchHitCount] = useState(0);
  const [activeSearchHit, setActiveSearchHit] = useState(0);
  const noteBodyRef = useRef<HTMLDivElement | null>(null);
  const searchHitsRef = useRef<HTMLSpanElement[]>([]);
  const currentSlugRef = useRef(slug);
  const lastEditRequestIdRef = useRef(editRequestId);
  const lastHandledRevisionRef = useRef(0);
  const autosaveStatusRef = useRef<string>("idle");
  currentSlugRef.current = slug;

  const loadNote = useCallback(
    async (hardReload: boolean) => {
      setError(null);
      if (hardReload) {
        setNote(null);
      }

      try {
        const res = await apiFetch(`/api/note/${encodeURIComponent(slug)}`);
        if (!res.ok) {
          throw new Error(await readErrorMessage(res, "Failed loading note"));
        }
        const json = (await res.json()) as { note: Note };
        if (slug !== currentSlugRef.current) return;
        setNote((prev) => (isNoteEqual(prev, json.note) ? prev : json.note));
      } catch (err) {
        if (slug !== currentSlugRef.current) return;
        setError(
          err instanceof Error ? err.message : "Unknown note loading error",
        );
      }
    },
    [slug],
  );

  const loadNoteLinks = useCallback(async () => {
    try {
      const res = await apiFetch(`/api/note/${encodeURIComponent(slug)}/links`);
      if (!res.ok) {
        throw new Error(
          await readErrorMessage(res, "Failed loading note links"),
        );
      }
      const json = (await res.json()) as NoteLinksResponse;
      if (slug !== currentSlugRef.current) return;
      setNoteLinks((prev) =>
        isNoteLinksEqual(prev, json.links) ? prev : json.links,
      );
    } catch {
      if (slug !== currentSlugRef.current) return;
      setNoteLinks(null);
    }
  }, [slug]);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      setLoading(true);
      await loadNote(true);
      await loadNoteLinks();
      if (!cancelled) {
        setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [loadNote, loadNoteLinks]);

  useEffect(() => {
    setIsEditing(false);
    setDraftContent("");
    setEditBaseHash("");
    setDraftNotice(null);
    setDraftStale(false);
    setConflict(false);
    setNoteChangedOnDisk(false);
    setEditorError(null);
    setSaving(false);
    setInlineDirty(false);
  }, [slug]);

  useEffect(() => {
    if (
      vaultRevision === 0 ||
      vaultRevision === lastHandledRevisionRef.current
    ) {
      return;
    }
    lastHandledRevisionRef.current = vaultRevision;

    // Never refetch the note out from under an open editor: doing so would move
    // the content hash the editor saves against and silently defeat the
    // optimistic-concurrency guard. Flag the change instead so the user can
    // reload deliberately.
    // D16: our own writes bump the revision twice. Refetching while the
    // document is dirty or a write is in flight would move the hash the next
    // save is made against and defeat the concurrency guard.
    if (isEditing || inlineDirty || autosaveStatusRef.current === "saving") {
      setNoteChangedOnDisk(true);
      return;
    }

    void loadNote(false);
    void loadNoteLinks();
  }, [loadNote, loadNoteLinks, vaultRevision, isEditing, inlineDirty]);

  useEffect(() => {
    window.localStorage.setItem(
      propertiesCollapsedStorageKey,
      propertiesCollapsed ? "1" : "0",
    );
  }, [propertiesCollapsed, propertiesCollapsedStorageKey]);

  const startEditing = useCallback(() => {
    if (!writeEnabled || !note || isEditing) {
      return;
    }

    const storedDraft = loadNoteDraft(note.slug);
    if (storedDraft && storedDraft.content !== note.content) {
      const stale = storedDraft.baseContentHash !== note.content_hash;
      setDraftContent(storedDraft.content);
      // Save against the version the draft was actually based on. If the note
      // moved on disk since, the server will reject the save (409) and the user
      // is prompted to reload rather than silently overwriting newer content.
      setEditBaseHash(storedDraft.baseContentHash);
      setDraftStale(stale);
      setDraftNotice(
        stale
          ? "Restored an earlier draft based on a previous version of this note. Reload the latest version before saving to avoid overwriting newer changes."
          : "Restored your unsaved draft for this note.",
      );
    } else {
      setDraftContent(note.content);
      setEditBaseHash(note.content_hash);
      setDraftStale(false);
      setDraftNotice(null);
    }
    setConflict(false);
    setNoteChangedOnDisk(false);
    setEditorError(null);
    setSaving(false);
    setIsEditing(true);
  }, [isEditing, note, writeEnabled]);

  useEffect(() => {
    if (editRequestId === lastEditRequestIdRef.current) {
      return;
    }

    lastEditRequestIdRef.current = editRequestId;
    startEditing();
  }, [editRequestId, startEditing]);

  useEffect(() => {
    if (!isEditing || !note) {
      return;
    }

    saveNoteDraft(note.slug, {
      slug: note.slug,
      content: draftContent,
      baseContentHash: editBaseHash || note.content_hash,
      savedAt: Date.now(),
    });
  }, [draftContent, editBaseHash, isEditing, note]);

  const parsed = useMemo(() => parseFrontmatter(note?.content ?? ""), [note]);

  useEffect(() => {
    if (!note) {
      onActiveNoteChange(null);
      return;
    }

    onActiveNoteChange({
      title: note.title,
      slug: note.slug,
      relativePath: note.relative_path,
      exportContent: stripVaultNoteLinks(parsed.body),
      contentHash: note.content_hash,
    });
  }, [note, onActiveNoteChange, parsed.body]);

  const renderInput = stripBlockIds(parsed.body);
  const { resolved: markdown, resolvedFor } = useResolvedWikilinks(
    renderInput,
    note?.relative_path ?? "",
  );
  // While resolution is in flight the rendered tree still describes the
  // previous document, so every block range on screen is stale (D28).
  const settling = resolvedFor !== renderInput;
  const searchQuery = useMemo(
    () => normalizeSearchQuery(new URLSearchParams(location.search).get("q")),
    [location.search],
  );
  const matchHeading = useMemo(
    () => new URLSearchParams(location.search).get("m"),
    [location.search],
  );
  const tocHeadings = useMemo(
    () => extractMarkdownHeadings(parsed.body),
    [parsed.body],
  );
  const rehypePlugins = useMemo(
    () => [rehypeKatex, createSearchHighlightPlugin(searchQuery)],
    [searchQuery],
  );
  const headingIdsBySourceLine = useMemo(
    () => new Map(tocHeadings.map(({ sourceLine, id }) => [sourceLine, id])),
    [tocHeadings],
  );
  // blockRange addresses blocks by line number, so inline editing is only safe
  // while the rendered body has exactly one line per source line. If a
  // transform ever collapses lines, editing would write to the wrong place and
  // confirm the hash, so the feature turns itself off for that note instead.
  const lineMappingIntact = useMemo(
    () => linesMatch(parsed.body, markdown),
    [parsed.body, markdown],
  );
  const inlineEditingEnabled =
    writeEnabled && !isEditing && lineMappingIntact && !!note;

  const markdownComponents = useMemo(
    () =>
      createNoteMarkdownComponents(
        note?.relative_path ?? "",
        headingIdsBySourceLine,
        { editable: inlineEditingEnabled },
      ),
    [note?.relative_path, headingIdsBySourceLine, inlineEditingEnabled],
  );

  const autosaveRef = useRef<ReturnType<typeof useNoteAutosave> | null>(null);
  // Stable per note: a ref an effect depends on cannot be reassigned, and the
  // history object mutates internally rather than being swapped out.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const history = useMemo(() => createEditHistory(""), [slug]);

  const handleInlineChange = (nextContent: string) => {
    if (!note) {
      return;
    }
    history.record(nextContent, Date.now());
    // Moving between units always ends a run, so undo steps line up with
    // blocks rather than with arbitrary pauses.
    history.breakRun();
    if (!inlineDirty) {
      setEditBaseHash(note.content_hash);
      setInlineDirty(true);
    }
    setDraftContent(nextContent);
    setNote((prev) => (prev ? { ...prev, content: nextContent } : prev));
    autosaveRef.current?.commit(nextContent);
  };

  const autosave = useNoteAutosave({
    content: note?.content ?? "",
    baseHash: note?.content_hash ?? "",
    enabled: inlineEditingEnabled,
    save: async (nextContent, expectedHash) => {
      const outcome = await updateNote(slug, nextContent, expectedHash);
      if (outcome.git_sync_warning) {
        onWriteNotice?.(`Git sync warning: ${outcome.git_sync_warning}`);
      }
      return outcome;
    },
    onSaved: (result) => {
      setNote((prev) =>
        prev && result.content_hash
          ? { ...prev, content_hash: result.content_hash }
          : prev,
      );
      setInlineDirty(false);
    },
  });

  autosaveRef.current = autosave;
  autosaveStatusRef.current = autosave.status;

  // Seed once per note. Without this, undo before the first edit would restore
  // the empty string the history was constructed with and blank the note.
  const seededSlugRef = useRef<string | null>(null);
  useEffect(() => {
    if (note && seededSlugRef.current !== slug) {
      seededSlugRef.current = slug;
      history.reset(note.content);
    }
  }, [note, slug, history]);

  const applyHistory = useCallback((next: string | null) => {
    if (next === null) {
      return;
    }
    setNote((prev) => (prev ? { ...prev, content: next } : prev));
    setDraftContent(next);
    setInlineDirty(true);
    autosaveRef.current?.commit(next);
  }, []);

  useEffect(() => {
    if (!inlineEditingEnabled) {
      return;
    }
    const onKeyDown = (event: KeyboardEvent) => {
      const meta = event.metaKey || event.ctrlKey;
      if (!meta || event.isComposing) {
        return;
      }
      const key = event.key.toLowerCase();
      const isUndo = key === "z" && !event.shiftKey;
      const isRedo = (key === "z" && event.shiftKey) || key === "y";
      if (!isUndo && !isRedo) {
        return;
      }
      // Always prevented: mixing our stack with the browser's native textarea
      // undo produces behaviour neither of them can explain.
      event.preventDefault();
      applyHistory((isUndo ? history.undo() : history.redo())?.content ?? null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [inlineEditingEnabled, applyHistory, history]);

  const handleActiveRangeChange = useCallback(
    (range: { startLine: number; endLine: number } | null) => {
      setActiveUnit(range ? `${range.startLine}:${range.endLine}` : null);
    },
    [],
  );

  const [dropActive, setDropActive] = useState(false);

  const handleBodyDrop = async (event: React.DragEvent<HTMLDivElement>) => {
    setDropActive(false);
    if (!inlineEditingEnabled || !note) {
      return;
    }
    const file = event.dataTransfer.files[0];
    if (!file) {
      return;
    }
    event.preventDefault();

    const rejection = attachmentRejection(file);
    if (rejection) {
      onWriteNotice?.(rejection);
      return;
    }

    // Where it lands is decided before the upload, so the insertion point is
    // the one the user aimed at rather than wherever the page has scrolled to
    // by the time the request comes back.
    const blocks = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(".editable-block"),
    )
      .map((el) => {
        const rect = el.getBoundingClientRect();
        return { el, top: rect.top, bottom: rect.bottom };
      })
      .flatMap(({ el, top, bottom }) => {
        const start = Number(el.dataset.startLine);
        const end = Number(el.dataset.endLine);
        return Number.isFinite(start) && Number.isFinite(end)
          ? [{ startLine: start, endLine: end, top, bottom }]
          : [];
      });
    const line = insertionLineForDrop(blocks, event.clientY);

    try {
      const result = await uploadNoteAttachment(
        file,
        note.relative_path,
        uploadAttachment,
      );
      if (result.gitSyncWarning) {
        onWriteNotice?.(`Git sync warning: ${result.gitSyncWarning}`);
      }
      handleInlineChange(insertEmbedAt(note.content, line, result.embedPath));
    } catch (uploadError) {
      onWriteNotice?.(
        uploadError instanceof Error ? uploadError.message : "Upload failed.",
      );
    }
  };

  const reviewConflict = () => {
    // The conflict review lives in source mode, which already knows how to
    // show the disk version beside the draft.
    setDraftContent(note?.content ?? "");
    setEditBaseHash(editBaseHash || (note?.content_hash ?? ""));
    setConflict(true);
    setIsEditing(true);
    void (async () => {
      try {
        const res = await apiFetch(`/api/note/${encodeURIComponent(slug)}`);
        if (res.ok) {
          const json = (await res.json()) as { note: Note };
          setConflictNote(json.note);
        }
      } catch {
        // The banner already said what happened; source mode still holds the draft.
      }
    })();
  };

  useLayoutEffect(() => {
    const root = noteBodyRef.current;
    if (!root) {
      searchHitsRef.current = [];
      setSearchHitCount(0);
      setActiveSearchHit(0);
      return;
    }

    const hits = Array.from(
      root.querySelectorAll<HTMLSpanElement>("mark.search-hit"),
    );
    searchHitsRef.current = hits;
    setSearchHitCount(hits.length);
    setActiveSearchHit(0);

    if (hits.length > 0) {
      setActiveSearchHitClass(hits, 0);
      scrollElementIntoView(hits[0], { block: "center", inline: "nearest" });
    } else if (matchHeading) {
      const parts = matchHeading.split(" > ");
      const lastSegment = parts[parts.length - 1] ?? matchHeading;
      jumpToHeading(slugifyHeading(lastSegment));
    }

    return () => {
      searchHitsRef.current = [];
    };
    // activeUnit is a dependency because entering a block removes its marks:
    // without recounting, SearchHitNavigator's indices silently shift.
  }, [markdown, note?.slug, searchQuery, matchHeading, activeUnit]);

  useEffect(() => {
    if (searchHitsRef.current.length === 0) {
      return;
    }
    setActiveSearchHitClass(searchHitsRef.current, activeSearchHit);
  }, [activeSearchHit]);

  if (loading) {
    return <NoteSkeleton />;
  }
  if (error && !note) {
    return (
      <StateBlock
        title="Note Unavailable"
        description={error}
        actionLabel="Retry"
        onAction={() => void loadNote(true)}
      />
    );
  }
  if (!note) {
    return (
      <StateBlock title="Not Found" description="This note no longer exists." />
    );
  }

  const handleCancelEditing = () => {
    const isDirty = draftContent !== note.content;
    if (
      isDirty &&
      !window.confirm("Discard your unsaved draft for this note?")
    ) {
      return;
    }

    clearNoteDraft(note.slug);
    setDraftContent(note.content);
    setEditorError(null);
    setDraftNotice(null);
    setDraftStale(false);
    setConflict(false);
    setConflictNote(null);
    setSaving(false);
    setIsEditing(false);

    // If the note changed on disk while we held the editor open, pick up the
    // latest now that the editor is closed.
    if (noteChangedOnDisk) {
      setNoteChangedOnDisk(false);
      setLoading(true);
      void (async () => {
        await loadNote(true);
        await loadNoteLinks();
        setLoading(false);
      })();
    }
  };

  const handleReloadLatest = async () => {
    setSaving(true);
    setEditorError(null);
    try {
      const res = await apiFetch(`/api/note/${encodeURIComponent(note.slug)}`);
      if (!res.ok) {
        throw new Error(await readErrorMessage(res, "Failed loading note"));
      }
      const json = (await res.json()) as { note: Note };
      setNote(json.note);
      setEditBaseHash(json.note.content_hash);
      saveNoteDraft(json.note.slug, {
        slug: json.note.slug,
        content: draftContent,
        baseContentHash: json.note.content_hash,
        savedAt: Date.now(),
      });
      setConflict(false);
      setConflictNote(null);
      setNoteChangedOnDisk(false);
      setDraftStale(false);
      setDraftNotice(
        "Loaded the latest version. Your text is preserved — review it, then Save to apply your changes over the latest.",
      );
    } catch {
      setEditorError(
        "Could not reload the latest version. Check your connection and try again.",
      );
    } finally {
      setSaving(false);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setEditorError(null);

    try {
      const outcome = await updateNote(note.slug, draftContent, editBaseHash);
      clearNoteDraft(note.slug);
      setConflict(false);
      setConflictNote(null);
      setNoteChangedOnDisk(false);
      setDraftStale(false);
      setDraftNotice(null);
      setIsEditing(false);
      setInlineDirty(false);
      onWriteNotice?.(describeWriteOutcome(outcome));
      // Patch the saved content in place so the reader updates instantly without
      // a skeleton flash, then reconcile title/links in the background.
      setNote((prev) =>
        prev
          ? {
              ...prev,
              content: draftContent,
              content_hash: outcome.content_hash ?? prev.content_hash,
            }
          : prev,
      );
      await loadNote(false);
      await loadNoteLinks();
    } catch (saveError) {
      if (saveError instanceof Error && saveError.name === "ConflictError") {
        setConflict(true);
        try {
          const res = await apiFetch(
            `/api/note/${encodeURIComponent(note.slug)}`,
          );
          if (res.ok) {
            const json = (await res.json()) as { note: Note };
            setConflictNote(json.note);
          }
        } catch {
          // The generic conflict error still leaves the draft safe in the editor.
        }
        setEditorError(
          "This note changed on disk since you started editing. Review the disk version against your draft before saving again.",
        );
      } else if (saveError instanceof Error) {
        setEditorError(saveError.message);
      } else {
        setEditorError("Failed saving note.");
      }
    } finally {
      setSaving(false);
      setLoading(false);
    }
  };

  const handleUseConflictDiskVersion = () => {
    if (!conflictNote) {
      return;
    }
    setNote(conflictNote);
    setDraftContent(conflictNote.content);
    setEditBaseHash(conflictNote.content_hash);
    saveNoteDraft(conflictNote.slug, {
      slug: conflictNote.slug,
      content: conflictNote.content,
      baseContentHash: conflictNote.content_hash,
      savedAt: Date.now(),
    });
    setConflict(false);
    setConflictNote(null);
    setNoteChangedOnDisk(false);
    setDraftStale(false);
    setEditorError(null);
    setDraftNotice("Using the disk version. Edit it, then Save when ready.");
  };

  const handleKeepConflictDraft = () => {
    if (!conflictNote) {
      return;
    }
    setNote(conflictNote);
    setEditBaseHash(conflictNote.content_hash);
    saveNoteDraft(conflictNote.slug, {
      slug: conflictNote.slug,
      content: draftContent,
      baseContentHash: conflictNote.content_hash,
      savedAt: Date.now(),
    });
    setConflict(false);
    setConflictNote(null);
    setNoteChangedOnDisk(false);
    setDraftStale(false);
    setEditorError(null);
    setDraftNotice(
      "Keeping your draft against the latest disk version. Review it, then Save again.",
    );
  };

  const handleUploadAttachment = async (file: File): Promise<string> => {
    const result = await uploadNoteAttachment(
      file,
      note.relative_path,
      uploadAttachment,
    );
    if (result.gitSyncWarning) {
      onWriteNotice?.(`Git sync warning: ${result.gitSyncWarning}`);
    }
    return result.embedPath;
  };

  return (
    <div className="note-page-layout">
      <article className="note-content">
        <div className="note-page-heading">
          <h2 className="note-page-title">{note.title}</h2>
          {writeEnabled && !isEditing ? (
            <div className="note-inline-actions">
              <SaveState status={autosave.status} savedAt={autosave.savedAt} />
              <UiButton
                className="close-note note-edit-button"
                onClick={startEditing}
              >
                Edit
              </UiButton>
            </div>
          ) : null}
        </div>
        {error ? <StatusBadge tone="warn" text="Showing cached note" /> : null}
        {autosave.status === "conflict" || autosave.status === "error" ? (
          <div className="write-notice" role="status">
            <div className="write-notice-messages">
              {autosave.status === "conflict"
                ? "Edits aren't saving. This note changed somewhere else."
                : "Edits aren't saving. Hatchdoor could not reach the vault."}
            </div>
            <UiButton className="close-note" onClick={reviewConflict}>
              Review
            </UiButton>
          </div>
        ) : null}
        {writeEnabled && !isEditing && !lineMappingIntact ? (
          <p className="note-editor-notice">
            This note&rsquo;s source and rendered lines don&rsquo;t line up, so
            inline editing is off here. Use Edit to open source mode.
          </p>
        ) : null}
        {searchHitCount > 0 ? (
          <SearchHitNavigator
            totalHits={searchHitCount}
            activeHit={activeSearchHit}
            onSelect={(nextIndex) => {
              setActiveSearchHit(nextIndex);
              const target = searchHitsRef.current[nextIndex];
              scrollElementIntoView(target, {
                block: "center",
                inline: "nearest",
              });
            }}
          />
        ) : null}
        <NoteProperties
          properties={parsed.properties}
          content={note.content}
          editable={inlineEditingEnabled}
          onChange={handleInlineChange}
          collapsed={propertiesCollapsed}
          onToggleCollapsed={() => setPropertiesCollapsed((prev) => !prev)}
          onTagSelect={onTagSelect}
        />
        <NoteLinksPanel links={noteLinks} />
        <NoteTocMobile headings={tocHeadings} />
        {isEditing ? (
          <NoteEditor
            content={draftContent}
            saving={saving}
            error={editorError}
            notice={
              noteChangedOnDisk
                ? "This note changed on disk while you were editing. Reload the latest version before saving to avoid overwriting those changes."
                : draftNotice
            }
            canReload={conflict || noteChangedOnDisk || draftStale}
            noteCandidates={noteCandidates}
            conflictReview={
              conflictNote
                ? {
                    diskContent: conflictNote.content,
                    draftContent,
                    onUseDisk: handleUseConflictDiskVersion,
                    onKeepDraft: handleKeepConflictDraft,
                  }
                : null
            }
            onChange={setDraftContent}
            onSave={handleSave}
            onReload={handleReloadLatest}
            onCancel={handleCancelEditing}
            onUploadAttachment={handleUploadAttachment}
            renderPreview={(value) => (
              <NotePreview content={value} relativePath={note.relative_path} />
            )}
          />
        ) : (
          <div ref={noteBodyRef} className="note-body" dir="auto">
            <div
              className={`note-body-drop${dropActive ? " drag-active" : ""}`}
              onDragOver={(event) => {
                if (
                  inlineEditingEnabled &&
                  event.dataTransfer.types.includes("Files")
                ) {
                  event.preventDefault();
                  setDropActive(true);
                }
              }}
              onDragLeave={() => setDropActive(false)}
              onDrop={(event) => void handleBodyDrop(event)}
            >
              <InlineEditorProvider
                content={note.content}
                frontmatterOffset={frontmatterLineOffset(note.content)}
                writeEnabled={inlineEditingEnabled}
                settling={settling}
                onChange={handleInlineChange}
                onActiveRangeChange={handleActiveRangeChange}
              >
                <ReactMarkdown
                  remarkPlugins={[remarkGfm, remarkMath]}
                  rehypePlugins={rehypePlugins}
                  components={markdownComponents}
                >
                  {markdown}
                </ReactMarkdown>
              </InlineEditorProvider>
            </div>
          </div>
        )}
      </article>

      <NoteTocDesktop headings={tocHeadings} />
    </div>
  );
}

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
import {
  frontmatterLineOffset,
  linesMatch,
  placeholderForBlankRange,
  type LineRange,
} from "../lib/sourceMap";
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
  VaultQualifiedLinks,
  VaultQualifiedNote,
  VaultSummary,
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
import { BlockGap } from "./note-page/BlockGap";
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

const TOUCH_EDIT_HINT_KEY = "hatchdoor.touchEditHintSeen";

/**
 * Whether the primary pointer cannot hover, which is what makes the double tap
 * the entry gesture and the hint worth showing. Guarded because jsdom and older
 * WebKit do not implement matchMedia.
 */
function isCoarsePointer(): boolean {
  return window.matchMedia?.("(pointer: coarse)").matches ?? false;
}

/** Flattens the wire response's per-link `vault_id` (always the note's own —
 * cross-Vault backlinks are ruled out by #62) into the simpler local shape
 * the rest of this page and `stateCompare` already work with. */
function unwrapLinks(wire: VaultQualifiedLinks): NoteLinks {
  return {
    outgoing: wire.outgoing.map((entry) => entry.link),
    backlinks: wire.backlinks.map((entry) => entry.link),
  };
}

export function NotePage({
  onActiveNoteChange,
  onTagSelect,
  propertiesCollapsedStorageKey,
  vaultRevision,
  writeEnabled,
  editRequestId,
  onWriteNotice,
  noteCandidates = [],
  vaults,
}: {
  onActiveNoteChange: (meta: ActiveNoteMeta | null) => void;
  onTagSelect: (tag: string) => void;
  propertiesCollapsedStorageKey: string;
  vaultRevision: number;
  writeEnabled: boolean;
  editRequestId: number;
  onWriteNotice?: (message: string | null) => void;
  noteCandidates?: ExplorerNote[];
  vaults: VaultSummary[];
}) {
  const params = useParams<{ vaultId: string; slug: string }>();
  const location = useLocation();
  const vaultId = params.vaultId ?? "";
  const slug = params.slug ?? "";
  // Exact reads show provenance whenever more than one Vault is enabled
  // (#140) — unlike collection surfaces, independent of the browsing scope:
  // a note's own Vault is never ambiguous just because scope is narrowed
  // elsewhere.
  const vaultName =
    vaults.length > 1
      ? (vaults.find((vault) => vault.vault_id === vaultId)?.name ?? vaultId)
      : undefined;
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
  const [activeRange, setActiveRange] = useState<LineRange | null>(null);
  const [saving, setSaving] = useState(false);
  const [propertiesCollapsed, setPropertiesCollapsed] = useState<boolean>(
    () => {
      return window.localStorage.getItem(propertiesCollapsedStorageKey) !== "0";
    },
  );
  // Entering a block on touch is a double tap, which is invisible: the gutter
  // rule signals "something is here" without saying what gesture reaches it.
  // Shown once per install, on coarse pointers only, and retired as soon as the
  // gesture has demonstrably been learned.
  const [touchEditHintSeen, setTouchEditHintSeen] = useState<boolean>(() => {
    return window.localStorage.getItem(TOUCH_EDIT_HINT_KEY) === "1";
  });
  const [searchHitCount, setSearchHitCount] = useState(0);
  const [activeSearchHit, setActiveSearchHit] = useState(0);
  const noteBodyRef = useRef<HTMLDivElement | null>(null);
  const searchHitsRef = useRef<HTMLSpanElement[]>([]);
  const noteKey = `${vaultId}:${slug}`;
  const currentNoteKeyRef = useRef(noteKey);
  const lastEditRequestIdRef = useRef(editRequestId);
  const lastHandledRevisionRef = useRef(0);
  const autosaveStatusRef = useRef<string>("idle");
  const activeUnitRef = useRef<string | null>(null);
  const latestContentRef = useRef("");
  currentNoteKeyRef.current = noteKey;

  const notePath = `/api/v1/vaults/${encodeURIComponent(vaultId)}/notes/${encodeURIComponent(slug)}`;

  const loadNote = useCallback(
    async (hardReload: boolean) => {
      setError(null);
      if (hardReload) {
        setNote(null);
      }

      try {
        const res = await apiFetch(notePath);
        if (!res.ok) {
          throw new Error(await readErrorMessage(res, "Failed loading note"));
        }
        const json = (await res.json()) as VaultQualifiedNote;
        if (noteKey !== currentNoteKeyRef.current) return;
        setNote((prev) => (isNoteEqual(prev, json.note) ? prev : json.note));
      } catch (err) {
        if (noteKey !== currentNoteKeyRef.current) return;
        setError(
          err instanceof Error ? err.message : "Unknown note loading error",
        );
      }
    },
    [noteKey, notePath],
  );

  const loadNoteLinks = useCallback(async () => {
    try {
      const res = await apiFetch(`${notePath}/links`);
      if (!res.ok) {
        throw new Error(
          await readErrorMessage(res, "Failed loading note links"),
        );
      }
      const json = (await res.json()) as VaultQualifiedLinks;
      const links = unwrapLinks(json);
      if (noteKey !== currentNoteKeyRef.current) return;
      setNoteLinks((prev) => (isNoteLinksEqual(prev, links) ? prev : links));
    } catch {
      if (noteKey !== currentNoteKeyRef.current) return;
      setNoteLinks(null);
    }
  }, [noteKey, notePath]);

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
  }, [noteKey]);

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
    if (isEditing) {
      setNoteChangedOnDisk(true);
      return;
    }

    // Our own writes bump the revision twice, so a bump arriving while a save
    // is in flight or the document is dirty is almost always ours. Flagging it
    // leaves a warning that never clears; a genuine external change is caught
    // by the next bump once things are quiet.
    if (
      inlineDirty ||
      activeUnitRef.current !== null ||
      autosaveStatusRef.current === "saving"
    ) {
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

    const storedDraft = loadNoteDraft(vaultId, note.slug);
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
  }, [isEditing, note, vaultId, writeEnabled]);

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

    saveNoteDraft(vaultId, note.slug, {
      vaultId,
      slug: note.slug,
      content: draftContent,
      baseContentHash: editBaseHash || note.content_hash,
      savedAt: Date.now(),
    });
  }, [draftContent, editBaseHash, isEditing, note, vaultId]);

  const parsed = useMemo(() => parseFrontmatter(note?.content ?? ""), [note]);

  useEffect(() => {
    if (!note) {
      onActiveNoteChange(null);
      return;
    }

    onActiveNoteChange({
      vaultId,
      title: note.title,
      slug: note.slug,
      relativePath: note.relative_path,
      exportContent: stripVaultNoteLinks(parsed.body),
      contentHash: note.content_hash,
    });
  }, [note, onActiveNoteChange, parsed.body, vaultId]);

  const renderInput = stripBlockIds(parsed.body);
  const { resolved: markdown, resolvedFor } = useResolvedWikilinks(
    vaultId,
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

  // Applied after wikilink resolution rather than before it: the resolver is
  // keyed on its input, so editing that input would mark the tree as settling
  // and disable editing for exactly as long as the caret sat on a blank line.
  //
  // The range arrives in file coordinates, and this is the body, so the
  // frontmatter offset comes back off.
  const frontmatterOffset = frontmatterLineOffset(note?.content ?? "");
  const renderedMarkdown = useMemo(
    () =>
      placeholderForBlankRange(
        markdown,
        activeRange
          ? {
              startLine: activeRange.startLine - frontmatterOffset,
              endLine: activeRange.endLine - frontmatterOffset,
            }
          : null,
      ),
    [markdown, activeRange, frontmatterOffset],
  );

  const markdownComponents = useMemo(
    () =>
      createNoteMarkdownComponents(
        vaultId,
        note?.relative_path ?? "",
        headingIdsBySourceLine,
        { editable: inlineEditingEnabled },
      ),
    [
      vaultId,
      note?.relative_path,
      headingIdsBySourceLine,
      inlineEditingEnabled,
    ],
  );

  const autosaveRef = useRef<ReturnType<typeof useNoteAutosave> | null>(null);
  // Stable per note: a ref an effect depends on cannot be reassigned, and the
  // history object mutates internally rather than being swapped out.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const history = useMemo(() => createEditHistory(""), [noteKey]);

  const dismissTouchEditHint = useCallback(() => {
    setTouchEditHintSeen((seen) => {
      if (!seen) {
        window.localStorage.setItem(TOUCH_EDIT_HINT_KEY, "1");
      }
      return true;
    });
  }, []);

  const handleInlineChange = (nextContent: string) => {
    if (!note) {
      return;
    }
    // Readable before React re-renders. A block committed inside an async
    // handler has to be visible to the rest of that handler, which still holds
    // the document this render closed over.
    latestContentRef.current = nextContent;
    // An edit landed, so the gesture has been learned and the hint has done its
    // job. Retiring it here rather than on entry means an accidental double tap
    // does not count as having taught anything.
    dismissTouchEditHint();
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
    baseHash: note?.content_hash ?? "",
    enabled: inlineEditingEnabled,
    save: async (nextContent, expectedHash) =>
      updateNote(vaultId, slug, nextContent, expectedHash),
    onSaved: (result) => {
      setNote((prev) =>
        prev && result.content_hash
          ? { ...prev, content_hash: result.content_hash }
          : prev,
      );
      setInlineDirty(false);
    },
  });

  useEffect(() => {
    autosaveRef.current = autosave;
    autosaveStatusRef.current = autosave.status;
  }, [autosave]);

  // Seed once per note. Without this, undo before the first edit would restore
  // the empty string the history was constructed with and blank the note.
  const seededSlugRef = useRef<string | null>(null);
  useEffect(() => {
    if (note && seededSlugRef.current !== noteKey) {
      seededSlugRef.current = noteKey;
      history.reset(note.content);
    }
  }, [note, noteKey, history]);

  useEffect(() => {
    latestContentRef.current = note?.content ?? "";
  }, [note?.content]);

  const [externalChange, setExternalChange] = useState(0);

  const applyHistory = useCallback((next: string | null) => {
    if (next === null) {
      return;
    }
    // The open block, if any, is seeded from the pre-undo document.
    setExternalChange((n) => n + 1);
    latestContentRef.current = next;
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

  // Text sitting in an open block exists nowhere else, so it is flushed after
  // an idle pause and on the way out of the page rather than waiting for blur.
  const handleInProgressChange = (nextContent: string) => {
    autosaveRef.current?.touch(nextContent);
  };

  const handleActiveRangeChange = useCallback(
    (range: { startLine: number; endLine: number } | null) => {
      const key = range ? `${range.startLine}:${range.endLine}` : null;
      activeUnitRef.current = key;
      setActiveUnit(key);
      setActiveRange(range);
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

    // An open block holds its text nowhere else, and its commit rewrites the
    // whole document from the copy it was seeded with. A drop does not move
    // focus, so left open it would commit after the write below and overwrite
    // it, dropping the embed and orphaning the file that was just uploaded.
    // Blurring commits it synchronously, so everything after this works from
    // one document rather than two.
    const focused = document.activeElement;
    if (
      focused instanceof HTMLElement &&
      event.currentTarget.contains(focused)
    ) {
      focused.blur();
    }

    // Where it lands is decided before the upload, so the insertion point is
    // the one the user aimed at rather than wherever the page has scrolled to
    // by the time the request comes back. The commit above replaces a block's
    // lines in place, so the line numbers collected here still hold.
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
        (uploadFile, targetRelativePath) =>
          uploadAttachment(vaultId, uploadFile, targetRelativePath),
      );
      // Not note.content: that is the document this render closed over, and a
      // block committed above has already moved past it.
      handleInlineChange(
        insertEmbedAt(latestContentRef.current, line, result.embedPath),
      );
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
        const res = await apiFetch(notePath);
        if (res.ok) {
          const json = (await res.json()) as VaultQualifiedNote;
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

    return () => {
      searchHitsRef.current = [];
    };
    // activeUnit is a dependency because entering a block removes its marks:
    // without recounting, SearchHitNavigator's indices silently shift.
  }, [markdown, note?.slug, searchQuery, matchHeading, activeUnit]);

  // Jumping to the first hit is a landing gesture, so it is deliberately not
  // tied to activeUnit the way the recount above is. Entering a block changes
  // the active unit, and scrolling on that would throw the reader back to the
  // top of the note the moment they clicked something near the bottom.
  //
  // Runs after the recount effect, which is what fills searchHitsRef: layout
  // effects fire in declaration order within a commit.
  useLayoutEffect(() => {
    if (!noteBodyRef.current) {
      return;
    }

    const hits = searchHitsRef.current;
    if (hits.length > 0) {
      setActiveSearchHitClass(hits, 0);
      scrollElementIntoView(hits[0], { block: "center", inline: "nearest" });
    } else if (matchHeading) {
      const parts = matchHeading.split(" > ");
      const lastSegment = parts[parts.length - 1] ?? matchHeading;
      jumpToHeading(slugifyHeading(lastSegment));
    }
  }, [markdown, note?.slug, searchQuery, matchHeading]);

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

    clearNoteDraft(vaultId, note.slug);
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
      const res = await apiFetch(notePath);
      if (!res.ok) {
        throw new Error(await readErrorMessage(res, "Failed loading note"));
      }
      const json = (await res.json()) as VaultQualifiedNote;
      setNote(json.note);
      setEditBaseHash(json.note.content_hash);
      saveNoteDraft(vaultId, json.note.slug, {
        vaultId,
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
      const outcome = await updateNote(
        vaultId,
        note.slug,
        draftContent,
        editBaseHash,
      );
      clearNoteDraft(vaultId, note.slug);
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
          const res = await apiFetch(notePath);
          if (res.ok) {
            const json = (await res.json()) as VaultQualifiedNote;
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
    saveNoteDraft(vaultId, conflictNote.slug, {
      vaultId,
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
    saveNoteDraft(vaultId, conflictNote.slug, {
      vaultId,
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
      (uploadFile, targetRelativePath) =>
        uploadAttachment(vaultId, uploadFile, targetRelativePath),
    );
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
        {inlineEditingEnabled && !touchEditHintSeen && isCoarsePointer() ? (
          // The same shell the write notice uses, so the × is a device already
          // established here. A notice that is silently its own dismiss target
          // has no affordance at all on touch, where there is no cursor to
          // change.
          <div className="write-notice touch-edit-hint" role="status">
            <div className="write-notice-messages">
              Double-tap a line to edit it.
            </div>
            <button
              type="button"
              className="write-notice-dismiss"
              aria-label="Dismiss hint"
              onClick={dismissTouchEditHint}
            >
              ×
            </button>
          </div>
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
          vaultName={vaultName}
          content={note.content}
          editable={inlineEditingEnabled}
          onChange={handleInlineChange}
          collapsed={propertiesCollapsed}
          onToggleCollapsed={() => setPropertiesCollapsed((prev) => !prev)}
          onTagSelect={onTagSelect}
        />
        <NoteLinksPanel vaultId={vaultId} links={noteLinks} />
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
              <NotePreview
                vaultId={vaultId}
                vaultName={vaultName}
                content={value}
                relativePath={note.relative_path}
              />
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
                externalChangeSignal={externalChange}
                onChange={handleInlineChange}
                onInProgressChange={handleInProgressChange}
                onActiveRangeChange={handleActiveRangeChange}
              >
                <BlockGap>
                  <ReactMarkdown
                    remarkPlugins={[remarkGfm, remarkMath]}
                    rehypePlugins={rehypePlugins}
                    components={markdownComponents}
                  >
                    {renderedMarkdown}
                  </ReactMarkdown>
                </BlockGap>
              </InlineEditorProvider>
            </div>
          </div>
        )}
      </article>

      <NoteTocDesktop headings={tocHeadings} />
    </div>
  );
}

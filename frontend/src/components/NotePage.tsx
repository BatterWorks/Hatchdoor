import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import ReactMarkdown from "react-markdown";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";
import { apiFetch } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import { noteInSyncConflict } from "../app/vaultSlotLogic";
import {
  parseFrontmatter,
  stripBlockIds,
  stripVaultNoteLinks,
} from "../lib/markdown";
import {
  duplicateTitleHeadingLine,
  extractMarkdownHeadings,
  slugifyHeading,
  type NoteHeading,
} from "../lib/noteHeadings";
import { detectLineEnding, frontmatterLineOffset } from "../lib/sourceMap";
import { useIsMobile } from "../hooks/useIsMobile";
import { useNoteAutosave } from "../hooks/useNoteAutosave";
import { holdAppReload } from "../lib/reloadGuard";
import { safeGetItem, safeSetItem } from "../lib/storage";
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
  VaultId,
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
  listHeldDrafts,
  loadNoteDraft,
  saveNoteDraft,
} from "../lib/writeDrafts";
import { NoteEditor, type UploadedAttachment } from "./NoteEditor";
import type { NoteCandidate } from "../lib/noteCandidates";
import { refreshVaultCollection } from "../vaults";
import { linkStyleOf, noteLinkText } from "./note-page/linkStyle";
import { NoteSkeleton, StateBlock, StatusBadge, UiButton } from "./ui";
import { SaveState } from "./note-page/SaveState";
import {
  attachmentEmbedText,
  uploadNoteAttachment,
  type NoteAttachmentUpload,
} from "./note-page/attachmentDrop";
import { jumpToHeading, scrollElementIntoView } from "./note-page/dom";
import {
  LiveEditor,
  type LiveEditorHandle,
} from "./note-page/live-editor/LiveEditor";
import {
  createNoteLinkResolver,
  wikilinkLabel,
} from "./note-page/live-editor/noteLinks";
import { NotePreview } from "./note-page/NotePreview";
import { createNoteMarkdownComponents } from "./note-page/renderers";
import { SavedQueryProvider } from "./note-page/SavedQueryBlock";
import {
  remarkHideQueryMarkers,
  useSavedQueries,
} from "./note-page/savedQueries";
import {
  NoteLinksPanel,
  NoteProperties,
  NoteTocDesktop,
  NoteTocMobile,
  SearchHitNavigator,
} from "./note-page/sections";
import {
  cachedAssetHref,
  resolveAssetTargets,
  resolveNoteTargets,
  useResolvedWikilinks,
} from "./note-page/wikilinks";

/** The browser remembers whether notes open rendered or in the editor. */
const READING_VIEW_KEY = "hatchdoor.noteReadingView";

/**
 * Trailing debounce on the localStorage draft write (#330). A draft written per
 * keystroke costs a full `JSON.stringify` of the note plus a synchronous
 * `setItem`, which on WebKit is a cross-process call and lands on the typing
 * path of a long note. One write per pause is enough: the page also writes
 * synchronously on the way out, which is the moment the draft exists for.
 */
const DRAFT_WRITE_DEBOUNCE_MS = 700;

/**
 * Browsers cap the total body of in-flight `keepalive` requests at 64KB and
 * reject anything over it outright, so a long note cannot leave by that door.
 * The margin covers the JSON envelope and the expected-hash field; above the
 * limit the save goes out as an ordinary request and the synchronous draft is
 * what actually survives the page.
 */
const KEEPALIVE_BODY_LIMIT_BYTES = 60_000;

/** Which note a scheduled draft write belongs to, and the version it is an
 * edit of. Captured when the write is scheduled, so a pending write cannot
 * follow the page onto the next note. */
type DraftTarget = {
  vaultId: string;
  slug: string;
  baseContentHash: string;
};

/**
 * Whether the primary pointer cannot hover, which is what puts the editor's
 * formatting in a bar above the keyboard rather than in a floating toolbar
 * (#540). Guarded because jsdom and older WebKit do not implement matchMedia.
 */
function isCoarsePointer(): boolean {
  return window.matchMedia?.("(pointer: coarse)").matches ?? false;
}

/**
 * The note split into the lines before the body (the frontmatter block, when
 * there is one) and the body with `\n` endings, which is what the editor
 * holds. `compose` puts an edited body back under the current frontmatter in
 * the file's own line ending, so a CRLF note stays CRLF (ADR-22).
 */
function splitBody(content: string): { body: string; offset: number } {
  const offset = frontmatterLineOffset(content);
  return {
    body: content.split(/\r?\n/).slice(offset).join("\n"),
    offset,
  };
}

function composeContent(current: string, body: string): string {
  const ending = detectLineEnding(current);
  const head = current.split(/\r?\n/).slice(0, frontmatterLineOffset(current));
  return [...head, ...body.split("\n")].join(ending);
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

const NOTE_REMARK_PLUGINS = [remarkGfm, remarkMath, remarkHideQueryMarkers];

export function NotePage({
  onActiveNoteChange,
  onHeadingsChange,
  onTagSelect,
  propertiesCollapsedStorageKey,
  vaultRevision,
  writeEnabled,
  editRequestId,
  onWriteNotice,
  onDemoRefusal,
  demoMode = false,
  noteCandidates = [],
  vaults,
}: {
  onActiveNoteChange: (meta: ActiveNoteMeta | null) => void;
  /** The open note's headings as the table of contents lists them (#530),
   * for the shell's phone "On this page" chip; `[]` when none or no note. */
  onHeadingsChange?: (headings: NoteHeading[]) => void;
  /** Tags are per-Vault vocabularies, so tapping one hands the search dialog
   * this note's own Vault to pre-select in its filter (#144). */
  onTagSelect: (tag: string, vaultId: VaultId) => void;
  propertiesCollapsedStorageKey: string;
  /** `null` until the collection client has discovered anything. */
  vaultRevision: number | null;
  writeEnabled: boolean;
  editRequestId: number;
  onWriteNotice?: (message: string | null) => void;
  /** A demo_read_only refusal on save takes over entirely (#152): the app's
   * own sentence lands in the notice strip instead of the inline editor
   * error, and editing closes rather than inviting a retry. Returns whether
   * the error was a demo refusal. */
  onDemoRefusal?: (error: unknown) => boolean;
  /** #152: suppresses the held-drafts banner entirely, since it names and
   * links to the Settings surface withheld from a demo visitor. */
  demoMode?: boolean;
  noteCandidates?: NoteCandidate[];
  vaults: VaultSummary[];
}) {
  const params = useParams<{ vaultId: string; slug: string }>();
  const location = useLocation();
  const navigate = useNavigate();
  const vaultId = params.vaultId ?? "";
  const slug = params.slug ?? "";
  // Exact reads show provenance whenever more than one Vault is enabled
  // (#140) — unlike collection surfaces, independent of the browsing scope:
  // a note's own Vault is never ambiguous just because scope is narrowed
  // elsewhere.
  const activeVault = vaults.find((vault) => vault.vault_id === vaultId);
  const vaultName =
    vaults.length > 1 ? (activeVault?.name ?? vaultId) : undefined;
  // No Vault condition blocks a save before it is attempted (#372). A sync
  // conflict (ADR-30) or a sync stopped on files changed by hand halts only
  // commit and sync; the note's own writes still land on disk. Saves the
  // server refuses surface through autosave's own status below.
  const [note, setNote] = useState<Note | null>(null);
  const [noteLinks, setNoteLinks] = useState<NoteLinks | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isEditing, setIsEditing] = useState(false);
  // An upload reads the Vault's link style after re-reading the Vault list,
  // so it needs the Vault as it stands then, not as this render saw it.
  const latestVaultRef = useRef(activeVault);
  useEffect(() => {
    latestVaultRef.current = activeVault;
  }, [activeVault]);
  // The style can change outside Hatchdoor, so opening the editor re-reads it.
  useEffect(() => {
    if (isEditing) {
      void refreshVaultCollection();
    }
  }, [isEditing]);
  const [draftContent, setDraftContent] = useState("");
  const [editBaseHash, setEditBaseHash] = useState("");
  const [draftNotice, setDraftNotice] = useState<string | null>(null);
  const [draftStale, setDraftStale] = useState(false);
  // What the inline write surface says when a draft outlived a crash, a reload
  // or a service-worker update (#330). Source mode has `draftNotice` for the
  // same job; the inline editor has no open/close moment to hang one on.
  const [recoveredDraftNotice, setRecoveredDraftNotice] = useState<
    string | null
  >(null);
  // A draft put back into the body and still owed to the vault (#330). Held
  // until autosave can take it, which is not the commit the note lands on.
  const [restoredCommit, setRestoredCommit] = useState<string | null>(null);
  // True once a block-editor autosave hits demo_read_only (#152): the app's
  // notice-strip sentence already covers it, so the generic autosave-error
  // banner below stays suppressed for the rest of this note session — the
  // same permanent-for-this-session lifetime `useNoteAutosave` itself gives
  // its own stopped state once a save fails.
  const [autosaveDemoRefusal, setAutosaveDemoRefusal] = useState(false);
  const [conflict, setConflict] = useState(false);
  // The disk version a conflict review compares against, tagged with the note
  // it was read for (#331). The route reuses this component across notes, so
  // an untagged copy outlived navigation and a review opened on one note could
  // resolve into another: "Keep draft on latest" then saved the second note's
  // text under the first note's slug and current hash, which the server has
  // no way to refuse.
  const [conflictDisk, setConflictDisk] = useState<{
    noteKey: string;
    note: Note;
  } | null>(null);
  const [noteChangedOnDisk, setNoteChangedOnDisk] = useState(false);
  const [editorError, setEditorError] = useState<string | null>(null);
  const [inlineDirty, setInlineDirty] = useState(false);
  // The live editor has focus: text may be sitting in it that no save has
  // seen yet, which is what the revision guard and the reload hold watch.
  const [editorFocused, setEditorFocused] = useState(false);
  const liveEditorRef = useRef<LiveEditorHandle | null>(null);
  // Rendered page instead of the editor, remembered per browser (#541).
  const [readingView, setReadingView] = useState<boolean>(
    () => safeGetItem(READING_VIEW_KEY) === "1",
  );
  const toggleReadingView = useCallback(() => {
    setReadingView((prev) => {
      safeSetItem(READING_VIEW_KEY, prev ? "0" : "1");
      return !prev;
    });
  }, []);
  const [editorSearchHits, setEditorSearchHits] = useState(0);
  const [saving, setSaving] = useState(false);
  // The phone remembers its own answer (#530): a grid opened once on a wide
  // screen used to stay open on every phone visit, where it costs the
  // first screen. Both start folded.
  const isMobile = useIsMobile(920);
  const propertiesCollapsedKey = isMobile
    ? `${propertiesCollapsedStorageKey}.mobile`
    : propertiesCollapsedStorageKey;
  const [propertiesCollapsed, setPropertiesCollapsed] = useState<boolean>(
    () => {
      return safeGetItem(propertiesCollapsedKey) !== "0";
    },
  );
  // Pre-#137 drafts recovered into Settings (#151): named here, not silently
  // acted on. Dismissing is per view, not persisted — it returns on every
  // load until the last held draft is dealt with.
  const [heldDraftsPresent] = useState(() => listHeldDrafts().length > 0);
  const [heldDraftsBannerDismissed, setHeldDraftsBannerDismissed] =
    useState(false);
  const [searchHitCount, setSearchHitCount] = useState(0);
  const [activeSearchHit, setActiveSearchHit] = useState(0);
  const noteBodyRef = useRef<HTMLDivElement | null>(null);
  const searchHitsRef = useRef<HTMLSpanElement[]>([]);
  const noteKey = `${vaultId}:${slug}`;
  const currentNoteKeyRef = useRef(noteKey);
  // Only ever the open note's own disk version, whatever is left in state.
  const conflictNote =
    conflictDisk && conflictDisk.noteKey === noteKey ? conflictDisk.note : null;
  /** Record the disk version read for `forKey`, unless the user has moved on
   * to another note while the read was in flight. */
  const showConflictFor = useCallback((forKey: string, disk: Note) => {
    if (forKey === currentNoteKeyRef.current) {
      setConflictDisk({ noteKey: forKey, note: disk });
    }
  }, []);
  const lastEditRequestIdRef = useRef(editRequestId);
  // `null` until a revision is known. The first one observed is the revision
  // the open note was already read at, not a change to it.
  const lastHandledRevisionRef = useRef<number | null>(null);
  const autosaveStatusRef = useRef<string>("idle");
  const editorFocusedRef = useRef(false);
  const latestContentRef = useRef("");
  currentNoteKeyRef.current = noteKey;

  // Draft persistence (#330). One writer serves both write surfaces: source
  // mode's textarea and the live editor, including text still sitting in the
  // editor, which until now existed nowhere but React state and died with the
  // tab.
  //
  // In source mode the draft is based on the hash the editor saves against; in
  // inline mode autosave keeps moving that hash forward and reports each new
  // one through `onSaved`, so the note's own hash is the current base.
  const draftTargetRef = useRef<DraftTarget | null>(null);
  draftTargetRef.current = note
    ? {
        vaultId,
        slug: note.slug,
        baseContentHash: isEditing
          ? editBaseHash || note.content_hash
          : note.content_hash,
      }
    : null;
  // Where a scheduled write is going is captured when it is scheduled, not read
  // when it fires: opening another note moves the target, and a timer left over
  // from the previous one would otherwise file its text under the new note.
  const draftPendingRef = useRef<{
    target: DraftTarget;
    content: string;
  } | null>(null);
  const draftTimerRef = useRef<number | null>(null);
  // The last document the vault confirmed it holds. A debounced write can be
  // scheduled before a save and fire after it, and recreating the draft then
  // would leave text the vault already has sitting in the store under a hash
  // that has moved on: `onSaved` cleared it a moment earlier, and the next
  // visit to the note would report a held edit that was in fact saved (#330).
  const savedContentRef = useRef<string | null>(null);
  // Latched once a draft write is refused: with site data blocked, storage
  // full, or a browser set to clear on exit, a silent failure is
  // indistinguishable from a working store, and the editor goes on promising a
  // safety net that is not there.
  const [draftStorageBlocked, setDraftStorageBlocked] = useState(false);

  const cancelDraftWrite = useCallback(() => {
    if (draftTimerRef.current !== null) {
      window.clearTimeout(draftTimerRef.current);
      draftTimerRef.current = null;
    }
    draftPendingRef.current = null;
  }, []);

  /** Write the draft immediately, optionally for content the debounce has not
   * seen yet (the unload flush knows the newest document before this does). */
  const writeDraftNow = useCallback((override?: string) => {
    if (draftTimerRef.current !== null) {
      window.clearTimeout(draftTimerRef.current);
      draftTimerRef.current = null;
    }
    const pending = draftPendingRef.current;
    draftPendingRef.current = null;
    const target =
      override === undefined
        ? (pending?.target ?? null)
        : draftTargetRef.current;
    const content = override ?? pending?.content ?? null;
    if (content === null || !target) {
      return;
    }
    // Nothing to rescue: this is the document on disk.
    if (content === savedContentRef.current) {
      return;
    }
    const stored = saveNoteDraft(target.vaultId, target.slug, {
      vaultId: target.vaultId,
      slug: target.slug,
      content,
      baseContentHash: target.baseContentHash,
      savedAt: Date.now(),
    });
    if (!stored) {
      setDraftStorageBlocked(true);
    }
  }, []);

  const scheduleDraftWrite = useCallback(
    (content: string) => {
      const target = draftTargetRef.current;
      if (!target) {
        return;
      }
      draftPendingRef.current = { target, content };
      if (draftTimerRef.current !== null) {
        window.clearTimeout(draftTimerRef.current);
      }
      draftTimerRef.current = window.setTimeout(() => {
        draftTimerRef.current = null;
        writeDraftNow();
      }, DRAFT_WRITE_DEBOUNCE_MS);
    },
    [writeDraftNow],
  );

  // The debounce window is exactly what a closing tab falls into, so the draft
  // is forced out synchronously before the document can be torn down. This runs
  // independently of the autosave flush: the network send can be refused (an
  // oversized keepalive body, an unreachable vault) and reports nothing back to
  // a page that no longer exists. Unmounting — leaving the note for another
  // part of the app — gets the same treatment.
  useEffect(() => {
    const flush = () => writeDraftNow();
    const onVisibility = () => {
      if (document.visibilityState === "hidden") {
        writeDraftNow();
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", flush);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", flush);
      writeDraftNow();
    };
  }, [writeDraftNow]);

  // Which note `note` was read for. The route reuses this component, so on the
  // render where the key changes `note` still holds the previous note until
  // the new one's read lands.
  const noteLoadedForRef = useRef<string | null>(null);

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
        noteLoadedForRef.current = noteKey;
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

    // The links read can take far longer than the note read in a large
    // Vault, so the body never waits for it (#361): the panel starts empty
    // and fills in when its read lands.
    setNoteLinks(null);
    void loadNoteLinks();
    void (async () => {
      setLoading(true);
      await loadNote(true);
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
    setConflictDisk(null);
    setNoteChangedOnDisk(false);
    setEditorError(null);
    setSaving(false);
    setInlineDirty(false);
    setRecoveredDraftNotice(null);
    setRestoredCommit(null);
    savedContentRef.current = null;
  }, [noteKey]);

  useEffect(() => {
    if (
      vaultRevision === null ||
      vaultRevision === lastHandledRevisionRef.current
    ) {
      return;
    }
    // The collection client publishes the revision it discovered the Vaults
    // at, which lands just after this note was read and describes the same
    // state. Adopting it without acting is what keeps a plain page load from
    // reading the note a second time and reseeding the hash the editor saves
    // against. A genuine later change reports a revision past this one.
    //
    // `null` above is "no discovery yet", never "revision 0": a server that
    // restarted and is genuinely at 0 publishes 0, takes it as the baseline
    // here, and its next change is acted on rather than eaten.
    const isBaseline = lastHandledRevisionRef.current === null;
    lastHandledRevisionRef.current = vaultRevision;
    if (isBaseline) {
      return;
    }

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
      editorFocusedRef.current ||
      autosaveStatusRef.current === "saving"
    ) {
      // Unless no write of ours can be in flight at all. `inlineDirty` is only
      // ever cleared by a save landing, so once autosave has stopped, "quiet
      // again" never arrives and the page would ignore every later revision
      // for the rest of the session (#330).
      // The bump is therefore someone else's. It is flagged rather than
      // followed: refetching here would replace unsaved inline text with the
      // version on disk, which is the loss this issue exists to prevent.
      if (
        autosaveStatusRef.current === "error" ||
        autosaveStatusRef.current === "conflict"
      ) {
        setNoteChangedOnDisk(true);
      }
      return;
    }

    void loadNote(false);
    void loadNoteLinks();
  }, [loadNote, loadNoteLinks, vaultRevision, isEditing, inlineDirty]);

  // The key moves when the window crosses 920px (a tablet rotating), and the
  // fold follows the key's own stored answer rather than carrying the other
  // width's across. Persisting happens on the toggle alone, so the move
  // itself never writes.
  useEffect(() => {
    setPropertiesCollapsed(safeGetItem(propertiesCollapsedKey) !== "0");
  }, [propertiesCollapsedKey]);
  const togglePropertiesCollapsed = useCallback(() => {
    setPropertiesCollapsed((prev) => {
      safeSetItem(propertiesCollapsedKey, prev ? "0" : "1");
      return !prev;
    });
  }, [propertiesCollapsedKey]);

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
    // The live editor keeps its own text until it loses focus; source mode
    // opening over it would show the version without that text (#530).
    if (editorFocusedRef.current) {
      return;
    }
    startEditing();
  }, [editRequestId, startEditing]);

  // A recovered draft (#151): Settings already seeded this note's ordinary
  // draft slot and navigated here with the content unsaved. Open the editor
  // the same way the Edit button would, then drop the marker so a refresh
  // does not reopen it.
  useEffect(() => {
    if (!note || isEditing) {
      return;
    }
    const queryParams = new URLSearchParams(location.search);
    if (queryParams.get("restoreEdit") !== "1") {
      return;
    }
    queryParams.delete("restoreEdit");
    const suffix = queryParams.toString();
    navigate(`${location.pathname}${suffix ? `?${suffix}` : ""}`, {
      replace: true,
    });
    startEditing();
  }, [
    note,
    isEditing,
    location.pathname,
    location.search,
    navigate,
    startEditing,
  ]);

  useEffect(() => {
    if (!isEditing || !note) {
      return;
    }

    scheduleDraftWrite(draftContent);
  }, [draftContent, isEditing, note, scheduleDraftWrite]);

  const parsed = useMemo(() => parseFrontmatter(note?.content ?? ""), [note]);

  // A note ends where its text ends — until someone navigates it by heading.
  // Reaching a heading near the end means scrolling past the end, so the first
  // jump adds the trailing space that makes that possible, and it stays for as
  // long as the reader is on this note. It cannot be transient: dropping the
  // space again would clamp the scroll and pull the heading straight back down.
  const [tailArmed, setTailArmed] = useState(false);
  useEffect(() => {
    setTailArmed(false);
  }, [note?.slug]);

  // The space has to be in the DOM before the scroll, or the jump clamps short.
  // In the editor a heading is a line, not an element, so the editor scrolls.
  const headingLinesRef = useRef(new Map<string, number>());
  const jumpToHeadingWithTail = useCallback((id: string) => {
    setTailArmed(true);
    window.requestAnimationFrame(() => {
      const line = headingLinesRef.current.get(id);
      if (liveEditorRef.current && line !== undefined) {
        liveEditorRef.current.scrollToLine(line);
      } else {
        jumpToHeading(id);
      }
    });
  }, []);

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
  const noteRelativePath = note?.relative_path ?? "";
  const { resolved: markdown, resolvedFor } = useResolvedWikilinks(
    vaultId,
    renderInput,
    noteRelativePath,
  );
  // The live editor's wikilinks, resolved by the server through the reading
  // view's cache, one request per pass of links coming into view (#544).
  const resolveNoteLink = useMemo(
    () =>
      createNoteLinkResolver((targets) =>
        resolveNoteTargets(vaultId, noteRelativePath, targets),
      ),
    [vaultId, noteRelativePath],
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
  headingLinesRef.current = new Map(
    tocHeadings.map(({ id, sourceLine }) => [id, sourceLine]),
  );
  // A note that opens with `# <its own title>` says the title twice (#530):
  // once in the page's title block and once as its first heading. That
  // heading stays in the file and in the DOM (line-addressed editing) but is
  // neither drawn nor listed. Only the first heading qualifies, and only
  // when nothing but blank lines precede it.
  const hiddenHeadingLine = useMemo(
    () => duplicateTitleHeadingLine(parsed.body, tocHeadings, note?.title),
    [parsed.body, tocHeadings, note?.title],
  );
  const visibleHeadings = useMemo(
    () =>
      hiddenHeadingLine === undefined
        ? tocHeadings
        : tocHeadings.filter((h) => h.sourceLine !== hiddenHeadingLine),
    [tocHeadings, hiddenHeadingLine],
  );
  useEffect(() => {
    onHeadingsChange?.(note ? visibleHeadings : []);
  }, [note, visibleHeadings, onHeadingsChange]);
  useEffect(() => () => onHeadingsChange?.([]), [onHeadingsChange]);
  const rehypePlugins = useMemo(
    () => [rehypeKatex, createSearchHighlightPlugin(searchQuery)],
    [searchQuery],
  );
  const headingIdsBySourceLine = useMemo(
    () => new Map(tocHeadings.map(({ sourceLine, id }) => [sourceLine, id])),
    [tocHeadings],
  );
  // Properties and the live editor both save through autosave; the editor
  // itself is the body unless the reader asked for the rendered page.
  const inlineEditingEnabled = writeEnabled && !isEditing && !!note;
  const liveEditingEnabled = inlineEditingEnabled && !readingView;
  const renderedMarkdown = markdown;

  // Evaluated for the reading view and the live editor alike, since both
  // draw the tables (#544); not while source mode holds the body, where the
  // definition shows as code.
  const savedQueries = useSavedQueries(
    notePath,
    note?.content,
    note?.content_hash,
    vaultRevision,
    !isEditing,
  );

  const markdownComponents = useMemo(
    () =>
      createNoteMarkdownComponents(
        vaultId,
        note?.relative_path ?? "",
        headingIdsBySourceLine,
        { hiddenHeadingLine },
      ),
    [vaultId, note?.relative_path, headingIdsBySourceLine, hiddenHeadingLine],
  );

  const autosaveRef = useRef<ReturnType<typeof useNoteAutosave> | null>(null);

  // The document differs from the one on disk, from this keystroke on. Read
  // by the revision guard, which must not refetch over unsaved text, and by
  // the reload hold.
  const markDirty = () => {
    if (note && !inlineDirty) {
      setEditBaseHash(note.content_hash);
      setInlineDirty(true);
    }
  };

  const handleInlineChange = (nextContent: string) => {
    if (!note) {
      return;
    }
    // Readable before React re-renders. A commit made inside an async handler
    // has to be visible to the rest of that handler, which still holds the
    // document this render closed over.
    latestContentRef.current = nextContent;
    markDirty();
    // The user has moved past the restored document, so replaying it would
    // write back text they have already edited.
    setRestoredCommit(null);
    setDraftContent(nextContent);
    setNote((prev) => (prev ? { ...prev, content: nextContent } : prev));
    // Before the write, not after it: the draft is what covers the write
    // failing, being refused, or never being attempted.
    scheduleDraftWrite(nextContent);
    autosaveRef.current?.commit(nextContent);
  };

  // A save started while the page is going away has to outlive the page, which
  // an ordinary fetch does not: it is cancelled with the document (#330). The
  // draft is written first and synchronously, because the send can still be
  // refused and nothing it reports can reach a page that is already gone.
  // Hiding a tab is not the same as closing it, so the outcome is returned
  // rather than dropped: autosave books it exactly like an ordinary save, and
  // a page that comes back has a current hash instead of conflicting on the
  // next keystroke. A page that is really gone never sees it resolve, which is
  // what the draft above covers.
  const flushSave = useCallback(
    async (content: string, expectedHash: string) => {
      writeDraftNow(content);
      const keepalive =
        new TextEncoder().encode(content).length < KEEPALIVE_BODY_LIMIT_BYTES;
      try {
        const outcome = await updateNote(vaultId, slug, content, expectedHash, {
          keepalive,
        });
        savedContentRef.current = content;
        return outcome;
      } catch (error) {
        if (onDemoRefusal?.(error)) {
          setAutosaveDemoRefusal(true);
        }
        throw error;
      }
    },
    [onDemoRefusal, slug, vaultId, writeDraftNow],
  );

  const autosave = useNoteAutosave({
    baseHash: note?.content_hash ?? "",
    enabled: inlineEditingEnabled,
    save: async (nextContent, expectedHash) => {
      try {
        const outcome = await updateNote(
          vaultId,
          slug,
          nextContent,
          expectedHash,
        );
        savedContentRef.current = nextContent;
        return outcome;
      } catch (error) {
        // Same defense-in-depth backstop as every other write path (#152):
        // the hook's own catch still stops autosave for this session either
        // way, but the notice shown for it must be the app's one sentence,
        // not the generic "could not reach the vault" banner below.
        if (onDemoRefusal?.(error)) {
          setAutosaveDemoRefusal(true);
        }
        throw error;
      }
    },
    flushSave,
    onSaved: (result) => {
      setNote((prev) =>
        prev && result.content_hash
          ? { ...prev, content_hash: result.content_hash }
          : prev,
      );
      setInlineDirty(false);
      // The vault holds this text now, so the local copy has nothing left to
      // rescue. A debounced write scheduled since carries newer text and
      // legitimately recreates it a moment later; one carrying the text that
      // was just saved is skipped by `writeDraftNow` instead of resurrecting
      // this key against a hash that has already moved.
      clearNoteDraft(vaultId, note?.slug ?? slug);
    },
  });

  useEffect(() => {
    autosaveRef.current = autosave;
    autosaveStatusRef.current = autosave.status;
  }, [autosave]);

  // Hold off the service worker's own reload while an edit is in the air
  // (#330). A nightly build activates and reloads the page with no prompt, and
  // the trigger for pulling it — coming back to the tab — is exactly the
  // moment the editor is sitting there with unsaved text. The draft now
  // survives that reload, but not causing it is better than recovering from
  // it. The hold is released the moment the save lands, the editor loses
  // focus, or this note is left. The source editor holds for as long as it is open: its text
  // reaches the draft on a debounce, so a reload mid-typing still costs the
  // last few keystrokes (#332).
  const reloadHeld =
    writeEnabled &&
    (isEditing ||
      inlineDirty ||
      editorFocused ||
      autosave.status === "saving" ||
      saving);
  useEffect(() => {
    holdAppReload(`note:${noteKey}`, reloadHeld);
    return () => holdAppReload(`note:${noteKey}`, false);
  }, [noteKey, reloadHeld]);

  useEffect(() => {
    latestContentRef.current = note?.content ?? "";
  }, [note?.content]);

  // Crash recovery for the inline write surface (#330). Source mode reads its
  // draft when the editor opens; the inline editor is always open, so its only
  // entry point is the note landing. A draft that still names the hash now on
  // disk is the write that was interrupted, so it is put back into the body and
  // handed to autosave to finish. One that names an older hash is not safe to
  // replay — the note moved underneath it — so the body is left alone and the
  // user is pointed at source mode, which already knows how to show a stale
  // draft against the current version.
  const draftRecoveredForRef = useRef<string | null>(null);
  useEffect(() => {
    if (!note || !writeEnabled || isEditing) {
      return;
    }
    if (draftRecoveredForRef.current === noteKey) {
      return;
    }
    // Held-draft recovery (#151) is already opening source mode with this same
    // draft; recovering it here too would fight that flow for the body.
    if (new URLSearchParams(location.search).get("restoreEdit") === "1") {
      return;
    }
    draftRecoveredForRef.current = noteKey;

    const stored = loadNoteDraft(vaultId, note.slug);
    if (!stored || stored.content === note.content) {
      return;
    }
    if (stored.baseContentHash !== note.content_hash) {
      setRecoveredDraftNotice(
        "An unsaved edit to this note is being held, but the note has changed since. Use Edit to review it against the current version.",
      );
      return;
    }

    latestContentRef.current = stored.content;
    setEditBaseHash(stored.baseContentHash);
    setInlineDirty(true);
    setDraftContent(stored.content);
    setNote((prev) => (prev ? { ...prev, content: stored.content } : prev));
    setRecoveredDraftNotice(
      "Restored an edit that had not reached the vault yet.",
    );
    // Not committed here: on the commit the note lands, wikilink resolution has
    // not settled, so inline editing — and with it autosave — is still off and
    // the write would be swallowed. Handed to the effect below, which fires as
    // soon as autosave can actually take it.
    setRestoredCommit(stored.content);
  }, [isEditing, location.search, note, noteKey, vaultId, writeEnabled]);

  // Finish the interrupted write once autosave is in a position to make it. A
  // restored edit that never gets this far is not lost: the draft it came from
  // is still on disk, and the notice above says the vault does not have it.
  useEffect(() => {
    if (restoredCommit === null || !inlineEditingEnabled) {
      return;
    }
    setRestoredCommit(null);
    autosaveRef.current?.commit(restoredCommit);
  }, [restoredCommit, inlineEditingEnabled]);

  // Text typed into the live editor has no other home in React state, so it is
  // flushed to the vault after an idle pause and on the way out of the page
  // rather than waiting for blur, and written to the local draft on the same
  // schedule, which is what survives the vault refusing it (#330).
  const handleInProgressChange = (nextContent: string) => {
    scheduleDraftWrite(nextContent);
    autosaveRef.current?.touch(nextContent);
  };

  // The editor holds the body alone; the frontmatter above it is whatever the
  // properties grid has made of it since.
  const handleEditorChange = (body: string) => {
    markDirty();
    handleInProgressChange(composeContent(latestContentRef.current, body));
  };
  const handleEditorCommit = (body: string) => {
    handleInlineChange(composeContent(latestContentRef.current, body));
  };
  const handleEditorFocusChange = useCallback((focused: boolean) => {
    editorFocusedRef.current = focused;
    setEditorFocused(focused);
  }, []);

  // What autocomplete and the attachment inserts write follows the Vault's
  // link style (ADR-33). The style is read from the Vault and can change in
  // another editor, so opening the editor and every upload re-read the Vault
  // list. An upload uses the style as it stands once `refreshing` has landed.
  const embedForUpload = async (
    upload: NoteAttachmentUpload,
    noteRelativePath: string,
    refreshing: Promise<void>,
  ): Promise<string> => {
    await refreshing.catch(() => undefined);
    return attachmentEmbedText(
      linkStyleOf(latestVaultRef.current),
      upload,
      noteRelativePath,
      (targets) => resolveAssetTargets(vaultId, noteRelativePath, targets),
    );
  };

  const reviewConflict = () => {
    // The conflict review lives in source mode, which already knows how to
    // show the disk version beside the draft.
    setDraftContent(note?.content ?? "");
    setEditBaseHash(editBaseHash || (note?.content_hash ?? ""));
    setConflict(true);
    setIsEditing(true);
    const reviewedKey = noteKey;
    void (async () => {
      try {
        const res = await apiFetch(notePath);
        if (res.ok) {
          const json = (await res.json()) as VaultQualifiedNote;
          showConflictFor(reviewedKey, json.note);
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
  }, [markdown, note?.slug, searchQuery, matchHeading]);

  // Jumping to the first hit is a landing gesture: it runs once per arrival,
  // never again on a later recount, which would throw the reader back to the
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
      jumpToHeadingWithTail(slugifyHeading(lastSegment));
    }
  }, [markdown, note?.slug, searchQuery, matchHeading, jumpToHeadingWithTail]);

  // A wikilink carrying a heading arrives as a fragment. The browser used to
  // resolve it on its own, back when following one meant loading the page
  // again; routing the link keeps the app mounted, so the jump is ours to
  // make.
  //
  // `settling` is what says the body on screen is still the note that was
  // linked *from*: the note loads before its wikilinks resolve, and a heading
  // of the same name in both notes would otherwise scroll the wrong one.
  //
  // Deliberately unconditional, rather than listing the states that might have
  // put the heading on screen. The body appears once the note's fetch, its
  // wikilink resolution and its render have all landed, in an order that has
  // already changed once between a cold visit and a warm one; naming a subset
  // of them means the jump silently stops happening when the order shifts
  // again. The ref makes this a no-op after the jump, so the cost is one
  // lookup per commit while a fragment is still waiting for its heading.
  //
  // The key is the history entry, not the note, so following the same link a
  // second time jumps again the way the browser always re-jumped, while a
  // content change under a reader who has since scrolled away leaves them
  // where they are.
  const hashTarget = location.hash
    ? decodeURIComponent(location.hash.slice(1))
    : "";
  const hashJumpKey = `${location.key}:${note?.slug ?? ""}#${hashTarget}`;
  const lastHashJumpRef = useRef<string | null>(null);
  useLayoutEffect(() => {
    if (!hashTarget || settling || lastHashJumpRef.current === hashJumpKey) {
      return;
    }
    // In the editor a heading is a line, known from the note's own text;
    // in the reading view it is an element that has to be on screen.
    const onScreen = liveEditorRef.current
      ? headingLinesRef.current.has(hashTarget)
      : !!noteBodyRef.current?.querySelector(`#${CSS.escape(hashTarget)}`);
    if (!onScreen) {
      return;
    }
    lastHashJumpRef.current = hashJumpKey;
    jumpToHeadingWithTail(hashTarget);
  });

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
        tone="error"
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

    cancelDraftWrite();
    clearNoteDraft(vaultId, note.slug);
    setDraftContent(note.content);
    setEditorError(null);
    setDraftNotice(null);
    setRecoveredDraftNotice(null);
    setDraftStale(false);
    setConflict(false);
    setConflictDisk(null);
    setSaving(false);
    setIsEditing(false);

    // If the note changed on disk while we held the editor open, pick up the
    // latest now that the editor is closed.
    if (noteChangedOnDisk) {
      setNoteChangedOnDisk(false);
      setLoading(true);
      void loadNoteLinks();
      void (async () => {
        await loadNote(true);
        setLoading(false);
      })();
    }
  };

  const handleReloadLatest = async () => {
    setSaving(true);
    setEditorError(null);
    const reloadKey = noteKey;
    try {
      const res = await apiFetch(notePath);
      if (!res.ok) {
        throw new Error(await readErrorMessage(res, "Failed loading note"));
      }
      const json = (await res.json()) as VaultQualifiedNote;
      // Left for another note mid-read: this version, and the draft rebased
      // onto it, belong to a note that is no longer open (#331).
      if (reloadKey !== currentNoteKeyRef.current) {
        return;
      }
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
      setConflictDisk(null);
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
    // Everything after the round trip describes this note. If the user has
    // opened another one meanwhile, none of it may land on that one (#331).
    const savedKey = noteKey;

    try {
      const outcome = await updateNote(
        vaultId,
        note.slug,
        draftContent,
        editBaseHash,
      );
      clearNoteDraft(vaultId, note.slug);
      if (savedKey !== currentNoteKeyRef.current) {
        return;
      }
      cancelDraftWrite();
      setConflict(false);
      setConflictDisk(null);
      setNoteChangedOnDisk(false);
      setDraftStale(false);
      setDraftNotice(null);
      setRecoveredDraftNotice(null);
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
      if (onDemoRefusal?.(saveError)) {
        if (savedKey !== currentNoteKeyRef.current) {
          return;
        }
        setIsEditing(false);
        setInlineDirty(false);
      } else if (savedKey !== currentNoteKeyRef.current) {
        // The draft for the note that failed is still in its store.
        return;
      } else if (
        saveError instanceof Error &&
        saveError.name === "ConflictError"
      ) {
        setConflict(true);
        // Set before the read below, which the user can outlast by leaving.
        setEditorError(
          "This note changed on disk since you started editing. Review the disk version against your draft before saving again.",
        );
        try {
          const res = await apiFetch(notePath);
          if (res.ok) {
            const json = (await res.json()) as VaultQualifiedNote;
            showConflictFor(savedKey, json.note);
          }
        } catch {
          // The generic conflict error still leaves the draft safe in the editor.
        }
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
    setConflictDisk(null);
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
    setConflictDisk(null);
    setNoteChangedOnDisk(false);
    setDraftStale(false);
    setEditorError(null);
    setDraftNotice(
      "Keeping your draft against the latest disk version. Review it, then Save again.",
    );
  };

  const handleUploadAttachment = async (
    file: File,
  ): Promise<UploadedAttachment> => {
    const refreshing = refreshVaultCollection();
    const result = await uploadNoteAttachment(
      file,
      note.relative_path,
      (uploadFile, targetRelativePath) =>
        uploadAttachment(vaultId, uploadFile, targetRelativePath),
    );
    return {
      path: result.embedPath,
      embed: await embedForUpload(result, note.relative_path, refreshing),
    };
  };

  // Links never cross Vaults, and a Markdown link needs a path in this one.
  const vaultNoteCandidates = noteCandidates.filter(
    (candidate) => candidate.vault_id === vaultId,
  );

  // A wikilink in the editor resolves the way the reading view's does (#544):
  // the server answers by title, alias or path, and the label is the target
  // without its folders, as the rendered link shows it. An archived note
  // keeps the path, the way the reading view keeps it.
  const resolveNoteTarget = async (target: string) => {
    const hit = await resolveNoteLink(target);
    return {
      label: hit ? wikilinkLabel(target, hit.archived === true) : target,
      missing: !hit,
    };
  };
  // Opens the note at the heading when the target names one. A `^block`
  // reference carries its id as the fragment, the way the reading view's
  // link does; neither view has a block to scroll to, so it opens at the top.
  const openNoteTarget = async (target: string) => {
    const hit = await resolveNoteLink(target);
    if (!hit) {
      return;
    }
    const hashIdx = target.indexOf("#");
    const caretIdx = target.indexOf("^");
    const anchor =
      hashIdx >= 0
        ? `#${slugifyHeading(target.slice(hashIdx + 1))}`
        : caretIdx >= 0
          ? `#${target.slice(caretIdx + 1)}`
          : "";
    navigate(`/v/${encodeURIComponent(vaultId)}/n/${hit.slug}${anchor}`);
  };

  const formatNoteLink = (candidate: ExplorerNote): string => {
    const target = vaultNoteCandidates.find(
      (vaultNote) => vaultNote.slug === candidate.slug,
    );
    if (!target) {
      return `[[${candidate.title}]]`;
    }
    return noteLinkText(
      linkStyleOf(activeVault),
      candidate.title,
      target.relativePath,
      note.relative_path,
      vaultNoteCandidates.map((vaultNote) => vaultNote.relativePath),
    );
  };

  return (
    <div className="note-page-layout">
      <article className="note-content" data-tail={tailArmed}>
        <div className="note-page-heading">
          <h2 className="note-page-title">{note.title}</h2>
        </div>
        {error ? <StatusBadge tone="warn" text="Showing cached note" /> : null}
        {autosave.status === "conflict" ||
        (autosave.status === "error" && !autosaveDemoRefusal) ? (
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
        {draftStorageBlocked ? (
          <div className="write-notice" role="status">
            <div className="write-notice-messages">
              Your browser isn&rsquo;t storing drafts, so saving is the only way
              to keep this edit.
            </div>
          </div>
        ) : null}
        {noteChangedOnDisk && !isEditing ? (
          <div className="write-notice" role="status">
            <div className="write-notice-messages">
              This note changed on disk while your edit was waiting to save.
              Open Edit to compare the two before writing over it.
            </div>
          </div>
        ) : null}
        {recoveredDraftNotice && !isEditing ? (
          <div className="write-notice" role="status">
            <div className="write-notice-messages">{recoveredDraftNotice}</div>
            <button
              type="button"
              className="write-notice-dismiss"
              aria-label="Dismiss notice"
              onClick={() => setRecoveredDraftNotice(null)}
            >
              ×
            </button>
          </div>
        ) : null}
        {writeEnabled ? (
          <SyncConflictNotice
            vault={activeVault}
            relativePath={note.relative_path}
          />
        ) : null}
        {(liveEditingEnabled ? editorSearchHits : searchHitCount) > 0 ? (
          <SearchHitNavigator
            totalHits={liveEditingEnabled ? editorSearchHits : searchHitCount}
            activeHit={activeSearchHit}
            onSelect={(nextIndex) => {
              setActiveSearchHit(nextIndex);
              if (liveEditingEnabled) {
                liveEditorRef.current?.scrollToHit(nextIndex);
                return;
              }
              const target = searchHitsRef.current[nextIndex];
              scrollElementIntoView(target, {
                block: "center",
                inline: "nearest",
              });
            }}
          />
        ) : null}
        {/* Reading chrome only: the editor carries its own frontmatter form,
            so the grid above it said everything twice (#530). */}
        {isEditing ? null : (
          <NoteProperties
            // Sharing the title's line cost the title width, and a long one
            // wrapped around them.
            actions={
              writeEnabled ? (
                <div className="note-inline-actions">
                  <SaveState
                    status={autosave.status}
                    savedAt={autosave.savedAt}
                  />
                  <UiButton
                    className="close-note note-view-toggle"
                    aria-pressed={readingView}
                    onClick={toggleReadingView}
                  >
                    {readingView ? "Editing" : "Reading"}
                  </UiButton>
                  <UiButton
                    className="close-note note-edit-button"
                    onClick={startEditing}
                  >
                    Source
                    {isMobile ? null : (
                      <span className="shortcut-hint" aria-hidden="true">
                        E
                      </span>
                    )}
                  </UiButton>
                </div>
              ) : null
            }
            properties={parsed.properties}
            vaultName={vaultName}
            content={note.content}
            editable={inlineEditingEnabled}
            onChange={handleInlineChange}
            collapsed={propertiesCollapsed}
            onToggleCollapsed={togglePropertiesCollapsed}
            onTagSelect={(tag) => onTagSelect(tag, vaultId)}
          />
        )}
        {/* Between the desktop breakpoint and the TOC column's own (920 to
            1160px) the headings fold into this strip; below 920 the shell's
            scope row carries them as a chip (#530). */}
        <NoteTocMobile
          headings={visibleHeadings}
          onJump={jumpToHeadingWithTail}
        />
        {heldDraftsPresent && !heldDraftsBannerDismissed && !demoMode ? (
          <div className="write-notice" role="status">
            <div className="write-notice-messages">
              <span>
                Unsaved drafts from before the move to multiple Vaults are being
                held — <Link to="/settings">find them in Settings</Link>.
              </span>
            </div>
            <button
              type="button"
              className="write-notice-dismiss"
              aria-label="Dismiss notice"
              onClick={() => setHeldDraftsBannerDismissed(true)}
            >
              ×
            </button>
          </div>
        ) : null}
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
            noteCandidates={
              activeVault?.link_style === "markdown"
                ? vaultNoteCandidates
                : noteCandidates
            }
            formatNoteLink={formatNoteLink}
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
            onDemoRefusal={onDemoRefusal}
            renderPreview={(value) => (
              <NotePreview
                vaultId={vaultId}
                vaultName={vaultName}
                content={value}
                relativePath={note.relative_path}
              />
            )}
          />
        ) : liveEditingEnabled ? (
          <div className="note-body" dir="auto">
            {/* The saved-query tables inside the editor's `base` widgets read
                the results the server evaluated for the note on disk; a
                block being edited matches none until its save lands. */}
            <SavedQueryProvider
              state={savedQueries}
              vaultId={vaultId}
              markdown={splitBody(note.content).body}
            >
              <LiveEditor
                key={noteKey}
                ref={liveEditorRef}
                value={splitBody(note.content).body}
                searchQuery={searchQuery}
                touch={isCoarsePointer()}
                noteCandidates={vaultNoteCandidates}
                formatNoteLink={formatNoteLink}
                resolveNote={resolveNoteTarget}
                onOpenNote={(target) => void openNoteTarget(target)}
                resolveAssetSrc={(raw) =>
                  cachedAssetHref(vaultId, raw, note.relative_path)
                }
                assetsResolvedFor={resolvedFor}
                onChange={handleEditorChange}
                onCommit={handleEditorCommit}
                onFocusChange={handleEditorFocusChange}
                onSearchHits={setEditorSearchHits}
                onUploadAttachment={handleUploadAttachment}
                onUploadNotice={(message) => onWriteNotice?.(message)}
                onUploadError={(uploadError) => {
                  if (onDemoRefusal?.(uploadError)) {
                    return;
                  }
                  onWriteNotice?.(
                    uploadError instanceof Error
                      ? uploadError.message
                      : "Upload failed.",
                  );
                }}
              />
            </SavedQueryProvider>
          </div>
        ) : (
          <div ref={noteBodyRef} className="note-body" dir="auto">
            <SavedQueryProvider
              state={savedQueries}
              vaultId={vaultId}
              markdown={renderedMarkdown}
            >
              <ReactMarkdown
                remarkPlugins={NOTE_REMARK_PLUGINS}
                rehypePlugins={rehypePlugins}
                components={markdownComponents}
              >
                {renderedMarkdown}
              </ReactMarkdown>
            </SavedQueryProvider>
          </div>
        )}
        {/* Links come after the text (#530): backlinks are what a reader
            consults once they have read, and every outgoing link is already
            a link in the body above. */}
        {isEditing ? null : (
          <NoteLinksPanel vaultId={vaultId} links={noteLinks} />
        )}
      </article>

      <NoteTocDesktop
        headings={visibleHeadings}
        onJump={jumpToHeadingWithTail}
      />
    </div>
  );
}

/** A note in the Vault's current sync conflict stays editable, but an edit
 * made before the conflict is resolved on the Git host can conflict again on
 * the same lines (ADR-30), so the page says so. */
function SyncConflictNotice({
  vault,
  relativePath,
}: {
  vault: VaultSummary | undefined;
  relativePath: string;
}) {
  if (!noteInSyncConflict(vault, relativePath)) return null;
  return (
    <p className="note-editor-notice" role="status">
      This note is part of a sync conflict with the Vault&rsquo;s remote. Edits
      made before the conflict is resolved may conflict again, so resolve it
      first from the Vault&rsquo;s settings.
    </p>
  );
}

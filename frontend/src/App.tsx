import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { Route, Routes, useLocation, useNavigate } from "react-router-dom";

import "./App.css";
import "./noteEnhancements.css";
import { AppTopbar } from "./app/AppTopbar";
import {
  DRAWER_OPEN_KEY,
  EXPLORER_SCROLL_TOP_KEY,
  EXPANDED_FOLDERS_KEY,
  LAST_NOTE_KEY,
  NOTE_PROPERTIES_COLLAPSED_KEY,
  RECENT_NOTES_KEY,
  SIDEBAR_WIDTH_KEY,
} from "./app/constants";
import { ExplorerPane } from "./app/ExplorerPane";
import {
  clampSidebarWidth,
  getStoredExpandedFolders,
  getStoredNumber,
  getStoredRecentNotes,
  getStoredString,
  isEditableTarget,
} from "./app/storage";
import { useIsMobile } from "./app/useIsMobile";
import { useTheme } from "./app/useTheme";
import { apiFetch, onUnauthorized, setToken, withAccessToken } from "./api";
import { readErrorMessage } from "./apiError";
import { copyText } from "./clipboard";
import {
  NoteActionsDialog,
  type NoteActionDialogKind,
} from "./components/NoteActionsDialog";
import { NotePage } from "./components/NotePage";
import { SearchDialog } from "./components/SearchDialog";
import { TokenPrompt } from "./components/TokenPrompt";
import { GraphPage } from "./components/GraphPage";
import { StatsPage } from "./components/StatsPage";
import { StateBlock } from "./components/ui";
import { isExplorerTreeEqual } from "./stateCompare";
import {
  archiveNote,
  createNote,
  deleteNote,
  describeWriteOutcome,
  getWriteCapabilities,
  moveNote,
  renameNote,
} from "./writeApi";
import { validateNotePath } from "./writePaths";
import { clearCreateDraft, pruneNoteDrafts } from "./writeDrafts";
import { collectFolderPaths } from "./app/folderPaths";
import { flattenNoteCandidates } from "./app/noteCandidates";
import type {
  ActiveNoteMeta,
  ExplorerFolder,
  ModifiedNote,
  RecentlyModifiedResponse,
  RecentNote,
  SearchResponse,
  SearchResult,
} from "./types";

function App() {
  const [tree, setTree] = useState<ExplorerFolder | null>(null);
  const [loadingTree, setLoadingTree] = useState(true);
  const [treeError, setTreeError] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState<boolean>(() => {
    return window.localStorage.getItem(DRAWER_OPEN_KEY) === "1";
  });
  const [sidebarWidth, setSidebarWidth] = useState<number>(() =>
    getStoredNumber(SIDEBAR_WIDTH_KEY, 268, 220, 420),
  );
  const [isOnline, setIsOnline] = useState(() => navigator.onLine);
  const [activeNote, setActiveNote] = useState<ActiveNoteMeta | null>(null);
  const [recentNotes, setRecentNotes] = useState<RecentNote[]>(() =>
    getStoredRecentNotes(),
  );
  const [modifiedNotes, setModifiedNotes] = useState<ModifiedNote[]>([]);
  const [expandedFolders, setExpandedFolders] = useState<
    Record<string, boolean>
  >(() => getStoredExpandedFolders());
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchIncludeContent, setSearchIncludeContent] = useState(false);
  const [searchResults, setSearchResults] = useState<SearchResult[]>([]);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [actionsMenuOpen, setActionsMenuOpen] = useState(false);
  const [mobileDrawerTop, setMobileDrawerTop] = useState(0);
  const [visualViewportHeight, setVisualViewportHeight] = useState(
    () => window.visualViewport?.height ?? window.innerHeight,
  );
  const [vaultRevision, setVaultRevision] = useState(0);
  const [authRequired, setAuthRequired] = useState(false);
  const [writeEnabled, setWriteEnabled] = useState(false);
  const [writeWarnings, setWriteWarnings] = useState<string[]>([]);
  const [writeNotice, setWriteNotice] = useState<string | null>(null);
  const [editRequestId, setEditRequestId] = useState(0);
  const [noteActionDialog, setNoteActionDialog] =
    useState<NoteActionDialogKind | null>(null);
  const [noteActionError, setNoteActionError] = useState<string | null>(null);
  const [noteActionInitialFolder, setNoteActionInitialFolder] = useState("");
  const location = useLocation();
  const navigate = useNavigate();
  const isMobile = useIsMobile(920);
  const { theme, cycleTheme } = useTheme();
  const resizingRef = useRef<{ startX: number; startWidth: number } | null>(
    null,
  );
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const topbarRef = useRef<HTMLElement | null>(null);
  const explorerPaneRef = useRef<HTMLElement | null>(null);
  const restoredExplorerScrollRef = useRef(false);
  const restoredLastNoteRef = useRef(false);
  const prevFocusRef = useRef<Element | null>(null);

  const loadTree = useCallback(async () => {
    setTreeError(null);
    try {
      const res = await apiFetch("/api/tree");
      if (!res.ok) {
        throw new Error(await readErrorMessage(res, "Failed loading tree"));
      }
      const nextTree = (await res.json()) as ExplorerFolder;
      setTree((prev) =>
        isExplorerTreeEqual(prev, nextTree) ? prev : nextTree,
      );
    } catch (err) {
      setTreeError(
        err instanceof Error ? err.message : "Unknown tree loading error",
      );
    }
  }, []);

  const loadModifiedNotes = useCallback(async () => {
    try {
      const params = new URLSearchParams({ limit: "5" });
      const res = await apiFetch(`/api/recently-modified?${params.toString()}`);
      if (!res.ok) {
        throw new Error(
          await readErrorMessage(res, "Failed loading modified notes"),
        );
      }
      const json = (await res.json()) as RecentlyModifiedResponse;
      setModifiedNotes(json.notes.slice(0, 5));
    } catch {
      setModifiedNotes([]);
    }
  }, []);

  useEffect(() => {
    onUnauthorized(() => setAuthRequired(true));
    return () => onUnauthorized(null);
  }, []);

  const folderPaths = useMemo(() => collectFolderPaths(tree), [tree]);
  const noteCandidates = useMemo(() => flattenNoteCandidates(tree), [tree]);

  const openCreateDialog = useCallback((folder: string) => {
    setNoteActionError(null);
    setNoteActionInitialFolder(folder);
    setNoteActionDialog("create");
  }, []);

  useEffect(() => {
    // Drafts only bridge an interrupted edit; drop ones older than a week.
    pruneNoteDrafts(7 * 24 * 60 * 60 * 1000);
  }, []);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const capabilities = await getWriteCapabilities();
        if (!cancelled) {
          setWriteEnabled(Boolean(capabilities.enabled));
          setWriteWarnings(
            Array.isArray(capabilities.warnings) ? capabilities.warnings : [],
          );
        }
      } catch {
        if (!cancelled) {
          setWriteEnabled(false);
          setWriteWarnings([]);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    void (async () => {
      setLoadingTree(true);
      await loadTree();
      await loadModifiedNotes();
      setLoadingTree(false);
    })();
  }, [loadModifiedNotes, loadTree]);

  useEffect(() => {
    if (!("EventSource" in window)) {
      return;
    }

    const events = new EventSource(withAccessToken("/api/vault-events"));
    const onVaultRevision = (event: MessageEvent<string>) => {
      try {
        const payload = JSON.parse(event.data) as { revision?: unknown };
        if (typeof payload.revision === "number") {
          const revision = payload.revision;
          setVaultRevision((current) =>
            revision > current ? revision : current,
          );
        }
      } catch {
        // Ignore malformed event payloads; the next valid revision will resync.
      }
    };
    events.addEventListener("vault-revision", onVaultRevision);

    return () => {
      events.removeEventListener("vault-revision", onVaultRevision);
      events.close();
    };
  }, []);

  useEffect(() => {
    if (vaultRevision === 0) {
      return;
    }

    void loadTree();
    void loadModifiedNotes();
  }, [loadModifiedNotes, loadTree, vaultRevision]);

  useEffect(() => {
    window.localStorage.setItem(
      DRAWER_OPEN_KEY,
      drawerOpen && isMobile ? "1" : "0",
    );
  }, [drawerOpen, isMobile]);

  useEffect(() => {
    window.localStorage.setItem(SIDEBAR_WIDTH_KEY, String(sidebarWidth));
  }, [sidebarWidth]);

  useEffect(() => {
    window.localStorage.setItem(RECENT_NOTES_KEY, JSON.stringify(recentNotes));
  }, [recentNotes]);

  useEffect(() => {
    window.localStorage.setItem(
      EXPANDED_FOLDERS_KEY,
      JSON.stringify(expandedFolders),
    );
  }, [expandedFolders]);

  useEffect(() => {
    const onOnline = () => setIsOnline(true);
    const onOffline = () => setIsOnline(false);

    window.addEventListener("online", onOnline);
    window.addEventListener("offline", onOffline);
    return () => {
      window.removeEventListener("online", onOnline);
      window.removeEventListener("offline", onOffline);
    };
  }, []);

  useEffect(() => {
    if (isMobile) {
      setDrawerOpen(false);
    }
    setActionsMenuOpen(false);
  }, [location.pathname, isMobile]);

  useLayoutEffect(() => {
    const updateVisualViewportHeight = () => {
      setVisualViewportHeight(
        window.visualViewport?.height ?? window.innerHeight,
      );
    };

    updateVisualViewportHeight();
    window.addEventListener("resize", updateVisualViewportHeight);
    window.visualViewport?.addEventListener(
      "resize",
      updateVisualViewportHeight,
    );

    return () => {
      window.removeEventListener("resize", updateVisualViewportHeight);
      window.visualViewport?.removeEventListener(
        "resize",
        updateVisualViewportHeight,
      );
    };
  }, []);

  useLayoutEffect(() => {
    if (!isMobile) {
      setMobileDrawerTop(0);
      return;
    }

    const updateDrawerTop = () => {
      const nextTop = topbarRef.current?.getBoundingClientRect().bottom ?? 0;
      setMobileDrawerTop(Math.ceil(nextTop));
    };

    updateDrawerTop();

    const resizeObserver =
      "ResizeObserver" in window ? new ResizeObserver(updateDrawerTop) : null;
    if (topbarRef.current) {
      resizeObserver?.observe(topbarRef.current);
    }

    window.addEventListener("resize", updateDrawerTop);
    window.addEventListener("scroll", updateDrawerTop, { passive: true });
    window.visualViewport?.addEventListener("resize", updateDrawerTop);

    return () => {
      resizeObserver?.disconnect();
      window.removeEventListener("resize", updateDrawerTop);
      window.removeEventListener("scroll", updateDrawerTop);
      window.visualViewport?.removeEventListener("resize", updateDrawerTop);
    };
  }, [isMobile]);

  useEffect(() => {
    if (location.pathname === "/") {
      setActiveNote(null);
    }
  }, [location.pathname]);

  useEffect(() => {
    if (!activeNote) {
      return;
    }

    setRecentNotes((prev) => {
      const withoutCurrent = prev.filter(
        (item) => item.slug !== activeNote.slug,
      );
      const next: RecentNote[] = [
        { ...activeNote, viewedAt: Date.now() },
        ...withoutCurrent,
      ].slice(0, 12);
      return next;
    });
  }, [activeNote]);

  useEffect(() => {
    if (!activeNote) {
      return;
    }
    window.localStorage.setItem(LAST_NOTE_KEY, activeNote.slug);
  }, [activeNote]);

  useEffect(() => {
    if (restoredLastNoteRef.current || location.pathname !== "/") {
      return;
    }
    restoredLastNoteRef.current = true;
    const lastSlug = getStoredString(LAST_NOTE_KEY);
    if (!lastSlug) {
      return;
    }
    navigate(`/n/${encodeURIComponent(lastSlug)}`, { replace: true });
  }, [location.pathname, navigate]);

  useEffect(() => {
    if (searchOpen) {
      prevFocusRef.current = document.activeElement;
      const id = window.setTimeout(() => searchInputRef.current?.focus(), 0);
      return () => window.clearTimeout(id);
    } else {
      if (prevFocusRef.current instanceof HTMLElement) {
        prevFocusRef.current.focus();
      }
      prevFocusRef.current = null;
    }
  }, [searchOpen]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen(true);
        return;
      }

      if (
        (event.ctrlKey || event.metaKey) &&
        event.key.toLowerCase() === "n" &&
        writeEnabled &&
        !isEditableTarget(event.target)
      ) {
        // Works in installed/standalone PWA contexts; harmless where the
        // browser reserves the shortcut.
        event.preventDefault();
        openCreateDialog("");
        return;
      }

      if (event.key === "/" && !isEditableTarget(event.target)) {
        event.preventDefault();
        setSearchOpen(true);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [openCreateDialog, writeEnabled]);

  useEffect(() => {
    let cancelled = false;

    if (!searchOpen) {
      return;
    }

    const query = searchQuery.trim();
    if (query.length < 2) {
      setSearchResults([]);
      setSearchLoading(false);
      setSearchError(null);
      return;
    }

    const id = window.setTimeout(() => {
      void (async () => {
        setSearchLoading(true);
        setSearchError(null);
        try {
          const params = new URLSearchParams({
            q: query,
            mode: searchIncludeContent ? "keyword" : "semantic",
            limit: "30",
            per_note_cap: "2",
          });
          const res = await apiFetch(`/api/search?${params.toString()}`);
          if (!res.ok) {
            throw new Error(await readErrorMessage(res, "Search failed"));
          }
          const json = (await res.json()) as SearchResponse;
          if (!cancelled) {
            setSearchResults(json.results);
          }
        } catch (error) {
          if (!cancelled) {
            setSearchResults([]);
            setSearchError(
              error instanceof Error ? error.message : "Unknown search error",
            );
          }
        } finally {
          if (!cancelled) {
            setSearchLoading(false);
          }
        }
      })();
    }, 150);

    return () => {
      cancelled = true;
      window.clearTimeout(id);
    };
  }, [searchIncludeContent, searchOpen, searchQuery]);

  useEffect(() => {
    const onPointerMove = (event: PointerEvent) => {
      const state = resizingRef.current;
      if (!state) {
        return;
      }
      const delta = event.clientX - state.startX;
      const next = clampSidebarWidth(state.startWidth + delta);
      setSidebarWidth(next);
    };

    const onPointerUp = () => {
      resizingRef.current = null;
      document.body.classList.remove("resizing");
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);

    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };
  }, []);

  useEffect(() => {
    if (restoredExplorerScrollRef.current || !tree) {
      return;
    }
    const container = explorerPaneRef.current;
    if (!container) {
      return;
    }
    const stored = getStoredNumber(EXPLORER_SCROLL_TOP_KEY, 0, 0, 1_000_000);
    container.scrollTop = stored;
    restoredExplorerScrollRef.current = true;
  }, [tree]);

  const treeIsStale = Boolean(tree && treeError);
  const openSearchForTag = useCallback((tag: string) => {
    setSearchQuery(tag);
    setSearchIncludeContent(true);
    setSearchOpen(true);
  }, []);
  const refreshVault = useCallback(async () => {
    try {
      await apiFetch("/api/refresh", { method: "POST" });
    } catch {
      // Fall back to tree refresh even if force refresh endpoint fails.
    }
    await loadTree();
    await loadModifiedNotes();
  }, [loadModifiedNotes, loadTree]);
  const copyNoteLink = useCallback(async () => {
    if (!activeNote) {
      return;
    }
    await copyText(window.location.href);
  }, [activeNote]);
  const copyPageContent = useCallback(async () => {
    if (!activeNote) {
      return;
    }
    await copyText(activeNote.exportContent ?? "");
  }, [activeNote]);
  const downloadMarkdown = useCallback(() => {
    if (!activeNote) {
      return;
    }
    const url = withAccessToken(
      `/api/note/${encodeURIComponent(activeNote.slug)}/download`,
    );
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.setAttribute("download", "");
    anchor.style.display = "none";
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
  }, [activeNote]);

  const closeNoteActionDialog = useCallback(() => {
    setNoteActionDialog(null);
    setNoteActionError(null);
    setNoteActionInitialFolder("");
  }, []);

  const requireActiveNoteHash = useCallback(() => {
    if (!activeNote?.slug || !activeNote.contentHash) {
      throw new Error("Current note is not ready for write actions");
    }
    return { slug: activeNote.slug, contentHash: activeNote.contentHash };
  }, [activeNote]);

  const handleCreateNote = useCallback(
    async (relativePath: string, content: string) => {
      setNoteActionError(null);
      const pathError = validateNotePath(relativePath, { label: "Note path" });
      if (pathError) {
        setNoteActionError(pathError);
        return;
      }
      try {
        const outcome = await createNote(relativePath, content);
        clearCreateDraft();
        setNoteActionDialog(null);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(`/n/${encodeURIComponent(outcome.slug)}`);
        }
      } catch (error) {
        setNoteActionError(
          error instanceof Error ? error.message : "Create failed",
        );
      }
    },
    [navigate, refreshVault],
  );

  const handleRenameNote = useCallback(
    async (newTitle: string) => {
      setNoteActionError(null);
      const trimmed = newTitle.trim();
      if (!trimmed) {
        setNoteActionError("New title is required.");
        return;
      }
      try {
        const { slug, contentHash } = requireActiveNoteHash();
        const outcome = await renameNote(slug, trimmed, contentHash);
        setNoteActionDialog(null);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(`/n/${encodeURIComponent(outcome.slug)}`);
        }
      } catch (error) {
        setNoteActionError(
          error instanceof Error ? error.message : "Rename failed",
        );
      }
    },
    [navigate, refreshVault, requireActiveNoteHash],
  );

  const handleMoveNote = useCallback(
    async (targetFolder: string) => {
      setNoteActionError(null);
      const pathError = validateNotePath(targetFolder, {
        allowEmpty: true,
        label: "Target folder",
      });
      if (pathError) {
        setNoteActionError(pathError);
        return;
      }
      try {
        const { slug, contentHash } = requireActiveNoteHash();
        const outcome = await moveNote(slug, targetFolder, contentHash);
        setNoteActionDialog(null);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(`/n/${encodeURIComponent(outcome.slug)}`);
        }
      } catch (error) {
        setNoteActionError(
          error instanceof Error ? error.message : "Move failed",
        );
      }
    },
    [navigate, refreshVault, requireActiveNoteHash],
  );

  const handleArchiveNote = useCallback(async () => {
    setNoteActionError(null);
    try {
      const { slug, contentHash } = requireActiveNoteHash();
      const outcome = await archiveNote(slug, contentHash);
      setNoteActionDialog(null);
      setWriteNotice(describeWriteOutcome(outcome));
      await refreshVault();
      if (outcome.slug) {
        navigate(`/n/${encodeURIComponent(outcome.slug)}`);
      }
    } catch (error) {
      setNoteActionError(
        error instanceof Error ? error.message : "Archive failed",
      );
    }
  }, [navigate, refreshVault, requireActiveNoteHash]);

  const handleDeleteNote = useCallback(async () => {
    setNoteActionError(null);
    try {
      const { slug, contentHash } = requireActiveNoteHash();
      const outcome = await deleteNote(slug, contentHash);
      setNoteActionDialog(null);
      setWriteNotice(describeWriteOutcome(outcome));
      await refreshVault();
      navigate("/");
    } catch (error) {
      setNoteActionError(
        error instanceof Error ? error.message : "Delete failed",
      );
    }
  }, [navigate, refreshVault, requireActiveNoteHash]);

  return (
    <div
      className={`app-shell ${drawerOpen ? "drawer-open" : ""}`}
      style={
        {
          "--mobile-drawer-top": `${mobileDrawerTop}px`,
          "--visual-viewport-height": `${Math.round(visualViewportHeight)}px`,
          "--sidebar-width": `${sidebarWidth}px`,
        } as CSSProperties
      }
    >
      {authRequired && (
        <TokenPrompt
          onSubmit={(token) => {
            setToken(token);
            setAuthRequired(false);
            window.location.reload();
          }}
        />
      )}
      <AppTopbar
        activeNote={activeNote}
        writeEnabled={writeEnabled}
        isMobile={isMobile}
        isOnline={isOnline}
        treeIsStale={treeIsStale}
        actionsMenuOpen={actionsMenuOpen}
        topbarRef={topbarRef}
        theme={theme}
        onToggleDrawer={() => setDrawerOpen((prev) => !prev)}
        onOpenSearch={() => setSearchOpen(true)}
        onToggleActionsMenu={() => setActionsMenuOpen((prev) => !prev)}
        onCloseActionsMenu={() => setActionsMenuOpen(false)}
        onCopyPageContent={() => void copyPageContent()}
        onCopyNoteLink={() => void copyNoteLink()}
        onDownloadMarkdown={() => downloadMarkdown()}
        onEditNote={() => setEditRequestId((prev) => prev + 1)}
        onNewNote={() => openCreateDialog("")}
        onRenameNote={() => {
          setNoteActionError(null);
          setNoteActionDialog("rename");
        }}
        onMoveNote={() => {
          setNoteActionError(null);
          setNoteActionDialog("move");
        }}
        onArchiveNote={() => {
          setNoteActionError(null);
          setNoteActionDialog("archive");
        }}
        onDeleteNote={() => {
          setNoteActionError(null);
          setNoteActionDialog("delete");
        }}
        onCycleTheme={cycleTheme}
      />

      {writeWarnings.length > 0 || writeNotice ? (
        <div className="write-notice" role="status">
          <div className="write-notice-messages">
            {writeWarnings.map((warning) => (
              <span key={warning}>{warning}</span>
            ))}
            {writeNotice ? <span>{writeNotice}</span> : null}
          </div>
          <button
            type="button"
            className="write-notice-dismiss"
            aria-label="Dismiss notice"
            onClick={() => {
              setWriteNotice(null);
              setWriteWarnings([]);
            }}
          >
            ×
          </button>
        </div>
      ) : null}

      <div className="app-layout">
        <ExplorerPane
          explorerPaneRef={explorerPaneRef}
          drawerOpen={drawerOpen}
          writeEnabled={writeEnabled}
          onCreateNoteInFolder={openCreateDialog}
          locationPathname={location.pathname}
          recentNotes={recentNotes}
          modifiedNotes={modifiedNotes}
          loadingTree={loadingTree}
          treeError={treeError}
          tree={tree}
          expandedFolders={expandedFolders}
          onExpandedFoldersChange={setExpandedFolders}
          onCloseDrawer={() => setDrawerOpen(false)}
          onRefreshTree={() => {
            void loadTree();
            void loadModifiedNotes();
          }}
          onScrollTopChange={(current) => {
            window.localStorage.setItem(
              EXPLORER_SCROLL_TOP_KEY,
              String(current),
            );
          }}
        />

        {!isMobile ? (
          <div
            className="sidebar-resizer"
            role="separator"
            aria-orientation="vertical"
            aria-label="Sidebar width"
            aria-valuenow={sidebarWidth}
            aria-valuemin={220}
            aria-valuemax={420}
            tabIndex={0}
            onPointerDown={(event) => {
              resizingRef.current = {
                startX: event.clientX,
                startWidth: sidebarWidth,
              };
              document.body.classList.add("resizing");
            }}
            onKeyDown={(event) => {
              const step = event.shiftKey ? 20 : 5;
              if (event.key === "ArrowRight") {
                event.preventDefault();
                setSidebarWidth((w) => clampSidebarWidth(w + step));
              } else if (event.key === "ArrowLeft") {
                event.preventDefault();
                setSidebarWidth((w) => clampSidebarWidth(w - step));
              } else if (event.key === "Home") {
                event.preventDefault();
                setSidebarWidth(220);
              } else if (event.key === "End") {
                event.preventDefault();
                setSidebarWidth(420);
              }
            }}
          />
        ) : null}

        <main
          className={`note-pane${location.pathname === "/graph" ? " graph-host" : ""}`}
        >
          <Routes>
            <Route path="/" element={<EmptyState />} />
            <Route path="/stats" element={<StatsPage />} />
            <Route path="/graph" element={<GraphPage />} />
            <Route
              path="/n/:slug"
              element={
                <NotePage
                  onActiveNoteChange={setActiveNote}
                  onTagSelect={openSearchForTag}
                  propertiesCollapsedStorageKey={NOTE_PROPERTIES_COLLAPSED_KEY}
                  vaultRevision={vaultRevision}
                  writeEnabled={writeEnabled}
                  editRequestId={editRequestId}
                  onWriteNotice={setWriteNotice}
                  noteCandidates={noteCandidates}
                />
              }
            />
          </Routes>
        </main>
      </div>

      {isMobile && drawerOpen ? (
        <button
          className="drawer-backdrop"
          aria-label="Close explorer"
          onClick={() => setDrawerOpen(false)}
        />
      ) : null}

      {searchOpen ? (
        <SearchDialog
          query={searchQuery}
          includeContent={searchIncludeContent}
          loading={searchLoading}
          error={searchError}
          results={searchResults}
          inputRef={searchInputRef}
          onClose={() => setSearchOpen(false)}
          onQueryChange={setSearchQuery}
          onIncludeContentChange={setSearchIncludeContent}
          onSelect={(selection) => {
            setSearchOpen(false);
            setSearchQuery("");
            const params = new URLSearchParams();
            if (selection.query) {
              params.set("q", selection.query);
            }
            if (selection.matchKind) {
              params.set("m", selection.matchKind);
            }
            const suffix = params.toString();
            navigate(`/n/${selection.slug}${suffix ? `?${suffix}` : ""}`);
          }}
        />
      ) : null}

      {noteActionDialog ? (
        <NoteActionsDialog
          kind={noteActionDialog}
          error={noteActionError}
          folderPaths={folderPaths}
          initialFolder={noteActionInitialFolder}
          onClose={closeNoteActionDialog}
          onCreate={(relativePath, content) =>
            void handleCreateNote(relativePath, content)
          }
          onRename={(newTitle) => void handleRenameNote(newTitle)}
          onMove={(targetFolder) => void handleMoveNote(targetFolder)}
          onArchive={() => void handleArchiveNote()}
          onDelete={() => void handleDeleteNote()}
        />
      ) : null}
    </div>
  );
}

function EmptyState() {
  return (
    <StateBlock
      title="Notes Explorer"
      description="Select any note from the explorer to start reading."
    />
  );
}

export default App;

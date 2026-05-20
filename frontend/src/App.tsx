import {
  useCallback,
  useEffect,
  useLayoutEffect,
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
import { copyText } from "./clipboard";
import { NotePage } from "./components/NotePage";
import { SearchDialog } from "./components/SearchDialog";
import { GraphPage } from "./components/GraphPage";
import { StatsPage } from "./components/StatsPage";
import { StateBlock } from "./components/ui";
import { isExplorerTreeEqual } from "./stateCompare";
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
  const [vaultRevision, setVaultRevision] = useState(0);
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
      const res = await fetch("/api/tree");
      if (!res.ok) {
        throw new Error(`Failed loading tree: ${res.status}`);
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
      const res = await fetch(`/api/recently-modified?${params.toString()}`);
      if (!res.ok) {
        throw new Error(`Failed loading modified notes: ${res.status}`);
      }
      const json = (await res.json()) as RecentlyModifiedResponse;
      setModifiedNotes(json.notes.slice(0, 5));
    } catch {
      setModifiedNotes([]);
    }
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

    const events = new EventSource("/api/vault-events");
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

      if (event.key === "/" && !isEditableTarget(event.target)) {
        event.preventDefault();
        setSearchOpen(true);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

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
          const res = await fetch(`/api/search?${params.toString()}`);
          if (!res.ok) {
            throw new Error(`Search failed: ${res.status}`);
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
      await fetch("/api/refresh", { method: "POST" });
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
    try {
      await navigator.clipboard.writeText(window.location.href);
    } catch {
      // Ignore clipboard errors in unsupported contexts.
    }
  }, [activeNote]);
  const copyPageContent = useCallback(async () => {
    if (!activeNote) {
      return;
    }
    await copyText(activeNote.exportContent ?? "");
  }, [activeNote]);
  const toggleProperties = useCallback(() => {
    window.dispatchEvent(new Event("hatchdoor:toggle-note-properties"));
  }, []);
  const downloadMarkdown = useCallback(() => {
    if (!activeNote) {
      return;
    }
    const url = `/api/note/${encodeURIComponent(activeNote.slug)}/download`;
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.setAttribute("download", "");
    anchor.style.display = "none";
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
  }, [activeNote]);

  return (
    <div
      className={`app-shell ${drawerOpen ? "drawer-open" : ""}`}
      style={
        {
          "--mobile-drawer-top": `${mobileDrawerTop}px`,
          "--sidebar-width": `${sidebarWidth}px`,
        } as CSSProperties
      }
    >
      <AppTopbar
        activeNote={activeNote}
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
        onRefreshVault={() => void refreshVault()}
        onCopyPageContent={() => void copyPageContent()}
        onCopyNoteLink={() => void copyNoteLink()}
        onDownloadMarkdown={() => downloadMarkdown()}
        onToggleProperties={toggleProperties}
        onCycleTheme={cycleTheme}
      />

      <div className="app-layout">
        <ExplorerPane
          explorerPaneRef={explorerPaneRef}
          drawerOpen={drawerOpen}
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

        <main className={`note-pane${location.pathname === "/graph" ? " graph-host" : ""}`}>
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

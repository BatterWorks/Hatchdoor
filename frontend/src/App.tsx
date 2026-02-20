import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { Route, Routes, useLocation, useNavigate } from "react-router-dom";

import "./App.css";
import "./noteEnhancements.css";
import { FolderTree, RecentNotesList } from "./components/Explorer";
import { NotePage } from "./components/NotePage";
import { SearchDialog } from "./components/SearchDialog";
import {
  ExplorerSkeleton,
  StateBlock,
  StatusBadge,
  UiButton,
} from "./components/ui";
import { isExplorerTreeEqual } from "./stateCompare";
import type {
  ActiveNoteMeta,
  ExplorerFolder,
  RecentNote,
  SearchResponse,
  SearchResult,
} from "./types";

const SIDEBAR_WIDTH_KEY = "hatchdoor.sidebarWidth";
const DRAWER_OPEN_KEY = "hatchdoor.drawerOpen";
const RECENT_NOTES_KEY = "hatchdoor.recentNotes";
const EXPANDED_FOLDERS_KEY = "hatchdoor.expandedFolders";
const EXPLORER_SCROLL_TOP_KEY = "hatchdoor.explorerScrollTop";
const LAST_NOTE_KEY = "hatchdoor.lastNote";
const NOTE_PROPERTIES_COLLAPSED_KEY = "hatchdoor.notePropertiesCollapsed";

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
  const location = useLocation();
  const navigate = useNavigate();
  const isMobile = useIsMobile(920);
  const resizingRef = useRef<{ startX: number; startWidth: number } | null>(
    null,
  );
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const explorerPaneRef = useRef<HTMLElement | null>(null);
  const restoredExplorerScrollRef = useRef(false);
  const restoredLastNoteRef = useRef(false);

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

  useEffect(() => {
    void (async () => {
      setLoadingTree(true);
      await loadTree();
      setLoadingTree(false);
    })();
  }, [loadTree]);

  useEffect(() => {
    const id = window.setInterval(() => {
      void loadTree();
    }, 10_000);

    return () => {
      window.clearInterval(id);
    };
  }, [loadTree]);

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
    if (!searchOpen) {
      return;
    }
    const id = window.setTimeout(() => searchInputRef.current?.focus(), 0);
    return () => window.clearTimeout(id);
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
            content: String(searchIncludeContent),
            limit: "30",
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
      const next = clamp(state.startWidth + delta, 220, 420);
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
  }, [loadTree]);
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
    <div className={`app-shell ${drawerOpen ? "drawer-open" : ""}`}>
      <header className="app-topbar">
        <div className="topbar-left">
          {isMobile ? (
            <UiButton
              className="icon-button"
              onClick={() => setDrawerOpen((prev) => !prev)}
              aria-label="Toggle explorer"
            >
              ☰
            </UiButton>
          ) : null}
          <div className="topbar-context">
            <h1>{activeNote?.title ?? "Hatchdoor"}</h1>
            <p className="topbar-subtitle">
              {activeNote ? `${activeNote.relativePath}.md` : "Notes Explorer"}
            </p>
          </div>
        </div>

        <div className="topbar-center">
          {!isMobile ? (
            <UiButton
              className="topbar-search-trigger"
              onClick={() => setSearchOpen(true)}
            >
              Search
              <span className="shortcut-hint" aria-hidden="true">
                ⌘K
              </span>
            </UiButton>
          ) : null}
          {!isOnline ? <StatusBadge tone="error" text="Offline" /> : null}
          {treeIsStale ? <StatusBadge tone="warn" text="Tree Stale" /> : null}
        </div>

        <div className="topbar-right">
          {isMobile ? (
            <UiButton
              className="icon-button"
              onClick={() => setSearchOpen(true)}
              aria-label="Search notes"
            >
              <span className="topbar-search-icon" aria-hidden="true">
                ⌕
              </span>
            </UiButton>
          ) : null}
          <UiButton
            className="icon-button"
            onClick={() => setActionsMenuOpen((prev) => !prev)}
            aria-haspopup="menu"
            aria-expanded={actionsMenuOpen}
            aria-label="More actions"
          >
            ...
          </UiButton>
          {actionsMenuOpen ? (
            <div className="topbar-menu" role="menu">
              <UiButton
                className="close-note"
                role="menuitem"
                onClick={() => {
                  setActionsMenuOpen(false);
                  setSearchOpen(true);
                }}
              >
                Search
              </UiButton>
              <UiButton
                className="close-note"
                role="menuitem"
                onClick={() => {
                  setActionsMenuOpen(false);
                  void refreshVault();
                }}
              >
                Refresh vault
              </UiButton>
              {activeNote ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    setActionsMenuOpen(false);
                    void copyNoteLink();
                  }}
                >
                  Copy note link
                </UiButton>
              ) : null}
              {activeNote ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    setActionsMenuOpen(false);
                    void downloadMarkdown();
                  }}
                >
                  Download .md
                </UiButton>
              ) : null}
              <UiButton
                className="close-note"
                role="menuitem"
                onClick={() => {
                  setActionsMenuOpen(false);
                  toggleProperties();
                }}
              >
                Toggle properties
              </UiButton>
            </div>
          ) : null}
        </div>
      </header>
      {isMobile ? (
        <div className="topbar-mobile-meta">
          <button
            type="button"
            className="topbar-mobile-path"
            onClick={() => setSearchOpen(true)}
            title={
              activeNote ? `${activeNote.relativePath}.md` : "Notes Explorer"
            }
          >
            {activeNote ? `${activeNote.relativePath}.md` : "Notes Explorer"}
          </button>
        </div>
      ) : null}

      <div
        className="app-layout"
        style={{ "--sidebar-width": `${sidebarWidth}px` } as CSSProperties}
      >
        <aside
          ref={explorerPaneRef}
          className="explorer-pane"
          data-open={drawerOpen}
          onScroll={(event) => {
            const current = event.currentTarget.scrollTop;
            window.localStorage.setItem(
              EXPLORER_SCROLL_TOP_KEY,
              String(current),
            );
          }}
        >
          <header className="explorer-header">
            <p>Vault Explorer</p>
            <div className="explorer-actions">
              <UiButton className="close-note" onClick={() => void loadTree()}>
                Refresh
              </UiButton>
            </div>
          </header>

          <RecentNotesList
            notes={recentNotes}
            currentPath={location.pathname}
            onNavigate={() => setDrawerOpen(false)}
          />

          {loadingTree ? <ExplorerSkeleton /> : null}
          {!loadingTree && treeError && !tree ? (
            <StateBlock
              title="Explorer Unavailable"
              description={treeError}
              actionLabel="Retry"
              onAction={() => void loadTree()}
            />
          ) : null}
          {tree ? (
            <FolderTree
              root={tree}
              currentPath={location.pathname}
              expandedFolders={expandedFolders}
              onExpandedFoldersChange={setExpandedFolders}
            />
          ) : null}
        </aside>

        {!isMobile ? (
          <div
            className="sidebar-resizer"
            role="separator"
            aria-orientation="vertical"
            onPointerDown={(event) => {
              resizingRef.current = {
                startX: event.clientX,
                startWidth: sidebarWidth,
              };
              document.body.classList.add("resizing");
            }}
          />
        ) : null}

        <main className="note-pane">
          <Routes>
            <Route path="/" element={<EmptyState />} />
            <Route
              path="/n/:slug"
              element={
                <NotePage
                  onActiveNoteChange={setActiveNote}
                  onTagSelect={openSearchForTag}
                  propertiesCollapsedStorageKey={NOTE_PROPERTIES_COLLAPSED_KEY}
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

function useIsMobile(maxWidth: number): boolean {
  const [isMobile, setIsMobile] = useState(
    () => window.matchMedia(`(max-width: ${maxWidth}px)`).matches,
  );

  useEffect(() => {
    const query = window.matchMedia(`(max-width: ${maxWidth}px)`);
    const onChange = (event: MediaQueryListEvent) => setIsMobile(event.matches);

    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, [maxWidth]);

  return isMobile;
}

function getStoredNumber(
  key: string,
  fallback: number,
  min: number,
  max: number,
): number {
  const raw = window.localStorage.getItem(key);
  const value = raw ? Number(raw) : fallback;
  if (Number.isNaN(value)) {
    return fallback;
  }
  return clamp(value, min, max);
}

function getStoredRecentNotes(): RecentNote[] {
  try {
    const raw = window.localStorage.getItem(RECENT_NOTES_KEY);
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw) as Partial<RecentNote>[];
    return parsed
      .filter(
        (item) =>
          typeof item.slug === "string" &&
          typeof item.title === "string" &&
          typeof item.relativePath === "string" &&
          typeof item.viewedAt === "number",
      )
      .slice(0, 12)
      .map((item) => ({
        slug: item.slug as string,
        title: item.title as string,
        relativePath: item.relativePath as string,
        viewedAt: item.viewedAt as number,
      }));
  } catch {
    return [];
  }
}

function getStoredExpandedFolders(): Record<string, boolean> {
  try {
    const raw = window.localStorage.getItem(EXPANDED_FOLDERS_KEY);
    if (!raw) {
      return {};
    }
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const result: Record<string, boolean> = {};
    for (const [key, value] of Object.entries(parsed)) {
      if (key.length > 0 && typeof value === "boolean") {
        result[key] = value;
      }
    }
    return result;
  } catch {
    return {};
  }
}

function getStoredString(key: string): string | null {
  const raw = window.localStorage.getItem(key);
  if (!raw) {
    return null;
  }
  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }

  const tagName = target.tagName.toLowerCase();
  return (
    tagName === "input" ||
    tagName === "textarea" ||
    tagName === "select" ||
    target.isContentEditable
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export default App;

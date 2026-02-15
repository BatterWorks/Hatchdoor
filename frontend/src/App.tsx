import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { Route, Routes, useLocation, useNavigate } from "react-router-dom";

import "./App.css";
import { FolderTree, RecentNotesList } from "./components/Explorer";
import { NotePage } from "./components/NotePage";
import { SearchDialog } from "./components/SearchDialog";
import {
  ExplorerSkeleton,
  StateBlock,
  StatusBadge,
  UiButton,
  UiToolbar,
} from "./components/ui";
import { isExplorerTreeEqual } from "./stateCompare";
import type {
  ActiveNoteMeta,
  ExplorerFolder,
  ReadPrefs,
  RecentNote,
  SearchResponse,
  SearchResult,
} from "./types";

const SIDEBAR_WIDTH_KEY = "hatchdoor.sidebarWidth";
const DRAWER_OPEN_KEY = "hatchdoor.drawerOpen";
const READER_PREFS_KEY = "hatchdoor.readerPrefs";
const RECENT_NOTES_KEY = "hatchdoor.recentNotes";
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
  const [readPrefs, setReadPrefs] = useState<ReadPrefs>(() => {
    return getStoredReaderPrefs();
  });
  const [isOnline, setIsOnline] = useState(() => navigator.onLine);
  const [activeNote, setActiveNote] = useState<ActiveNoteMeta | null>(null);
  const [recentNotes, setRecentNotes] = useState<RecentNote[]>(() =>
    getStoredRecentNotes(),
  );
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
    window.localStorage.setItem(READER_PREFS_KEY, JSON.stringify(readPrefs));
  }, [readPrefs]);

  useEffect(() => {
    window.localStorage.setItem(RECENT_NOTES_KEY, JSON.stringify(recentNotes));
  }, [recentNotes]);

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
  const focusReaderSettings = useCallback(() => {
    const toolbar = document.querySelector(".reader-toolbar");
    if (toolbar instanceof HTMLElement) {
      toolbar.scrollIntoView({ behavior: "smooth", block: "start" });
      const select = toolbar.querySelector("select");
      if (select instanceof HTMLElement) {
        select.focus();
      }
    }
  }, []);

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
              ⌕
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
              <UiButton
                className="close-note"
                role="menuitem"
                onClick={() => {
                  setActionsMenuOpen(false);
                  focusReaderSettings();
                }}
              >
                Reader settings
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
        <aside className="explorer-pane" data-open={drawerOpen}>
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
            <FolderTree root={tree} currentPath={location.pathname} />
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

        <main
          className="note-pane"
          style={
            {
              "--reader-font-size": `${readPrefs.fontSize}px`,
              "--reader-line-height": String(readPrefs.lineHeight),
              "--reader-max-width": `${readPrefs.maxWidth}px`,
            } as CSSProperties
          }
        >
          <ReaderToolbar prefs={readPrefs} onChange={setReadPrefs} />
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
          onSelect={(slug) => {
            setSearchOpen(false);
            setSearchQuery("");
            navigate(`/n/${slug}`);
          }}
        />
      ) : null}
    </div>
  );
}

function ReaderToolbar({
  prefs,
  onChange,
}: {
  prefs: ReadPrefs;
  onChange: (next: ReadPrefs) => void;
}) {
  return (
    <UiToolbar className="reader-toolbar">
      <label>
        Size
        <select
          value={prefs.fontSize}
          onChange={(e) =>
            onChange({ ...prefs, fontSize: Number(e.target.value) })
          }
        >
          {[15, 16, 18, 20].map((size) => (
            <option key={size} value={size}>
              {size}px
            </option>
          ))}
        </select>
      </label>

      <label>
        Line
        <select
          value={prefs.lineHeight}
          onChange={(e) =>
            onChange({ ...prefs, lineHeight: Number(e.target.value) })
          }
        >
          {[1.4, 1.55, 1.7, 1.85].map((lineHeight) => (
            <option key={lineHeight} value={lineHeight}>
              {lineHeight}
            </option>
          ))}
        </select>
      </label>

      <label>
        Width
        <select
          value={prefs.maxWidth}
          onChange={(e) =>
            onChange({ ...prefs, maxWidth: Number(e.target.value) })
          }
        >
          {[720, 860, 980].map((width) => (
            <option key={width} value={width}>
              {width}px
            </option>
          ))}
        </select>
      </label>
    </UiToolbar>
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

function getStoredReaderPrefs(): ReadPrefs {
  const fallback: ReadPrefs = {
    fontSize: 16,
    lineHeight: 1.65,
    maxWidth: 860,
  };

  try {
    const raw = window.localStorage.getItem(READER_PREFS_KEY);
    if (!raw) {
      return fallback;
    }

    const parsed = JSON.parse(raw) as Partial<ReadPrefs>;
    return {
      fontSize: clamp(Number(parsed.fontSize ?? fallback.fontSize), 14, 22),
      lineHeight: clamp(
        Number(parsed.lineHeight ?? fallback.lineHeight),
        1.3,
        2.0,
      ),
      maxWidth: clamp(Number(parsed.maxWidth ?? fallback.maxWidth), 640, 1200),
    };
  } catch {
    return fallback;
  }
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

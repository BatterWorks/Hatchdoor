import {
  Children,
  isValidElement,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type CSSProperties,
  type HTMLAttributes,
  type ReactNode,
} from "react";
import ReactMarkdown from "react-markdown";
import {
  NavLink,
  Route,
  Routes,
  useLocation,
  useNavigate,
  useParams,
} from "react-router-dom";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "./App.css";
import {
  escapeMarkdownLabel,
  normalizeTags,
  parseFrontmatter,
  parseWikilinkTarget,
  type FrontmatterValue,
} from "./markdown";

type ExplorerFolder = {
  name: string;
  folders: ExplorerFolder[];
  notes: ExplorerNote[];
};

type ExplorerNote = {
  title: string;
  slug: string;
};

type Note = {
  title: string;
  slug: string;
  relative_path: string;
  content: string;
};

type ActiveNoteMeta = {
  title: string;
  slug: string;
  relativePath: string;
};

type RecentNote = ActiveNoteMeta & {
  viewedAt: number;
};

type ReadPrefs = {
  fontSize: number;
  lineHeight: number;
  maxWidth: number;
};

type ResolveBatchResponse = {
  results: Array<{
    target: string;
    slug: string | null;
  }>;
};

type SearchResult = {
  title: string;
  slug: string;
  relative_path: string;
  match_kind: string;
  snippet: string | null;
};

type SearchResponse = {
  results: SearchResult[];
};

type MermaidApi = {
  initialize: (config: {
    startOnLoad: boolean;
    securityLevel: "strict";
  }) => void;
  render: (id: string, chart: string) => Promise<{ svg: string }>;
};

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
    getStoredNumber(SIDEBAR_WIDTH_KEY, 320, 240, 520),
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
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
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
      setTree((await res.json()) as ExplorerFolder);
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
    setMobileMenuOpen(false);
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
            content: searchIncludeContent ? "1" : "0",
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
      const next = clamp(state.startWidth + delta, 240, 520);
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
          <div>
            <h1>Hatchdoor</h1>
            <p className="topbar-subtitle">
              {activeNote ? `${activeNote.relativePath}.md` : "Notes Explorer"}
            </p>
          </div>
        </div>

        <div className="topbar-center">
          {!isOnline ? <StatusBadge tone="error" text="Offline" /> : null}
          {treeIsStale ? <StatusBadge tone="warn" text="Tree Stale" /> : null}
        </div>

        <div className="topbar-right">
          <UiButton className="close-note" onClick={() => setSearchOpen(true)}>
            Search
          </UiButton>
          {isMobile ? (
            <>
              <UiButton className="close-note" onClick={() => navigate(-1)}>
                Back
              </UiButton>
              <UiButton
                className="close-note"
                onClick={() => setMobileMenuOpen((prev) => !prev)}
                aria-haspopup="menu"
                aria-expanded={mobileMenuOpen}
                aria-label="More actions"
              >
                ...
              </UiButton>
              {mobileMenuOpen ? (
                <div className="topbar-menu" role="menu">
                  <UiButton
                    className="close-note"
                    role="menuitem"
                    onClick={() => {
                      setMobileMenuOpen(false);
                      navigate(1);
                    }}
                  >
                    Forward
                  </UiButton>
                  <UiButton
                    className="close-note"
                    role="menuitem"
                    onClick={() => {
                      setMobileMenuOpen(false);
                      navigate("/");
                    }}
                  >
                    Close
                  </UiButton>
                </div>
              ) : null}
            </>
          ) : (
            <>
              <UiButton className="close-note" onClick={() => navigate(-1)}>
                Back
              </UiButton>
              <UiButton className="close-note" onClick={() => navigate(1)}>
                Forward
              </UiButton>
              <UiButton className="close-note" onClick={() => navigate("/")}>
                Close
              </UiButton>
            </>
          )}
        </div>
      </header>

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

function UiButton({
  className,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button className={`ui-button ${className ?? ""}`.trim()} {...props}>
      {children}
    </button>
  );
}

function UiPanel({
  className,
  children,
  ...props
}: HTMLAttributes<HTMLElement>) {
  return (
    <section className={`ui-panel ${className ?? ""}`.trim()} {...props}>
      {children}
    </section>
  );
}

function UiToolbar({
  className,
  children,
}: {
  className?: string;
  children: ReactNode;
}) {
  return (
    <div className={`ui-toolbar ${className ?? ""}`.trim()}>{children}</div>
  );
}

function StatusBadge({ tone, text }: { tone: "warn" | "error"; text: string }) {
  return <span className={`ui-badge status-badge ${tone}`}>{text}</span>;
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

function StateBlock({
  title,
  description,
  actionLabel,
  onAction,
}: {
  title: string;
  description: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  return (
    <UiPanel className="state-block ui-empty-state">
      <h2>{title}</h2>
      <p>{description}</p>
      {actionLabel && onAction ? (
        <UiButton className="close-note" onClick={onAction}>
          {actionLabel}
        </UiButton>
      ) : null}
    </UiPanel>
  );
}

function ExplorerSkeleton() {
  return (
    <div className="skeleton-list" aria-hidden="true">
      {Array.from({ length: 8 }).map((_, idx) => (
        <div
          key={idx}
          className="skeleton-line"
          style={{ width: `${72 - idx * 5}%` }}
        />
      ))}
    </div>
  );
}

function NoteSkeleton() {
  return (
    <div className="skeleton-list" aria-hidden="true">
      <div className="skeleton-line" style={{ width: "45%" }} />
      <div className="skeleton-line" style={{ width: "90%" }} />
      <div className="skeleton-line" style={{ width: "84%" }} />
      <div className="skeleton-line" style={{ width: "88%" }} />
      <div className="skeleton-line" style={{ width: "72%" }} />
    </div>
  );
}

function RecentNotesList({
  notes,
  currentPath,
  onNavigate,
}: {
  notes: RecentNote[];
  currentPath: string;
  onNavigate: () => void;
}) {
  if (notes.length === 0) {
    return null;
  }

  return (
    <UiPanel className="recent-notes" data-testid="recent-notes">
      <p className="recent-notes-title">Recent Notes</p>
      <ul className="tree root-tree">
        {notes.map((note) => (
          <li key={note.slug} className="note-item">
            <NavLink
              className={
                currentPath === `/n/${note.slug}`
                  ? "note-link active-note"
                  : "note-link"
              }
              to={`/n/${note.slug}`}
              onClick={onNavigate}
              title={`${note.relativePath}.md`}
            >
              {note.title}
            </NavLink>
          </li>
        ))}
      </ul>
    </UiPanel>
  );
}

function SearchDialog({
  query,
  includeContent,
  loading,
  error,
  results,
  inputRef,
  onClose,
  onQueryChange,
  onIncludeContentChange,
  onSelect,
}: {
  query: string;
  includeContent: boolean;
  loading: boolean;
  error: string | null;
  results: SearchResult[];
  inputRef: React.RefObject<HTMLInputElement | null>;
  onClose: () => void;
  onQueryChange: (value: string) => void;
  onIncludeContentChange: (value: boolean) => void;
  onSelect: (slug: string) => void;
}) {
  return (
    <div
      className="search-overlay"
      role="dialog"
      aria-modal="true"
      aria-label="Search notes"
      onClick={onClose}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          onClose();
        }
      }}
    >
      <UiPanel
        className="search-panel"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="search-header">
          <h2>Search</h2>
          <UiButton className="close-note" onClick={onClose}>
            Close
          </UiButton>
        </header>

        <input
          ref={inputRef}
          className="search-input"
          placeholder="Search notes (title, path, content)"
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
        />

        <label className="search-toggle">
          <input
            type="checkbox"
            checked={includeContent}
            onChange={(event) => onIncludeContentChange(event.target.checked)}
          />
          Include content matches
        </label>

        {loading ? <p>Searching…</p> : null}
        {error ? <p className="error">{error}</p> : null}
        {!loading &&
        !error &&
        query.trim().length >= 2 &&
        results.length === 0 ? (
          <p>No matching notes.</p>
        ) : null}

        <ul className="search-results">
          {results.map((result) => (
            <li key={`${result.slug}-${result.match_kind}`}>
              <UiButton
                className="search-result"
                onClick={() => onSelect(result.slug)}
              >
                <div className="search-main">
                  <strong>{result.title}</strong>
                  <span>{result.relative_path}.md</span>
                </div>
                <span className="search-kind">{result.match_kind}</span>
                {result.snippet ? (
                  <p className="search-snippet">{result.snippet}</p>
                ) : null}
              </UiButton>
            </li>
          ))}
        </ul>
      </UiPanel>
    </div>
  );
}

function FolderTree({
  root,
  currentPath,
}: {
  root: ExplorerFolder;
  currentPath: string;
}) {
  const currentSlug = pathToNoteSlug(currentPath);
  const [manuallyExpandedFolders, setManuallyExpandedFolders] = useState<
    Record<string, boolean>
  >({});

  const activePathFolders = useMemo(
    () => collectAncestorFolderPaths(root, currentSlug),
    [currentSlug, root],
  );

  return (
    <ul className="tree root-tree">
      {root.folders.map((folder) => (
        <FolderNode
          key={`folder-${folder.name}`}
          folder={folder}
          currentPath={currentPath}
          folderPath={folder.name}
          expandedFolders={manuallyExpandedFolders}
          activePathFolders={activePathFolders}
          onToggleFolder={(path, open) =>
            setManuallyExpandedFolders((prev) => ({ ...prev, [path]: open }))
          }
        />
      ))}
      {root.notes.map((note) => (
        <NoteNode key={note.slug} note={note} currentPath={currentPath} />
      ))}
    </ul>
  );
}

function FolderNode({
  folder,
  currentPath,
  folderPath,
  expandedFolders,
  activePathFolders,
  onToggleFolder,
}: {
  folder: ExplorerFolder;
  currentPath: string;
  folderPath: string;
  expandedFolders: Record<string, boolean>;
  activePathFolders: Set<string>;
  onToggleFolder: (path: string, open: boolean) => void;
}) {
  const shouldOpen =
    expandedFolders[folderPath] ?? activePathFolders.has(folderPath);

  return (
    <li className="folder-item">
      <details
        open={shouldOpen}
        onToggle={(event) =>
          onToggleFolder(
            folderPath,
            (event.currentTarget as HTMLDetailsElement).open,
          )
        }
      >
        <summary>{folder.name}</summary>
        <ul className="tree">
          {folder.folders.map((child) => (
            <FolderNode
              key={`${folder.name}-${child.name}`}
              folder={child}
              currentPath={currentPath}
              folderPath={`${folderPath}/${child.name}`}
              expandedFolders={expandedFolders}
              activePathFolders={activePathFolders}
              onToggleFolder={onToggleFolder}
            />
          ))}
          {folder.notes.map((note) => (
            <NoteNode key={note.slug} note={note} currentPath={currentPath} />
          ))}
        </ul>
      </details>
    </li>
  );
}

function pathToNoteSlug(pathname: string): string | null {
  const match = pathname.match(/^\/n\/([^/]+)$/);
  if (!match) {
    return null;
  }

  return decodeURIComponent(match[1]);
}

function collectAncestorFolderPaths(
  root: ExplorerFolder,
  slug: string | null,
): Set<string> {
  const paths = new Set<string>();
  if (!slug) {
    return paths;
  }

  const visit = (folder: ExplorerFolder, folderPath: string): boolean => {
    if (folder.notes.some((note) => note.slug === slug)) {
      paths.add(folderPath);
      return true;
    }

    for (const child of folder.folders) {
      const childPath = `${folderPath}/${child.name}`;
      if (visit(child, childPath)) {
        paths.add(folderPath);
        return true;
      }
    }

    return false;
  };

  for (const folder of root.folders) {
    visit(folder, folder.name);
  }

  return paths;
}

function NoteNode({
  note,
  currentPath,
}: {
  note: ExplorerNote;
  currentPath: string;
}) {
  return (
    <li className="note-item">
      <NavLink
        className={
          currentPath === `/n/${note.slug}`
            ? "note-link active-note"
            : "note-link"
        }
        to={`/n/${note.slug}`}
        title={`${note.title}.md`}
      >
        {note.title}
      </NavLink>
    </li>
  );
}

function NotePage({
  onActiveNoteChange,
  onTagSelect,
}: {
  onActiveNoteChange: (meta: ActiveNoteMeta | null) => void;
  onTagSelect: (tag: string) => void;
}) {
  const params = useParams<{ slug: string }>();
  const slug = params.slug ?? "";
  const [note, setNote] = useState<Note | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [propertiesCollapsed, setPropertiesCollapsed] = useState<boolean>(
    () => {
      return window.localStorage.getItem(NOTE_PROPERTIES_COLLAPSED_KEY) !== "0";
    },
  );

  const loadNote = useCallback(
    async (hardReload: boolean) => {
      setError(null);
      if (hardReload) {
        setNote(null);
      }

      try {
        const res = await fetch(`/api/note/${encodeURIComponent(slug)}`);
        if (!res.ok) {
          throw new Error(`Failed loading note: ${res.status}`);
        }
        const json = (await res.json()) as { note: Note };
        setNote(json.note);
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Unknown note loading error",
        );
      }
    },
    [slug],
  );

  useEffect(() => {
    void (async () => {
      setLoading(true);
      await loadNote(true);
      setLoading(false);
    })();
  }, [loadNote]);

  useEffect(() => {
    const id = window.setInterval(() => {
      void loadNote(false);
    }, 10_000);

    return () => {
      window.clearInterval(id);
    };
  }, [loadNote]);

  useEffect(() => {
    if (!note) {
      onActiveNoteChange(null);
      return;
    }

    onActiveNoteChange({
      title: note.title,
      slug: note.slug,
      relativePath: note.relative_path,
    });
  }, [note, onActiveNoteChange]);

  useEffect(() => {
    window.localStorage.setItem(
      NOTE_PROPERTIES_COLLAPSED_KEY,
      propertiesCollapsed ? "1" : "0",
    );
  }, [propertiesCollapsed]);

  const parsed = useMemo(() => parseFrontmatter(note?.content ?? ""), [note]);
  const markdown = useResolvedWikilinks(parsed.body);

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

  return (
    <article className="note-content">
      <h2>{note.title}</h2>
      {error ? <StatusBadge tone="warn" text="Showing cached note" /> : null}
      <NoteProperties
        properties={parsed.properties}
        collapsed={propertiesCollapsed}
        onToggleCollapsed={() => setPropertiesCollapsed((prev) => !prev)}
        onTagSelect={onTagSelect}
      />
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        components={{
          pre(props) {
            const first = Children.toArray(props.children)[0];
            if (
              isValidElement<{ className?: string }>(first) &&
              first.type !== "code"
            ) {
              return first;
            }
            return <pre>{props.children}</pre>;
          },
          code(props) {
            const { children, className } = props;
            const content = String(children).replace(/\n$/, "");
            const match = /language-(\w+)/.exec(className || "");

            if (match?.[1] === "mermaid") {
              return <MermaidDiagram chart={content} />;
            }

            if (!match) {
              return <code className={className}>{children}</code>;
            }

            return <CodeBlock language={match[1]} content={content} />;
          },
          a(props) {
            const { href, children } = props;
            if (typeof href === "string" && href.startsWith("/__missing__/")) {
              const target = decodeURIComponent(
                href.replace("/__missing__/", ""),
              );
              return (
                <span className="broken-link" title={`Missing: ${target}`}>
                  {children}
                </span>
              );
            }
            return <a href={href}>{children}</a>;
          },
          blockquote(props) {
            return <CalloutOrQuote>{props.children}</CalloutOrQuote>;
          },
        }}
      >
        {markdown}
      </ReactMarkdown>
    </article>
  );
}

function NoteProperties({
  properties,
  collapsed,
  onToggleCollapsed,
  onTagSelect,
}: {
  properties: Record<string, FrontmatterValue>;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  onTagSelect: (tag: string) => void;
}) {
  const entries = Object.entries(properties);
  if (entries.length === 0) {
    return null;
  }

  return (
    <section className="note-properties" data-collapsed={collapsed}>
      <header className="note-properties-head">
        <h3>Properties</h3>
        <UiButton
          className="close-note"
          onClick={onToggleCollapsed}
          aria-expanded={!collapsed}
          aria-controls="note-properties-grid"
        >
          {collapsed ? "Show" : "Hide"}
        </UiButton>
      </header>

      {!collapsed ? (
        <dl id="note-properties-grid" className="note-properties-grid">
          {entries.map(([key, value]) => (
            <div key={key} className="note-property-row">
              <dt>{key}</dt>
              <dd>
                {key === "tags" ? (
                  <TagChips
                    tags={normalizeTags(value)}
                    onSelect={onTagSelect}
                  />
                ) : (
                  <PropertyValue value={value} />
                )}
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
    </section>
  );
}

function PropertyValue({ value }: { value: FrontmatterValue }) {
  if (Array.isArray(value)) {
    return <span>{value.join(", ")}</span>;
  }
  return <span>{value}</span>;
}

function TagChips({
  tags,
  onSelect,
}: {
  tags: string[];
  onSelect: (tag: string) => void;
}) {
  if (tags.length === 0) {
    return <span>None</span>;
  }

  return (
    <div className="tag-chip-list">
      {tags.map((tag) => (
        <button
          type="button"
          key={tag}
          className="tag-chip"
          onClick={() => onSelect(tag)}
          title={`Search tag: ${tag}`}
        >
          #{tag}
        </button>
      ))}
    </div>
  );
}

function useResolvedWikilinks(markdown: string): string {
  const [resolved, setResolved] = useState(markdown);

  useEffect(() => {
    let cancelled = false;

    if (!markdown) {
      queueMicrotask(() => setResolved(""));
      return;
    }

    void (async () => {
      const matches = [...markdown.matchAll(/\[\[([^\]]+)\]\]/g)];
      const rawTargets = matches
        .map((m) => parseWikilinkTarget(m[1]).target)
        .filter((target) => target.length > 0);
      const uniqueTargets = [...new Set(rawTargets)];

      const map = new Map<string, string | null>();

      if (uniqueTargets.length > 0) {
        try {
          const res = await fetch("/api/resolve-batch", {
            method: "POST",
            headers: {
              "Content-Type": "application/json",
            },
            body: JSON.stringify({ targets: uniqueTargets }),
          });

          if (res.ok) {
            const json = (await res.json()) as ResolveBatchResponse;
            for (const result of json.results) {
              map.set(result.target, result.slug);
            }
          }
        } catch {
          // Leave unresolved values as null fallback.
        }
      }

      const rewritten = markdown.replace(
        /\[\[([^\]]+)\]\]/g,
        (_whole, body: string) => {
          const parsed = parseWikilinkTarget(body);
          const slug = map.get(parsed.target) ?? null;
          if (slug) {
            return `[${escapeMarkdownLabel(parsed.label)}](/n/${slug})`;
          }
          return `[${escapeMarkdownLabel(parsed.label)}](/__missing__/${encodeURIComponent(parsed.target)})`;
        },
      );

      if (!cancelled) {
        setResolved(rewritten);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [markdown]);

  return resolved;
}

function CalloutOrQuote({ children }: { children: ReactNode }) {
  const nodes = Children.toArray(children);
  const firstContentIndex = nodes.findIndex(
    (node) => !(typeof node === "string" && node.trim().length === 0),
  );

  if (firstContentIndex === -1) {
    return <blockquote>{children}</blockquote>;
  }

  const first = nodes[firstContentIndex];

  if (isValidElement<{ children?: ReactNode }>(first) && first.type === "p") {
    const firstText = flattenText(first.props.children).trim();
    const match = firstText.match(/^\[!([A-Za-z0-9_]+)\]([+-])?\s*(.*)$/);

    if (match) {
      const kind = match[1].toLowerCase();
      const fold = match[2] ?? null;
      const title = match[3] || kind[0].toUpperCase() + kind.slice(1);
      const bodyNodes = nodes
        .slice(firstContentIndex + 1)
        .filter(
          (node) => !(typeof node === "string" && node.trim().length === 0),
        );

      if (fold) {
        return (
          <details
            className={`callout callout-${kind} callout-collapsible`}
            open={fold === "+"}
          >
            <summary className="callout-title">{title}</summary>
            <div className="callout-body">{bodyNodes}</div>
          </details>
        );
      }

      return (
        <div className={`callout callout-${kind}`}>
          <div className="callout-title">{title}</div>
          <div className="callout-body">{bodyNodes}</div>
        </div>
      );
    }
  }

  return <blockquote>{children}</blockquote>;
}

function flattenText(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }
  if (!node) {
    return "";
  }
  if (Array.isArray(node)) {
    return node.map(flattenText).join("");
  }
  if (isValidElement<{ children?: ReactNode }>(node)) {
    return flattenText(node.props.children);
  }
  return "";
}

function CodeBlock({
  language,
  content,
}: {
  language: string;
  content: string;
}) {
  const [copied, setCopied] = useState(false);

  const onCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
    }
  }, [content]);

  return (
    <div className="code-block">
      <div className="code-block-head">
        <span className="code-lang">{language}</span>
        <UiButton className="close-note" onClick={() => void onCopy()}>
          {copied ? "Copied" : "Copy"}
        </UiButton>
      </div>
      <pre>
        <code>{content}</code>
      </pre>
    </div>
  );
}

function MermaidDiagram({ chart }: { chart: string }) {
  const [svg, setSvg] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;

    void (async () => {
      try {
        const mermaidModule = (await import("mermaid")) as {
          default: MermaidApi;
        };
        const api = mermaidModule.default;
        api.initialize({ startOnLoad: false, securityLevel: "strict" });
        const id = `m-${Math.random().toString(36).slice(2)}`;
        const { svg: rendered } = await api.render(id, chart);
        if (mounted) {
          setSvg(rendered);
          setError(null);
        }
      } catch (err) {
        if (mounted) {
          setError(
            err instanceof Error ? err.message : "Invalid mermaid diagram",
          );
        }
      }
    })();

    return () => {
      mounted = false;
    };
  }, [chart]);

  if (error) {
    return <pre className="error">Mermaid error: {error}</pre>;
  }

  return <div className="mermaid" dangerouslySetInnerHTML={{ __html: svg }} />;
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

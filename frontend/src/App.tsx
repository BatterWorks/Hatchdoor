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
  RECENT_NOTES_COLLAPSED_KEY,
  EXPANDED_FOLDERS_KEY,
  LAST_NOTE_KEY,
  NOTE_PROPERTIES_COLLAPSED_KEY,
  RECENT_NOTES_KEY,
  SCOPE_ZONE_COLLAPSED_KEY,
  SIDEBAR_WIDTH_KEY,
} from "./app/constants";
import { ExplorerPane } from "./app/ExplorerPane";
import {
  clampSidebarWidth,
  getStoredNumber,
  getStoredRecentNotes,
  getStoredLastNote,
  getStoredExpandedFolders,
  isEditableTarget,
} from "./lib/storage";
import { useIsMobile } from "./hooks/useIsMobile";
import { useTheme } from "./hooks/useTheme";
import { onUnauthorized, setToken, withAccessToken } from "./api/api";
import { copyText } from "./lib/clipboard";
import { NoteActionsDialog } from "./components/NoteActionsDialog";
import { NotePage } from "./components/NotePage";
import { TokenPrompt } from "./components/TokenPrompt";
import { GraphPage } from "./components/graph/GraphPage";
import { SettingsPage } from "./features/settings/SettingsPage";
import { StatsPage } from "./components/StatsPage";
import { StateBlock } from "./components/ui";
import { useNoteActions } from "./hooks/useNoteActions";
import { useVaultTree } from "./hooks/useVaultTree";
import {
  resolvePrimaryVaultId,
  useVaultDiscovery,
  useVaultNoteCounts,
  useVaultScope,
} from "./hooks/useVaultScope";
import { useWriteMode } from "./hooks/useWriteMode";
import { pruneNoteDrafts } from "./lib/writeDrafts";
import type { ActiveNoteMeta, RecentNote } from "./types";
import { StartupGate } from "./startup/StartupGate";
import { SearchDialog, useSearch } from "./features/search";

export function VaultApp() {
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
  const [actionsMenuOpen, setActionsMenuOpen] = useState(false);
  const [mobileDrawerTop, setMobileDrawerTop] = useState(0);
  const [visualViewportHeight, setVisualViewportHeight] = useState(
    () => window.visualViewport?.height ?? window.innerHeight,
  );
  const [editRequestId, setEditRequestId] = useState(0);
  const location = useLocation();
  const navigate = useNavigate();
  const isMobile = useIsMobile(920);
  const { theme, cycleTheme } = useTheme();

  const [scope, setScope] = useVaultScope();
  const { vaults, demoMode } = useVaultDiscovery();
  const primaryVaultId = resolvePrimaryVaultId(activeNote?.vaultId, vaults);

  const {
    tree,
    vaultTrees,
    loadingTree,
    treeError,
    modifiedNotes,
    modifiedNotesPartial,
    modifiedNotesMissingVaults,
    vaultRevision,
    folderPaths,
    noteCandidates,
    loadTree,
    loadModifiedNotes,
  } = useVaultTree(scope);
  const vaultNoteCounts = useVaultNoteCounts(vaults.length > 1, vaultRevision);
  const refreshVault = useCallback(async () => {
    await loadTree();
    await loadModifiedNotes();
  }, [loadModifiedNotes, loadTree]);
  const {
    writeEnabled,
    writeWarnings,
    setWriteWarnings,
    writeNotice,
    setWriteNotice,
  } = useWriteMode(primaryVaultId);
  const settingsEnabled = !demoMode;
  const {
    searchOpen,
    setSearchOpen,
    searchQuery,
    setSearchQuery,
    searchIncludeContent,
    setSearchIncludeContent,
    searchResults,
    searchPartial,
    searchMissingVaultNames,
    searchParticipants,
    searchInitialVaultFilter,
    searchLoading,
    searchError,
    searchInputRef,
    openSearchForTag,
  } = useSearch(scope);
  const {
    noteActionDialog,
    noteActionError,
    noteActionInitialFolder,
    openCreateDialog,
    openActionDialog,
    closeNoteActionDialog,
    handleCreateNote,
    handleRenameNote,
    handleMoveNote,
    handleArchiveNote,
    handleDeleteNote,
  } = useNoteActions({ activeNote, vaults, refreshVault, setWriteNotice });

  const resizingRef = useRef<{ startX: number; startWidth: number } | null>(
    null,
  );
  const topbarRef = useRef<HTMLElement | null>(null);
  const explorerScrollRef = useRef<HTMLElement | null>(null);
  // Recently viewed remembers whether it is folded away; the design is
  // explicit that leaving it closed is a fine way to use the sidebar.
  const [recentCollapsed, setRecentCollapsed] = useState<boolean>(
    () => window.localStorage.getItem(RECENT_NOTES_COLLAPSED_KEY) === "1",
  );
  // The Scope zone remembers whether it is folded away, same as Recently
  // viewed; default expanded per the design spec.
  const [scopeZoneCollapsed, setScopeZoneCollapsed] = useState<boolean>(
    () => window.localStorage.getItem(SCOPE_ZONE_COLLAPSED_KEY) === "1",
  );
  const restoredExplorerScrollRef = useRef(false);
  const restoredLastNoteRef = useRef(false);

  useEffect(() => {
    // Drafts only bridge an interrupted edit; drop ones older than a week.
    pruneNoteDrafts(7 * 24 * 60 * 60 * 1000);
  }, []);

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
    // Reset transient shell UI on navigation — an accepted effect→setState
    // pattern (the state is not derivable from render inputs alone).
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
        (item) =>
          !(
            item.vaultId === activeNote.vaultId && item.slug === activeNote.slug
          ),
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
    window.localStorage.setItem(
      LAST_NOTE_KEY,
      JSON.stringify({ vaultId: activeNote.vaultId, slug: activeNote.slug }),
    );
  }, [activeNote]);

  useEffect(() => {
    if (restoredLastNoteRef.current || location.pathname !== "/") {
      return;
    }
    restoredLastNoteRef.current = true;
    // Malformed or pre-#137 slug-only stored state resolves to null and is
    // ignored, same as before.
    const last = getStoredLastNote();
    if (!last) {
      return;
    }
    navigate(
      `/v/${encodeURIComponent(last.vaultId)}/n/${encodeURIComponent(last.slug)}`,
      { replace: true },
    );
  }, [location.pathname, navigate]);

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
  }, [openCreateDialog, setSearchOpen, writeEnabled]);

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
    const container = explorerScrollRef.current;
    if (!container) {
      return;
    }
    const stored = getStoredNumber(EXPLORER_SCROLL_TOP_KEY, 0, 0, 1_000_000);
    container.scrollTop = stored;
    restoredExplorerScrollRef.current = true;
  }, [tree]);

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
      `/api/v1/vaults/${encodeURIComponent(activeNote.vaultId)}/notes/${encodeURIComponent(activeNote.slug)}/download`,
    );
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
          "--visual-viewport-height": `${Math.round(visualViewportHeight)}px`,
          "--sidebar-width": `${sidebarWidth}px`,
        } as CSSProperties
      }
    >
      <AppTopbar
        activeNote={activeNote}
        vaults={vaults}
        scope={scope}
        writeEnabled={writeEnabled}
        isMobile={isMobile}
        isOnline={isOnline}
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
        onRenameNote={() => openActionDialog("rename")}
        onMoveNote={() => openActionDialog("move")}
        onArchiveNote={() => openActionDialog("archive")}
        onDeleteNote={() => openActionDialog("delete")}
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
          explorerScrollRef={explorerScrollRef}
          drawerOpen={drawerOpen}
          isMobile={isMobile}
          writeEnabled={writeEnabled}
          settingsEnabled={settingsEnabled}
          onCreateNoteInFolder={openCreateDialog}
          locationPathname={location.pathname}
          recentNotes={recentNotes}
          modifiedNotes={modifiedNotes}
          modifiedNotesPartial={modifiedNotesPartial}
          modifiedNotesMissingVaults={modifiedNotesMissingVaults}
          loadingTree={loadingTree}
          treeError={treeError}
          tree={tree}
          vaultTrees={vaultTrees}
          expandedFolders={expandedFolders}
          recentCollapsed={recentCollapsed}
          onRecentCollapsedChange={(next) => {
            setRecentCollapsed(next);
            window.localStorage.setItem(
              RECENT_NOTES_COLLAPSED_KEY,
              next ? "1" : "0",
            );
          }}
          vaults={vaults}
          scope={scope}
          onScopeChange={setScope}
          viewingVaultId={activeNote?.vaultId}
          vaultNoteCounts={vaultNoteCounts}
          scopeZoneCollapsed={scopeZoneCollapsed}
          onScopeZoneCollapsedChange={(next) => {
            setScopeZoneCollapsed(next);
            window.localStorage.setItem(
              SCOPE_ZONE_COLLAPSED_KEY,
              next ? "1" : "0",
            );
          }}
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
            <Route path="/settings" element={<SettingsPage />} />
            <Route
              path="/v/:vaultId/n/:slug"
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
                  vaults={vaults}
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
          partial={searchPartial}
          missingVaultNames={searchMissingVaultNames}
          participants={searchParticipants}
          initialVaultFilter={searchInitialVaultFilter}
          vaults={vaults}
          scope={scope}
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
            navigate(
              `/v/${encodeURIComponent(selection.vaultId)}/n/${selection.slug}${suffix ? `?${suffix}` : ""}`,
            );
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

export function App() {
  const [authRequired, setAuthRequired] = useState(false);

  useEffect(() => {
    onUnauthorized(() => setAuthRequired(true));
    return () => onUnauthorized(null);
  }, []);

  return (
    <>
      {authRequired ? (
        <TokenPrompt
          onSubmit={(token) => {
            setToken(token);
            setAuthRequired(false);
            window.location.reload();
          }}
        />
      ) : null}
      <StartupGate>
        <VaultApp />
      </StartupGate>
    </>
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

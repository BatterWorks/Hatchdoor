import {
  useCallback,
  useMemo,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import {
  Navigate,
  Route,
  Routes,
  useLocation,
  useMatch,
  useNavigate,
} from "react-router-dom";

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
import { ExplorerPane, type StartupProgress } from "./app/ExplorerPane";
import {
  clampSidebarWidth,
  getStoredNumber,
  getStoredRecentNotes,
  clearStoredLastNote,
  getStoredLastNote,
  getStoredLastNoteForVault,
  getStoredExpandedFolders,
  isEditableTarget,
  pruneStoredLastNotesByVault,
  rememberLastNoteForVault,
  safeGetItem,
  safeSetItem,
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
import { resolvePrimaryVaultId, useVaultScope } from "./hooks/useVaultScope";
import { useVaultCollection, useVaultProjection } from "./vaults";
import { scopeName } from "./app/vaultSlotLogic";
import { useWriteMode } from "./hooks/useWriteMode";
import { pruneNoteDrafts } from "./lib/writeDrafts";
import { isDemoReadOnlyError } from "./api/writeApi";
import type { ActiveNoteMeta, RecentNote, VaultScope } from "./types";
import { StartupGate } from "./startup/StartupGate";
import {
  useStartupStatus,
  type StartupStatus,
} from "./startup/useStartupStatus";
import {
  CONTEXTUAL_HELP,
  ContextualHelpLink,
  type HelpLocation,
  HelpProvider,
  useHelp,
} from "./features/help";
import { SearchDialog, useSearch } from "./features/search";
import {
  fetchFreshInstall,
  FirstRunChecklist,
  recordSearchResults,
  reopenFirstRun,
  shouldShowFirstRun,
  useFirstRunState,
} from "./features/first-run";
import { UpdateBanner } from "./features/update-banner";
import { WhatsNew } from "./features/whats-new";

function VaultWorkspace({
  startupStatus,
  onRetryModelSetup,
}: {
  startupStatus: StartupStatus | null;
  onRetryModelSetup: () => void;
}) {
  const [drawerOpen, setDrawerOpen] = useState<boolean>(() => {
    return safeGetItem(DRAWER_OPEN_KEY) === "1";
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
  const [scopeSheetOpen, setScopeSheetOpen] = useState(false);
  const [mobileDrawerTop, setMobileDrawerTop] = useState(0);
  const [visualViewportHeight, setVisualViewportHeight] = useState(
    () => window.visualViewport?.height ?? window.innerHeight,
  );
  const [editRequestId, setEditRequestId] = useState(0);
  const location = useLocation();
  const navigate = useNavigate();
  // The one route that shows a note, and so the one state a scope switch is
  // allowed to navigate out of. Matched by the router itself rather than by a
  // second spelling of the path.
  const onNoteRoute = useMatch("/v/:vaultId/n/:slug") !== null;
  const isMobile = useIsMobile(920);
  const { theme, cycleTheme } = useTheme();
  const help = useHelp();

  const [scope, setScope, scopeFallbackNotice] = useVaultScope();
  const {
    vaults,
    demoMode,
    loading: vaultsLoading,
    readState: collectionReadState,
    error: collectionError,
    recovery: registryRecovery,
    noteCounts: vaultNoteCounts,
    revision: collectionRevision,
    refresh: loadVaults,
  } = useVaultCollection();
  const vaultProjection = useVaultProjection();
  const hasRegistryRecovery = Boolean(registryRecovery);
  const primaryVaultId = resolvePrimaryVaultId(activeNote?.vaultId, vaults);

  const {
    tree,
    vaultTrees,
    loadingTree,
    treeError,
    treePartial,
    treeMissingVaults,
    modifiedNotes,
    modifiedNotesPartial,
    modifiedNotesMissingVaults,
    modifiedNotesError,
    vaultRevision,
    folderPathsByVault,
    noteCandidates,
    loadTree,
    loadModifiedNotes,
  } = useVaultTree(scope);
  // Every enabled Vault, whatever the browsing scope: a note can be created in
  // a Vault that is not currently being browsed, and the picker says so.
  const dialogVaults = useMemo(
    () =>
      vaults.map((vault) => ({
        vaultId: vault.vault_id,
        name: vault.name,
      })),
    [vaults],
  );
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
    recheck: recheckWriteMode,
  } = useWriteMode(primaryVaultId, {
    demoMode,
    revision: collectionRevision,
  });
  // A stored scope whose Vault left the browsing list has already been put
  // back to All Vaults by `useVaultScope` (#335); the shared notice strip says
  // why, so the explorer widening on its own is not a mystery.
  useEffect(() => {
    if (scopeFallbackNotice) {
      setWriteNotice(scopeFallbackNotice.message);
    }
  }, [scopeFallbackNotice, setWriteNotice]);
  // `demoMode` defaults to `false` until Vault discovery's fetch resolves
  // (#152) — the same gap the "/settings" route itself guards below.
  // Without `!vaultsLoading` here, the sidebar footer's Settings link would
  // render and stay clickable for that entire fetch on a demo instance.
  const settingsEnabled = !vaultsLoading && !demoMode;
  // The first-run checklist (#419): on a fresh install (#424) until closed,
  // or once reopened from Help, and never in demo mode.
  const firstRun = useFirstRunState();
  const [freshInstall, setFreshInstall] = useState(false);
  useEffect(() => {
    if (!settingsEnabled) return;
    const controller = new AbortController();
    void fetchFreshInstall(controller.signal).then((fresh) => {
      if (!controller.signal.aborted) setFreshInstall(fresh);
    });
    return () => controller.abort();
  }, [settingsEnabled]);
  const showFirstRun =
    settingsEnabled &&
    shouldShowFirstRun({
      demoMode,
      freshInstall,
      dismissed: firstRun.dismissed,
      reopened: firstRun.reopened,
    });
  // A demo_read_only refusal is the one write error rendered in the app's
  // own words rather than the server's (#152). `writeEnabled` already stays
  // false once the collection knows it is on a demo instance, and re-derives
  // when the backend flips into demo mode (#339), so every write affordance
  // this flag gates (New note, Edit, attachment drop) disappears on its own.
  // This handler is the backstop for a write that reaches the server before
  // the shell has noticed: one sentence in the shared notice strip, no retry,
  // and the app re-asks the server what it is permitted to do — both the
  // collection (which carries `demo_mode`) and `write-capabilities` itself.
  const handleDemoRefusal = useCallback(
    (error: unknown): boolean => {
      if (!isDemoReadOnlyError(error)) {
        return false;
      }
      setWriteNotice(
        "This is a public read-only demo, so that change was not saved.",
      );
      void loadVaults();
      recheckWriteMode();
      return true;
    },
    [loadVaults, recheckWriteMode, setWriteNotice],
  );
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
  } = useSearch();
  // A search that found something proves the notes are indexed, which ticks
  // the checklist's last step in this browser.
  useEffect(() => {
    if (!searchLoading && !searchError && searchResults.length > 0) {
      recordSearchResults(searchQuery, searchResults.length);
    }
  }, [searchLoading, searchError, searchResults, searchQuery]);
  const {
    noteActionDialog,
    noteActionError,
    noteActionInitialFolder,
    noteActionInitialVaultId,
    openCreateDialog,
    openActionDialog,
    closeNoteActionDialog,
    handleCreateNote,
    restoreCreateDraft,
    handleRenameNote,
    handleMoveNote,
    handleArchiveNote,
    handleDeleteNote,
  } = useNoteActions({
    activeNote,
    vaults,
    refreshVault,
    setWriteNotice,
    onDemoRefusal: handleDemoRefusal,
  });

  const resizingRef = useRef<{ startX: number; startWidth: number } | null>(
    null,
  );
  const topbarRef = useRef<HTMLElement | null>(null);
  const explorerScrollRef = useRef<HTMLElement | null>(null);
  // Recently viewed remembers whether it is folded away; the design is
  // explicit that leaving it closed is a fine way to use the sidebar.
  const [recentCollapsed, setRecentCollapsed] = useState<boolean>(
    () => safeGetItem(RECENT_NOTES_COLLAPSED_KEY) === "1",
  );
  // The Scope zone remembers whether it is folded away, same as Recently
  // viewed; default expanded per the design spec.
  const [scopeZoneCollapsed, setScopeZoneCollapsed] = useState<boolean>(
    () => safeGetItem(SCOPE_ZONE_COLLAPSED_KEY) === "1",
  );
  const restoredExplorerScrollRef = useRef(false);
  const restoredLastNoteRef = useRef(false);
  // The `v` shortcut's keyboard-accessible route to the Scope zone/sheet
  // (#146). `scopeFocusRequestId` is a bare counter: bumping it asks
  // whichever surface is on screen to (re)focus its current scope row.
  // `scopeShortcutOriginRef` remembers where focus was before the zone/sheet
  // opened, so `Escape` without picking can give it back.
  const [scopeFocusRequestId, setScopeFocusRequestId] = useState(0);
  const scopeShortcutOriginRef = useRef<HTMLElement | null>(null);
  const [scopeLiveMessage, setScopeLiveMessage] = useState("");
  const announcedScopeRef = useRef<VaultScope | null>(null);
  const announcedScopeSlotRef = useRef(false);

  useEffect(() => {
    // Drafts only bridge an interrupted edit; drop ones older than a week.
    pruneNoteDrafts(7 * 24 * 60 * 60 * 1000);
  }, []);

  useEffect(() => {
    safeSetItem(DRAWER_OPEN_KEY, drawerOpen && isMobile ? "1" : "0");
  }, [drawerOpen, isMobile]);

  useEffect(() => {
    safeSetItem(SIDEBAR_WIDTH_KEY, String(sidebarWidth));
  }, [sidebarWidth]);

  useEffect(() => {
    safeSetItem(RECENT_NOTES_KEY, JSON.stringify(recentNotes));
  }, [recentNotes]);

  useEffect(() => {
    safeSetItem(EXPANDED_FOLDERS_KEY, JSON.stringify(expandedFolders));
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
    setScopeSheetOpen(false);
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
    safeSetItem(
      LAST_NOTE_KEY,
      JSON.stringify({ vaultId: activeNote.vaultId, slug: activeNote.slug }),
    );
    // The same note again, filed under its own Vault: the landing redirect
    // above needs one note, a scope switch needs one per Vault.
    rememberLastNoteForVault(activeNote.vaultId, activeNote.slug);
  }, [activeNote]);

  useEffect(() => {
    if (restoredLastNoteRef.current || location.pathname !== "/") {
      return;
    }
    // Discovery still in flight means `vaults` is a temporary `[]`, which
    // would read as "the stored Vault is gone" for every stored note. Wait for
    // the real list before judging it. A failed discovery leaves the same
    // `[]` behind it, and judging that would forget the note over a network
    // error (#333): wait for a discovery that actually answered, which the
    // Try again action or the revision stream's reconnect delivers.
    if (
      vaultsLoading ||
      collectionReadState === "loading" ||
      collectionReadState === "error"
    ) {
      return;
    }
    restoredLastNoteRef.current = true;
    // Malformed or pre-#137 slug-only stored state resolves to null and is
    // ignored, same as before.
    const last = getStoredLastNote();
    if (!last) {
      return;
    }
    // A Vault that has left the collection cannot be restored into: the note
    // route resolves to "Vault definition was not found", and because the
    // landing redirect runs again on every visit, the reader is pinned to that
    // error with no way back short of editing the URL. Forget it instead.
    if (!vaults.some((vault) => vault.vault_id === last.vaultId)) {
      clearStoredLastNote();
      return;
    }
    navigate(
      `/v/${encodeURIComponent(last.vaultId)}/n/${encodeURIComponent(last.slug)}`,
      { replace: true },
    );
  }, [collectionReadState, location.pathname, navigate, vaults, vaultsLoading]);

  useEffect(() => {
    // A departed Vault's remembered note is unusable for the same reason the
    // landing restore forgets one: the note route answers "Vault definition
    // was not found". Dropping it here also keeps the map from holding an
    // entry per Vault ever connected. An empty browsing list is never
    // evidence of that — a broken registry and a paused-everything
    // collection both produce one — so it forgets nothing at all rather than
    // everything.
    if (
      vaultsLoading ||
      collectionReadState === "error" ||
      hasRegistryRecovery ||
      vaults.length === 0
    ) {
      return;
    }
    pruneStoredLastNotesByVault(vaults.map((vault) => vault.vault_id));
  }, [collectionReadState, hasRegistryRecovery, vaults, vaultsLoading]);

  // Narrowing the browsing scope to one Vault carries the reader with it: the
  // note that Vault was last left on comes back, the same restore the landing
  // redirect does on a fresh load, and a Vault with nothing remembered lands
  // on the empty state rather than leaving the previous Vault's note on
  // screen. Only from a note page — a scope pick made in Settings, on the
  // Graph or in Statistics is a filter, not a request to go and read
  // something. Widening back to `all` moves nobody: it adds Vaults to what is
  // listed, it does not choose one.
  const handleScopeChange = useCallback(
    (next: VaultScope) => {
      setScope(next);
      if (
        next === scope ||
        next === "all" ||
        activeNote?.vaultId === next ||
        !onNoteRoute
      ) {
        return;
      }
      const slug = getStoredLastNoteForVault(next);
      if (!slug) {
        // Nothing is open any more, so nothing should be restored: forgetting
        // the landing note keeps the empty state on screen both now (the
        // landing redirect finds nothing to put back) and after a reload,
        // which would otherwise return the note of the Vault just left while
        // the selector still reads the new one.
        clearStoredLastNote();
        navigate("/");
        return;
      }
      navigate(`/v/${encodeURIComponent(next)}/n/${encodeURIComponent(slug)}`);
    },
    [activeNote?.vaultId, navigate, onNoteRoute, scope, setScope],
  );

  const handleScopeZoneCollapsedChange = useCallback((next: boolean) => {
    setScopeZoneCollapsed(next);
    safeSetItem(SCOPE_ZONE_COLLAPSED_KEY, next ? "1" : "0");
  }, []);

  // Give focus back to wherever `v` was pressed (#146) — read once, then
  // cleared, so a later `Escape` with no shortcut in flight is a no-op.
  const restoreScopeFocusOrigin = useCallback(() => {
    const origin = scopeShortcutOriginRef.current;
    scopeShortcutOriginRef.current = null;
    origin?.focus();
  }, []);

  // `v` — the keyboard route to the Scope zone/sheet (#146). Wide, it
  // unfolds the zone for good and focuses the current row; narrow, it opens
  // the sheet with focus on the current row. Pressing it again while already
  // open just re-homes focus — harmless, not a toggle.
  const handleScopeShortcut = useCallback(() => {
    if (vaults.length <= 1) {
      return;
    }
    const activeElement =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    // A repeat `v` press while focus already sits on a scope row (wide) or
    // inside the sheet (narrow) must stay harmless — it must not clobber the
    // origin with the row itself, or `Escape` would "restore" focus to where
    // it already is instead of back to wherever `v` was first pressed.
    const alreadyInScopeUi =
      activeElement?.closest(".scope-zone, .scope-sheet") != null;
    if (isMobile) {
      setScopeSheetOpen((prev) => {
        if (!prev && !alreadyInScopeUi) {
          scopeShortcutOriginRef.current = activeElement;
        }
        return true;
      });
    } else {
      if (!alreadyInScopeUi) {
        scopeShortcutOriginRef.current = activeElement;
      }
      handleScopeZoneCollapsedChange(false);
    }
    setScopeFocusRequestId((id) => id + 1);
  }, [vaults.length, isMobile, handleScopeZoneCollapsedChange]);

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
        return;
      }

      if (
        event.key.toLowerCase() === "v" &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.altKey &&
        !isEditableTarget(event.target) &&
        !noteActionDialog &&
        !searchOpen &&
        !actionsMenuOpen
      ) {
        event.preventDefault();
        handleScopeShortcut();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    openCreateDialog,
    setSearchOpen,
    writeEnabled,
    handleScopeShortcut,
    noteActionDialog,
    searchOpen,
    actionsMenuOpen,
  ]);

  // The shell's polite scope live region (#146): announces the scope name
  // the instant a pick lands, then its count-or-condition in the same
  // breath if already known, or as a short second sentence once it resolves
  // — never a value that is not yet known.
  const scopeSlotDescription = vaultProjection.describeScope(scope);

  useEffect(() => {
    // Discovery still in flight means `vaults` is a temporary `[]`, not a
    // real "zero Vaults" answer — announcing off it would read a value the
    // shell does not actually have yet. Wait it out.
    if (vaultsLoading || announcedScopeRef.current === scope) {
      return;
    }
    // The baseline scope discovery lands with, on mount, is not a pick —
    // record it silently so the first genuine scope change is the first
    // announcement.
    const isInitialBaseline = announcedScopeRef.current === null;
    announcedScopeRef.current = scope;
    if (isInitialBaseline) {
      announcedScopeSlotRef.current = Boolean(scopeSlotDescription);
      return;
    }
    const name = scopeName(scope, vaults);
    if (scopeSlotDescription) {
      announcedScopeSlotRef.current = true;
      setScopeLiveMessage(`${name}. ${scopeSlotDescription}`);
    } else {
      announcedScopeSlotRef.current = false;
      setScopeLiveMessage(name);
    }
    // Only a genuine scope change (or discovery finally landing) should
    // restart the announcement; `vaults` and `scopeSlotDescription` are read
    // for their value at that moment, not watched for their own updates
    // (the effect below owns that).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, vaultsLoading]);

  useEffect(() => {
    if (
      vaultsLoading ||
      announcedScopeSlotRef.current ||
      !scopeSlotDescription
    ) {
      return;
    }
    announcedScopeSlotRef.current = true;
    setScopeLiveMessage(scopeSlotDescription);
  }, [scopeSlotDescription, vaultsLoading]);

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

  // The scope-change motion policy (#147): the explorer returns to the top
  // the instant the browsing scope changes — never on mount (the restore
  // effect above owns that) and never on the accordion's own unfold, which
  // deliberately never touches scope.
  const previousScopeRef = useRef(scope);
  useEffect(() => {
    if (previousScopeRef.current === scope) {
      return;
    }
    previousScopeRef.current = scope;
    const container = explorerScrollRef.current;
    if (container) {
      container.scrollTop = 0;
    }
  }, [scope]);

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
        helpOpen={help.isOpen}
        onToggleHelp={() => (help.isOpen ? help.closeHelp() : help.openHelp())}
        onScopeChange={handleScopeChange}
        viewingVaultId={activeNote?.vaultId}
        vaultNoteCounts={vaultNoteCounts}
        scopeSheetOpen={scopeSheetOpen}
        onToggleScopeSheet={() =>
          setScopeSheetOpen((prev) => {
            const next = !prev;
            if (next) {
              scopeShortcutOriginRef.current =
                document.activeElement instanceof HTMLElement
                  ? document.activeElement
                  : null;
            }
            return next;
          })
        }
        onCloseScopeSheet={() => setScopeSheetOpen(false)}
        scopeFocusRequestId={scopeFocusRequestId}
        onRestoreScopeFocus={restoreScopeFocusOrigin}
        demoMode={demoMode}
      />

      <div className="visually-hidden" aria-live="polite" aria-atomic="true">
        {scopeLiveMessage}
      </div>

      {/* The opt-in update check's banner (#425). Signed in only: it reads
          the settings response, which demo mode does not serve. */}
      {settingsEnabled ? <UpdateBanner /> : null}

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
          modifiedNotesError={modifiedNotesError}
          onRetryModifiedNotes={() => {
            void loadModifiedNotes();
          }}
          loadingTree={loadingTree}
          treeError={treeError}
          treePartial={treePartial}
          treeMissingVaults={treeMissingVaults}
          tree={tree}
          vaultTrees={vaultTrees}
          expandedFolders={expandedFolders}
          recentCollapsed={recentCollapsed}
          onRecentCollapsedChange={(next) => {
            setRecentCollapsed(next);
            safeSetItem(RECENT_NOTES_COLLAPSED_KEY, next ? "1" : "0");
          }}
          vaults={vaults}
          scope={scope}
          onScopeChange={handleScopeChange}
          viewingVaultId={activeNote?.vaultId}
          vaultNoteCounts={vaultNoteCounts}
          scopeZoneCollapsed={scopeZoneCollapsed}
          onScopeZoneCollapsedChange={handleScopeZoneCollapsedChange}
          scopeFocusRequestId={scopeFocusRequestId}
          onRestoreScopeFocus={restoreScopeFocusOrigin}
          startupProgress={deriveStartupProgress(startupStatus)}
          onExpandedFoldersChange={setExpandedFolders}
          onCloseDrawer={() => setDrawerOpen(false)}
          onRefreshTree={() => {
            void loadTree();
            void loadModifiedNotes();
          }}
          onScrollTopChange={(current) => {
            safeSetItem(EXPLORER_SCROLL_TOP_KEY, String(current));
          }}
          demoMode={demoMode}
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
            <Route
              path="/"
              element={
                vaultsLoading ||
                collectionReadState ===
                  "loading" ? null : collectionReadState === "error" ? (
                  // Discovery failed and nothing is known about the
                  // collection: never the zero-Vault state, which would tell
                  // the reader their Vaults are gone (#333).
                  <BrokenStartState
                    title="Vaults Unavailable"
                    message={collectionError ?? "Could not load your Vaults."}
                    onTryAgain={() => void loadVaults()}
                    manual={CONTEXTUAL_HELP.vaultsUnavailable}
                  />
                ) : registryRecovery ? (
                  <BrokenStartState
                    message={registryRecovery.message}
                    onTryAgain={() => void loadVaults()}
                  />
                ) : showFirstRun ? (
                  <FirstRunChecklist
                    vaults={vaults}
                    onVaultCreated={() => void loadVaults()}
                    onAddGitVault={() =>
                      navigate("/settings", {
                        state: { openVaultCreation: true },
                      })
                    }
                    onOpenSearch={() => setSearchOpen(true)}
                  />
                ) : vaults.length === 0 ? (
                  <ZeroVaultState
                    demoMode={demoMode}
                    onAddVault={() =>
                      navigate("/settings", {
                        state: { openVaultCreation: true },
                      })
                    }
                  />
                ) : (
                  <EmptyState />
                )
              }
            />
            <Route
              path="/stats"
              element={
                hasRegistryRecovery ? (
                  <Navigate to="/" replace />
                ) : (
                  <StatsPage />
                )
              }
            />
            <Route
              path="/graph"
              element={
                hasRegistryRecovery ? (
                  <Navigate to="/" replace />
                ) : (
                  <GraphPage />
                )
              }
            />
            <Route
              path="/settings"
              element={
                // `demoMode` defaults to `false` until Vault discovery's
                // fetch resolves — same transient gap the "/" route below
                // already guards against zero-Vault vs. broken-registry.
                // Rendering `null` through that gap keeps a demo visitor who
                // opens this route directly (bookmark, shared link, reload)
                // from ever seeing SettingsPage begin its own mount, even
                // for one frame (#152).
                vaultsLoading ? null : demoMode || hasRegistryRecovery ? (
                  // Settings is an operator surface, absent rather than
                  // disabled in demo mode (#152) — same posture the backend
                  // already takes by not registering these routes at all.
                  // Silent, like every other withheld operator affordance:
                  // no explanation, just back to the ordinary workspace.
                  <Navigate to="/" replace />
                ) : (
                  <SettingsPage
                    vaults={vaults}
                    onRestoreCreateDraft={(
                      targetVaultId,
                      folder,
                      name,
                      content,
                    ) =>
                      restoreCreateDraft(
                        targetVaultId,
                        folder ? `${folder}/${name}` : name,
                        content,
                      )
                    }
                  />
                )
              }
            />
            <Route
              path="/v/:vaultId/n/:slug"
              element={
                hasRegistryRecovery ? (
                  <Navigate to="/" replace />
                ) : (
                  <NotePage
                    onActiveNoteChange={setActiveNote}
                    onTagSelect={openSearchForTag}
                    propertiesCollapsedStorageKey={
                      NOTE_PROPERTIES_COLLAPSED_KEY
                    }
                    vaultRevision={vaultRevision}
                    writeEnabled={writeEnabled}
                    editRequestId={editRequestId}
                    onWriteNotice={setWriteNotice}
                    onDemoRefusal={handleDemoRefusal}
                    demoMode={demoMode}
                    noteCandidates={noteCandidates}
                    vaults={vaults}
                  />
                )
              }
            />
            {/* Any other address — a stale bookmark, a pre-#137 `/n/:slug`
                link, a typo — says so rather than leaving the pane empty
                (#339). */}
            <Route
              path="*"
              element={<NotFoundState onGoHome={() => navigate("/")} />}
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
          startupStatus={startupStatus}
          onRetryModelSetup={onRetryModelSetup}
          demoMode={demoMode}
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
          vaults={dialogVaults}
          folderPathsByVault={folderPathsByVault}
          initialVaultId={noteActionInitialVaultId}
          initialFolder={noteActionInitialFolder}
          onClose={closeNoteActionDialog}
          onCreate={(targetVaultId, relativePath) =>
            void handleCreateNote(targetVaultId, relativePath)
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

/** The workspace mounted on its own, without the startup gate above it. The
 * gate needs the same collection state to decide whether a broken registry
 * should stop it locking the workspace (#150); both read it from the one
 * collection client (#198), so neither has to hand it to the other. */
export function VaultApp({
  startupStatus,
  onRetryModelSetup,
}: {
  startupStatus: StartupStatus | null;
  onRetryModelSetup: () => void;
}) {
  return (
    <VaultWorkspace
      startupStatus={startupStatus}
      onRetryModelSetup={onRetryModelSetup}
    />
  );
}

/** The Scope zone's own reading of the shrunk startup gate's progress
 * (#150): `undefined` outside `downloading`/`scanning`/`indexing`, since every
 * other state already renders the ordinary aggregate slot. `downloading` only
 * reaches here once the gate has stepped aside, which is a model re-download
 * (a retry after a failed setup): without this slot it ran invisibly (#339). */
function deriveStartupProgress(
  status: StartupStatus | null,
): StartupProgress | undefined {
  if (status?.state === "downloading") {
    const percent = status.percent ?? null;
    return {
      label:
        percent === null
          ? "Downloading search model"
          : `Downloading search model ${percent}%`,
      percent,
      eta: null,
    };
  }
  if (status?.state === "scanning") {
    return { label: "Scanning", percent: null, eta: null };
  }
  if (status?.state === "indexing") {
    const eta = formatEtaSeconds(status.eta_seconds);
    return {
      label: eta
        ? `Indexing ${status.percent}%, ${eta}`
        : `Indexing ${status.percent}%`,
      percent: status.percent,
      eta,
    };
  }
  return undefined;
}

/** The index ETA in the coarsest unit that still says something true: a
 * second-by-second countdown on an estimate this noisy would read as more
 * precision than the number has. `null` when the backend has none yet. */
function formatEtaSeconds(seconds: number | undefined): string | null {
  if (seconds === undefined || seconds <= 0) {
    return null;
  }
  if (seconds < 60) {
    return "under a minute left";
  }
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m left`;
  }
  return `${Math.round(minutes / 60)}h left`;
}

export function App() {
  // Bumped to remount the whole app in place. Unlock does that instead of a
  // page reload when the browser refused to store the token (#339): a reload
  // would forget it, and the remount drops every cached 401 all the same,
  // since the collection store starts over once its last subscriber leaves.
  const [session, setSession] = useState(0);
  return (
    <AppSession
      key={session}
      onUnlockInPlace={() => setSession((current) => current + 1)}
    />
  );
}

function AppSession({ onUnlockInPlace }: { onUnlockInPlace: () => void }) {
  const navigate = useNavigate();
  const [authRequired, setAuthRequired] = useState(false);
  const collection = useVaultCollection();
  const hasRegistryRecovery = Boolean(collection.recovery);
  // Only a discovery that answered can say there are no Vaults; a failed one
  // (`readState` "error") knows nothing either way.
  const hasNoVaults =
    !collection.loading &&
    collection.readState === "empty" &&
    !hasRegistryRecovery;
  // The startup route is neither useful nor permitted to poll while the
  // workspace is a zero-Vault or broken-registry recovery surface (#150).
  // Resolve the collection first so either condition can win before a model
  // step ever has a chance to gate the page.
  const startup = useStartupStatus(
    !collection.loading && !hasRegistryRecovery && !hasNoVaults,
  );

  useEffect(() => {
    onUnauthorized(() => setAuthRequired(true));
    return () => onUnauthorized(null);
  }, []);

  return (
    <HelpProvider
      demoMode={collection.demoMode}
      signedOut={authRequired}
      onOpenSetupChecklist={
        authRequired || collection.loading || collection.demoMode
          ? undefined
          : () => {
              reopenFirstRun();
              navigate("/");
            }
      }
    >
      {authRequired ? (
        <TokenPrompt
          onSubmit={(token) => {
            setAuthRequired(false);
            if (setToken(token)) {
              window.location.reload();
            } else {
              onUnlockInPlace();
            }
          }}
        />
      ) : null}
      <StartupGate
        status={startup.status}
        connectionIssue={startup.connectionIssue}
        hasSteppedPastGate={startup.hasSteppedPastGate}
        discoveryLoading={collection.loading}
        hasRegistryRecovery={hasRegistryRecovery}
        hasNoVaults={hasNoVaults}
        demoMode={collection.demoMode}
        onAcceptGemma={() => void startup.acceptGemma()}
        onDeclineGemma={() => void startup.declineGemma()}
      >
        <VaultWorkspace
          startupStatus={startup.status}
          onRetryModelSetup={() => void startup.retryModelSetup()}
        />
        {/* After an upgrade (#418), over the workspace only: the gate holds
            its children until discovery has said whether this is a demo,
            which never shows it (the server refuses it there too). */}
        {!authRequired && !collection.demoMode ? <WhatsNew /> : null}
      </StartupGate>
    </HelpProvider>
  );
}

/** An address that matches no route (#339). The shell around it still works,
 * and the action takes the reader back to the landing route. */
function NotFoundState({ onGoHome }: { onGoHome: () => void }) {
  return (
    <StateBlock
      title="Page Not Found"
      description="Nothing lives at this address. The link may be out of date."
      actionLabel="Go to notes"
      onAction={onGoHome}
    />
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

/** The zero-Vault workspace (#150): a genuine "nothing added yet" instance,
 * indistinguishable whether it is a brand-new install or one just emptied
 * out — driven purely by `vaults.length === 0`, never a first-visit flag.
 * The action opens the same creation flow #148 wires into the Settings
 * index (#153) — this route has no room for the flow itself, so it
 * navigates there and asks it to open immediately. Absent rather than
 * disabled in demo mode, matching every other operator affordance. */
function ZeroVaultState({
  demoMode,
  onAddVault,
}: {
  demoMode: boolean;
  onAddVault: () => void;
}) {
  return (
    <StateBlock
      title="No Vaults Yet"
      description={
        demoMode
          ? "This demo has no Vaults loaded."
          : "Add a Vault to start browsing and searching your notes."
      }
      actionLabel={demoMode ? undefined : "Add a Vault"}
      onAction={demoMode ? undefined : onAddVault}
      help={
        demoMode ? undefined : (
          <ContextualHelpLink to={CONTEXTUAL_HELP.noVaults} />
        )
      }
    />
  );
}

/** A broken start (#150): the registry file itself is unreadable, or
 * discovery failed. Both open the ordinary workspace with this same
 * documented error block rather than a full-screen gate. */
function BrokenStartState({
  title = "Vault Registry Unavailable",
  message,
  onTryAgain,
  manual = CONTEXTUAL_HELP.registryRecovery,
}: {
  title?: string;
  message: string;
  onTryAgain?: () => void;
  /** The manual page that explains this start (#423). */
  manual?: HelpLocation;
}) {
  return (
    <StateBlock
      tone="error"
      title={title}
      description={`${message} Nothing was changed, and your Markdown is untouched.`}
      actionLabel={onTryAgain ? "Try again" : undefined}
      onAction={onTryAgain}
      help={<ContextualHelpLink to={manual} />}
    />
  );
}

export default App;

import {
  Fragment,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type RefObject,
} from "react";
import { NavLink } from "react-router-dom";

import { ChangesPanel } from "../components/ChangesPanel";
import {
  FolderTree,
  RecentNotesList,
  SideHead,
  type ExpandedFoldersUpdate,
} from "../components/Explorer";
import {
  BarChartIcon,
  Graph3Icon,
  HelpIcon,
  SettingsIcon,
} from "../components/icons";
import { ExplorerSkeleton, StateBlock, UiButton } from "../components/ui";
import { CHANGES_COLLAPSED_KEY } from "./constants";
import { safeGetItem, safeSetItem } from "../lib/storage";
import { describeMissingVaults } from "../lib/vaultParticipants";
import { VaultAggregateSlot, VaultSlot } from "./vaultSlot";
import {
  deriveVaultAggregate,
  deriveVaultSlot,
  scopeName,
} from "./vaultSlotLogic";
import {
  expandedFoldersForVault,
  getStoredUnfoldedVault,
  isVaultUnfoldable,
  resolveInitialUnfoldedVault,
  resolveLandingVaultId,
  setStoredUnfoldedVault,
  vaultFolderUpdate,
} from "./vaultAccordion";
import type {
  ExplorerFolder,
  ModifiedNote,
  RecentNote,
  VaultId,
  VaultScope,
  VaultSummary,
  VaultTree,
} from "../types";

/** How long each face of the alternating startup slot (percent, then time
 * left) holds before the other takes its place. Long enough to read, short
 * enough that a reader who glanced at the wrong moment does not wait. */
const STARTUP_SLOT_FACE_MS = 3_000;

/** How long a scope change holds the outgoing scope's content on screen
 * before giving way to the skeleton (#147). `loadingTree` only toggles on a
 * scope change or first mount — the SSE-driven background tree refresh
 * never touches it — so gating the skeleton on this timer rather than on
 * `loadingTree` directly is what keeps a fast answer silent and a slow one
 * announced, without the skeleton ever stacking on top of the tree it is
 * about to replace. */
const SCOPE_CHANGE_SKELETON_DELAY_MS = 200;

function countNotes(folder: ExplorerFolder): number {
  return (
    folder.notes.length +
    folder.folders.reduce((sum, f) => sum + countNotes(f), 0)
  );
}

/** The first-run model-setup/indexing progress the shrunk startup gate no
 * longer blocks on (#150): `percent` is unknown during `scanning`, known
 * during `indexing`. */
export type StartupProgress = {
  label: string;
  percent: number | null;
  /** Time left for this index, already worded ("2m left"); `null` until the
   * backend has enough throughput to estimate one. */
  eta: string | null;
};

/**
 * The startup slot's reading is the moving part: the number itself carries
 * the shimmer, so what the eye tracks is the value that is changing rather
 * than a separate bar beside it. Where a time estimate exists, the slot
 * alternates between the percent and it — one slot, two readings, never two
 * things competing for the same corner.
 *
 * A Vault still scanning has neither, so the slot falls back to the
 * per-Vault indexing bar: something is moving and no number describes it yet.
 */
function StartupProgressSlot({ progress }: { progress: StartupProgress }) {
  const [showEta, setShowEta] = useState(false);
  const canAlternate = progress.percent !== null && progress.eta !== null;

  useEffect(() => {
    if (!canAlternate) {
      setShowEta(false);
      return;
    }
    const timer = window.setInterval(
      () => setShowEta((prev) => !prev),
      STARTUP_SLOT_FACE_MS,
    );
    return () => window.clearInterval(timer);
  }, [canAlternate]);

  if (progress.percent === null) {
    return (
      <span
        className="vault-slot-indexing"
        role="status"
        aria-label={progress.label}
      >
        <span className="vault-slot-indexing-bar" aria-hidden="true" />
      </span>
    );
  }
  const reading =
    showEta && progress.eta !== null ? progress.eta : `${progress.percent}%`;
  return (
    <span
      className="vault-slot-indexing"
      role="status"
      aria-label={progress.label}
    >
      <span className="side-count slot-shimmer-reading">{reading}</span>
    </span>
  );
}

/**
 * Vault scope, readable and changeable in exactly one place on the desktop
 * (#138): a collapsible zone pinned above the rail, never scrolling with the
 * notes. Absent at one enabled Vault or on mobile — narrowing scope has
 * nothing to offer there (mobile scope chrome is #145's ticket).
 *
 * The row list is a pick-exactly-one radiogroup (#146): one tab stop for the
 * whole group, up/down move between rows, `Enter`/`Space` picks via the
 * button's own native activation. `scopeFocusRequestId` is a bare counter —
 * bumping it (regardless of the new value) asks this zone to focus its
 * currently selected row, the `v` shortcut's job in `App.tsx`.
 */
function ScopeZone({
  vaults,
  scope,
  onScopeChange,
  viewingVaultId,
  collapsed,
  onToggleCollapsed,
  noteCounts,
  scopeFocusRequestId,
  onRestoreScopeFocus,
  startupProgress,
  demoMode = false,
}: {
  vaults: VaultSummary[];
  scope: VaultScope;
  onScopeChange: (next: VaultScope) => void;
  viewingVaultId: VaultId | undefined;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  noteCounts: Record<VaultId, number | undefined>;
  scopeFocusRequestId: number;
  onRestoreScopeFocus: () => void;
  /** The first-run model-setup/indexing progress the startup gate no longer
   * blocks on (#150), surfaced here in the zone's own slot instead. */
  startupProgress?: StartupProgress;
  /** Clamps every condition slot to the amber tier (#152). */
  demoMode?: boolean;
}) {
  const rowRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const rowIds: VaultScope[] = [
    "all",
    ...vaults.map((vault) => vault.vault_id),
  ];
  const selectedIndex = Math.max(0, rowIds.indexOf(scope));

  useEffect(() => {
    if (scopeFocusRequestId === 0 || collapsed) {
      return;
    }
    rowRefs.current[selectedIndex]?.focus();
    // Only the counter bumping matters; re-running on every selection change
    // would steal focus back whenever `scope` itself changes elsewhere.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scopeFocusRequestId, collapsed]);

  // Absent at exactly one enabled Vault only when there is no startup work to
  // report — narrowing scope has nothing to offer there. During first-run
  // scanning/indexing it stays visible solely for the documented progress
  // slot. Present at zero Vaults too (#150): the zone holds its place reading
  // "All Vaults" with no rows beneath it, in neutral ink.
  if (vaults.length === 1 && !startupProgress) {
    return null;
  }

  const focusRow = (index: number) => {
    const wrapped = (index + rowIds.length) % rowIds.length;
    rowRefs.current[wrapped]?.focus();
  };

  const onRowKeyDown = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      focusRow(index + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      focusRow(index - 1);
    } else if (event.key === "Escape") {
      event.preventDefault();
      onRestoreScopeFocus();
    }
  };

  const viewingVault = vaults.find(
    (vault) => vault.vault_id === viewingVaultId,
  );
  // The collapsed head names the scope in the worst ink present across every
  // enabled Vault, not just the selected one, so narrowing scope never hides
  // trouble elsewhere (#116, amended by #117).
  const aggregate = deriveVaultAggregate(vaults, noteCounts, demoMode);
  const worstTierClass =
    aggregate.kind === "shortfall" ? ` vault-tier-${aggregate.tier}` : "";

  return (
    <div className="scope-zone">
      <button
        type="button"
        className="side-head scope-zone-head"
        data-open={!collapsed}
        aria-expanded={!collapsed}
        aria-controls="scope-zone-list"
        onClick={onToggleCollapsed}
      >
        <span className="side-caret" aria-hidden="true" />
        <span className="side-label">Scope</span>
        <span className="scope-zone-keycap" aria-hidden="true">
          V
        </span>
        <span className="side-rule" />
        {collapsed ? (
          <>
            <span className={`scope-zone-current${worstTierClass}`}>
              {scopeName(scope, vaults)}
            </span>
            {startupProgress ? (
              <StartupProgressSlot progress={startupProgress} />
            ) : (
              <VaultAggregateSlot
                vaults={vaults}
                counts={noteCounts}
                demoMode={demoMode}
                compact
              />
            )}
          </>
        ) : (
          <span className="side-count">
            {String(vaults.length).padStart(2, "0")}
          </span>
        )}
      </button>

      {collapsed && viewingVault ? (
        <p className="scope-zone-viewing-line">viewing {viewingVault.name}</p>
      ) : null}

      {collapsed ? null : (
        <ul
          id="scope-zone-list"
          className="scope-zone-list"
          role="radiogroup"
          aria-label="Vault scope"
        >
          <li>
            <button
              type="button"
              role="radio"
              aria-checked={scope === "all"}
              tabIndex={selectedIndex === 0 ? 0 : -1}
              ref={(el) => {
                rowRefs.current[0] = el;
              }}
              className={`scope-row${scope === "all" ? " is-selected" : ""}`}
              onClick={() => onScopeChange("all")}
              onKeyDown={(event) => onRowKeyDown(event, 0)}
            >
              <span className="scope-row-label">All Vaults</span>
              {startupProgress ? (
                <StartupProgressSlot progress={startupProgress} />
              ) : (
                <VaultAggregateSlot
                  vaults={vaults}
                  counts={noteCounts}
                  demoMode={demoMode}
                />
              )}
            </button>
          </li>
          {vaults.map((vault, index) => (
            <li key={vault.vault_id}>
              <button
                type="button"
                role="radio"
                aria-checked={scope === vault.vault_id}
                tabIndex={selectedIndex === index + 1 ? 0 : -1}
                ref={(el) => {
                  rowRefs.current[index + 1] = el;
                }}
                className={`scope-row${scope === vault.vault_id ? " is-selected" : ""}`}
                onClick={() => onScopeChange(vault.vault_id)}
                onKeyDown={(event) => onRowKeyDown(event, index + 1)}
              >
                <span className="scope-row-label">{vault.name}</span>
                {viewingVaultId === vault.vault_id ? (
                  <span className="scope-row-viewing">viewing</span>
                ) : null}
                <VaultSlot
                  vault={vault}
                  noteCount={noteCounts[vault.vault_id]}
                  demoMode={demoMode}
                />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * The explorer tree under `all` with more than one enabled Vault (#142): an
 * accordion of per-Vault sections, exactly one unfolded at a time. Every
 * Vault keeps a permanent one-line head in Vault-management order; only the
 * unfolded Vault's own tree (from `vaultTrees`, never the merged `tree`) is
 * shown, so height stays one tree plus N one-line heads whatever N is.
 * Unfolding only calls `onUnfold` — it never touches scope.
 */
function VaultAccordion({
  vaults,
  vaultTrees,
  unfoldedVaultId,
  onUnfold,
  noteCounts,
  currentPath,
  expandedFolders,
  onExpandedFoldersChange,
  writeEnabled,
  onCreateNoteInFolder,
  treeAnswered,
  demoMode = false,
}: {
  vaults: VaultSummary[];
  vaultTrees: VaultTree[];
  unfoldedVaultId: VaultId | undefined;
  onUnfold: (vaultId: VaultId) => void;
  noteCounts: Record<VaultId, number | undefined>;
  currentPath: string;
  expandedFolders: Record<string, boolean>;
  onExpandedFoldersChange: (update: ExpandedFoldersUpdate) => void;
  writeEnabled: boolean;
  onCreateNoteInFolder: (folderPath: string, vaultId: VaultId) => void;
  /** Whether a tree read has answered for this scope. Before it has, a
   * Vault with no tree is simply not loaded yet; after, it did not answer. */
  treeAnswered: boolean;
  demoMode?: boolean;
}) {
  const treesByVault = new Map(
    vaultTrees.map((entry) => [entry.vault_id, entry.tree]),
  );

  return (
    <>
      {vaults.map((vault) => {
        const isOpen = vault.vault_id === unfoldedVaultId;
        const unfoldable = isVaultUnfoldable(vault);
        const vaultTree = treesByVault.get(vault.vault_id);

        return (
          <Fragment key={vault.vault_id}>
            <SideHead
              label={vault.name}
              slot={
                <VaultSlot
                  vault={vault}
                  noteCount={noteCounts[vault.vault_id]}
                  demoMode={demoMode}
                />
              }
              collapsible
              open={isOpen}
              disabled={!unfoldable}
              className="vault-accordion-head"
              onToggle={() => onUnfold(vault.vault_id)}
            />
            {isOpen && vaultTree ? (
              <FolderTree
                root={vaultTree}
                currentPath={currentPath}
                expandedFolders={expandedFoldersForVault(
                  expandedFolders,
                  vault.vault_id,
                )}
                onExpandedFoldersChange={(update) =>
                  onExpandedFoldersChange(
                    vaultFolderUpdate(vault.vault_id, update),
                  )
                }
                writeEnabled={writeEnabled}
                // The tree knows folders, not which Vault they belong to. The
                // Vault is bound here, where the accordion still knows whose
                // tree this is, so a new note lands in the Vault it was
                // started from rather than in whichever one was inferred
                // elsewhere.
                onCreateNoteInFolder={(folderPath) =>
                  onCreateNoteInFolder(folderPath, vault.vault_id)
                }
              />
            ) : isOpen && treeAnswered ? (
              // The read left this Vault out: it did not answer. An empty
              // section here read as "this Vault has no notes" (#334).
              <p className="explorer-tree-partial">
                {describeMissingVaults([vault.name])}
              </p>
            ) : null}
          </Fragment>
        );
      })}
    </>
  );
}

/**
 * Whole-vault destinations. Lives inside the sidebar rather than the topbar
 * on purpose: the topbar's four mobile slots are the hard constraint, and the
 * rail sits inside the drawer, outside that budget. It sits in the footer
 * beside New note (#530), where a row of destinations belongs and where a
 * thumb reaches it; it holds destinations only, so nothing it opens appears
 * out of sight at the top of the list. On the phone Help is its last item,
 * since the phone's top bar has no slot for it (#417).
 */
function ExplorerRail({
  settingsEnabled,
  onToggleHelp,
}: {
  settingsEnabled?: boolean;
  onToggleHelp?: () => void;
}) {
  return (
    <div className="explorer-rail">
      <NavLink
        className={({ isActive }) =>
          `explorer-rail-item${isActive ? " active" : ""}`
        }
        to="/stats"
        aria-label="Stats"
        title="Stats"
      >
        <BarChartIcon />
      </NavLink>
      <NavLink
        className={({ isActive }) =>
          `explorer-rail-item${isActive ? " active" : ""}`
        }
        to="/graph"
        aria-label="Graph"
        title="Graph"
      >
        <Graph3Icon />
      </NavLink>
      {settingsEnabled ? (
        <NavLink
          className={({ isActive }) =>
            `explorer-rail-item explorer-rail-settings${isActive ? " active" : ""}`
          }
          to="/settings"
          aria-label="Settings"
          title="Settings"
        >
          <SettingsIcon />
        </NavLink>
      ) : null}
      {onToggleHelp ? (
        <button
          type="button"
          className="explorer-rail-item"
          aria-label="Help"
          title="Help"
          onClick={onToggleHelp}
        >
          <HelpIcon />
        </button>
      ) : null}
    </div>
  );
}

type ExplorerPaneProps = {
  explorerScrollRef: RefObject<HTMLElement | null>;
  drawerOpen: boolean;
  isMobile: boolean;
  writeEnabled: boolean;
  settingsEnabled?: boolean;
  onCreateNoteInFolder: (folderPath: string, vaultId?: VaultId) => void;
  locationPathname: string;
  recentNotes: RecentNote[];
  modifiedNotes: ModifiedNote[];
  modifiedNotesPartial: boolean;
  modifiedNotesMissingVaults: string[];
  modifiedNotesError: string | null;
  onRetryModifiedNotes: () => void;
  loadingTree: boolean;
  treeError: string | null;
  /** The tree read's own partiality (#334): Vaults it asked that did not
   * answer fresh, named in a trailing line like Changed on disk's. */
  treePartial: boolean;
  treeMissingVaults: string[];
  tree: ExplorerFolder | null;
  vaultTrees: VaultTree[];
  expandedFolders: Record<string, boolean>;
  recentCollapsed: boolean;
  onRecentCollapsedChange: (next: boolean) => void;
  onExpandedFoldersChange: (update: ExpandedFoldersUpdate) => void;
  onCloseDrawer: () => void;
  onRefreshTree: () => void;
  onScrollTopChange: (top: number) => void;
  vaults: VaultSummary[];
  scope: VaultScope;
  onScopeChange: (next: VaultScope) => void;
  viewingVaultId: VaultId | undefined;
  scopeZoneCollapsed: boolean;
  onScopeZoneCollapsedChange: (next: boolean) => void;
  vaultNoteCounts: Record<VaultId, number | undefined>;
  scopeFocusRequestId: number;
  onRestoreScopeFocus: () => void;
  startupProgress?: StartupProgress;
  /** Clamps every condition slot to the amber tier (#152). */
  demoMode?: boolean;
  /** Given below 920px only (#530): the phone's top bar has no Help slot, so
   * the rail carries it as its last item. */
  onToggleHelp?: () => void;
};

export function ExplorerPane({
  explorerScrollRef,
  drawerOpen,
  isMobile,
  writeEnabled,
  settingsEnabled,
  onCreateNoteInFolder,
  locationPathname,
  recentNotes,
  modifiedNotes,
  modifiedNotesPartial,
  modifiedNotesMissingVaults,
  modifiedNotesError,
  onRetryModifiedNotes,
  loadingTree,
  treeError,
  treePartial,
  treeMissingVaults,
  tree,
  vaultTrees,
  expandedFolders,
  recentCollapsed,
  onRecentCollapsedChange,
  onExpandedFoldersChange,
  onCloseDrawer,
  onRefreshTree,
  onScrollTopChange,
  vaults,
  scope,
  onScopeChange,
  viewingVaultId,
  scopeZoneCollapsed,
  onScopeZoneCollapsedChange,
  vaultNoteCounts,
  scopeFocusRequestId,
  onRestoreScopeFocus,
  startupProgress,
  demoMode = false,
  onToggleHelp,
}: ExplorerPaneProps) {
  // Local, not lifted: the shell already carries a large prop surface, and the
  // module map is explicit that this is a coordination seam rather than an
  // invitation to move feature state into it. Changed on disk starts folded
  // and remembers its fold the way Recently viewed does (#530).
  const [changesCollapsed, setChangesCollapsed] = useState<boolean>(
    () => safeGetItem(CHANGES_COLLAPSED_KEY) !== "0",
  );
  const toggleChangesCollapsed = () => {
    setChangesCollapsed((prev) => {
      safeSetItem(CHANGES_COLLAPSED_KEY, prev ? "0" : "1");
      return !prev;
    });
  };

  // The accordion's unfolded Vault (#142). Narrowing scope always sets it to
  // the Vault just left, so widening restores that Vault — resolved eagerly
  // off `scope` alone, no need to wait for anything else. The landing
  // default (note's own Vault, else the last persisted, else nothing) is
  // resolved once, off the URL and storage directly rather than `viewingVaultId`
  // (which depends on the open note's own content fetch and would race
  // App.tsx's last-note redirect).
  const [unfoldedVaultId, setUnfoldedVaultId] = useState<VaultId | undefined>(
    undefined,
  );
  const initializedUnfoldRef = useRef(false);

  useEffect(() => {
    if (scope !== "all") {
      initializedUnfoldRef.current = true;
      setUnfoldedVaultId(scope);
      setStoredUnfoldedVault(scope);
      return;
    }
    if (initializedUnfoldRef.current || vaults.length === 0) {
      return;
    }
    initializedUnfoldRef.current = true;
    const landingVaultId = resolveLandingVaultId(locationPathname);
    const initial = resolveInitialUnfoldedVault(
      landingVaultId,
      getStoredUnfoldedVault(),
      vaults,
    );
    setUnfoldedVaultId(initial);
    if (initial) {
      setStoredUnfoldedVault(initial);
    }
  }, [scope, vaults, locationPathname]);

  const handleUnfoldVault = (vaultId: VaultId) => {
    // Clicking the unfolded Vault folds it, leaving nothing unfolded — the
    // same plain list of names §29 already documents for an instance with no
    // history. A head that ignores every click after the first reads as
    // broken, and there is no other way back to that state by hand.
    if (vaultId === unfoldedVaultId) {
      setUnfoldedVaultId(undefined);
      setStoredUnfoldedVault(null);
      return;
    }
    setUnfoldedVaultId(vaultId);
    setStoredUnfoldedVault(vaultId);
  };

  // The scope-change motion policy (#147): the outgoing tree stays on screen
  // untouched until the narrowed answer lands, and only gives way to the
  // skeleton once loading has run longer than the hold. A fast answer never
  // shows the skeleton at all. A cold mount has no prior content to hold, so
  // it keeps the pre-#147 behavior of showing the skeleton immediately —
  // `tree` is read only to snapshot that at the instant loading starts, not
  // to react to the fetch later replacing it.
  const [showTreeSkeleton, setShowTreeSkeleton] = useState(false);
  useEffect(() => {
    if (!loadingTree) {
      setShowTreeSkeleton(false);
      return;
    }
    if (tree === null) {
      setShowTreeSkeleton(true);
      return;
    }
    const id = window.setTimeout(
      () => setShowTreeSkeleton(true),
      SCOPE_CHANGE_SKELETON_DELAY_MS,
    );
    return () => window.clearTimeout(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadingTree]);

  // The content region's own scope-derived branch (accordion vs. flat tree,
  // which Vault's slot) holds at the outgoing scope until `tree`/`vaultTrees`
  // actually land for the new one — never the live `scope` mid-flight, or a
  // scope change would pair the new Vault's header with the old Vault's (or
  // old accordion's) still-loaded content. The chrome above (Scope zone /
  // topbar) is unaffected — it always reflects live `scope`.
  //
  // Synced off `tree`/`vaultTrees` themselves, not off `loadingTree` — a
  // `loadingTree`-keyed sync raced `useVaultTree`'s own scope-triggered
  // fetch: React fires a child's effects before its parent's in the same
  // commit, so this component's effect could observe the new `scope` prop
  // with `loadingTree` still momentarily false, one tick before the parent's
  // effect sets it true. `vaultTrees` never has that gap: `useVaultTree`
  // only ever replaces it (a fresh array, no equality bail-out) once a fetch
  // for the current `scope` has actually resolved, so watching it directly
  // is the one signal that can't fire early.
  const [committedScope, setCommittedScope] = useState(scope);
  useEffect(() => {
    setCommittedScope(scope);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tree, vaultTrees]);

  const showAccordion = committedScope === "all" && vaults.length > 1;
  const narrowedVault =
    committedScope !== "all"
      ? vaults.find((vault) => vault.vault_id === committedScope)
      : undefined;
  // The Vault whose condition the flat tree's head reports. At exactly one
  // enabled Vault there is no Scope zone, no accordion and no mobile scope
  // row, so without this the one Vault's conflict, stopped sync or failed
  // index showed nowhere in the workspace (#334). Narrowing scope has
  // nothing to offer there; reporting the Vault's health still does.
  const headVault =
    narrowedVault ?? (vaults.length === 1 ? vaults[0] : undefined);
  // At one enabled Vault that head is the only desktop place the Vault's
  // condition shows, so while it carries one it is pinned to the scrolling
  // nav's top or bottom edge rather than scrolling out of view under a long
  // tree or below Changed on disk and Recently viewed (#334).
  const pinHead =
    vaults.length === 1 &&
    headVault !== undefined &&
    deriveVaultSlot(headVault, vaultNoteCounts[headVault.vault_id], demoMode)
      .kind === "condition";
  // A settled read that produced no tree and no error: every Vault it asked
  // left itself out. Without this the pane was blank, with nothing to click.
  const treeAnswered = !loadingTree && !treeError;
  const treeEmptyUnanswered =
    !showAccordion && treeAnswered && tree === null && vaults.length > 0;

  return (
    <aside className="explorer-pane" data-open={drawerOpen}>
      {isMobile ? null : (
        <ScopeZone
          vaults={vaults}
          scope={scope}
          onScopeChange={onScopeChange}
          viewingVaultId={viewingVaultId}
          collapsed={scopeZoneCollapsed}
          onToggleCollapsed={() =>
            onScopeZoneCollapsedChange(!scopeZoneCollapsed)
          }
          noteCounts={vaultNoteCounts}
          scopeFocusRequestId={scopeFocusRequestId}
          onRestoreScopeFocus={onRestoreScopeFocus}
          startupProgress={startupProgress}
          demoMode={demoMode}
        />
      )}
      {/* Only this middle zone scrolls; the Scope zone and footer stay put. */}
      <div
        className="explorer-nav"
        ref={explorerScrollRef as RefObject<HTMLDivElement | null>}
        onScroll={(event) => {
          onScrollTopChange(event.currentTarget.scrollTop);
        }}
      >
        <ChangesPanel
          notes={modifiedNotes}
          onNavigate={onCloseDrawer}
          vaults={vaults}
          scope={scope}
          partial={modifiedNotesPartial}
          missingVaultNames={modifiedNotesMissingVaults}
          error={modifiedNotesError}
          onRetry={onRetryModifiedNotes}
          collapsed={changesCollapsed}
          onToggleCollapsed={toggleChangesCollapsed}
        />

        <RecentNotesList
          notes={recentNotes}
          onNavigate={onCloseDrawer}
          collapsed={recentCollapsed}
          onToggleCollapsed={() => onRecentCollapsedChange(!recentCollapsed)}
          vaults={vaults}
        />

        {showTreeSkeleton ? <ExplorerSkeleton /> : null}
        {!showTreeSkeleton && !loadingTree && treeError && !tree ? (
          <StateBlock
            title="Explorer Unavailable"
            description={treeError}
            actionLabel="Retry"
            onAction={onRefreshTree}
          />
        ) : null}
        {showTreeSkeleton ? null : showAccordion ? (
          <VaultAccordion
            vaults={vaults}
            vaultTrees={vaultTrees}
            unfoldedVaultId={unfoldedVaultId}
            onUnfold={handleUnfoldVault}
            noteCounts={vaultNoteCounts}
            currentPath={locationPathname}
            expandedFolders={expandedFolders}
            onExpandedFoldersChange={onExpandedFoldersChange}
            writeEnabled={writeEnabled}
            onCreateNoteInFolder={onCreateNoteInFolder}
            treeAnswered={treeAnswered}
            demoMode={demoMode}
          />
        ) : (
          <>
            {tree || (headVault && !loadingTree) ? (
              <SideHead
                label="Notes"
                className={pinHead ? "is-pinned" : undefined}
                count={headVault || !tree ? undefined : countNotes(tree)}
                slot={
                  headVault ? (
                    <VaultSlot
                      vault={headVault}
                      noteCount={vaultNoteCounts[headVault.vault_id]}
                      demoMode={demoMode}
                    />
                  ) : undefined
                }
              />
            ) : null}
            {treeEmptyUnanswered ? (
              <StateBlock
                tone="error"
                title="Nothing Found"
                description={
                  treeMissingVaults.length > 0
                    ? describeMissingVaults(treeMissingVaults)
                    : "The note tree came back empty."
                }
                actionLabel="Retry"
                onAction={onRefreshTree}
              />
            ) : null}
            {tree ? (
              <FolderTree
                root={tree}
                currentPath={locationPathname}
                expandedFolders={expandedFolders}
                onExpandedFoldersChange={onExpandedFoldersChange}
                writeEnabled={writeEnabled}
                onCreateNoteInFolder={(folderPath) =>
                  onCreateNoteInFolder(folderPath, narrowedVault?.vault_id)
                }
              />
            ) : null}
          </>
        )}
        {/* Never a banner: the tree still renders what did answer, and this
            trailing line names only the Vaults that did not (#334). The
            empty case already says it in its error block. */}
        {!showTreeSkeleton &&
        treeAnswered &&
        treePartial &&
        treeMissingVaults.length > 0 &&
        !treeEmptyUnanswered ? (
          <p className="explorer-tree-partial">
            {describeMissingVaults(treeMissingVaults)}
          </p>
        ) : null}
      </div>

      {/* The rail's destinations and the one create action (#530). Nothing
          here opens anything inside the list above, so the footer never
          points at something it cannot see. */}
      <div className="explorer-footer">
        <ExplorerRail
          settingsEnabled={settingsEnabled}
          onToggleHelp={isMobile ? onToggleHelp : undefined}
        />
        {writeEnabled ? (
          <UiButton
            className="close-note explorer-new-note"
            onClick={() => onCreateNoteInFolder("")}
          >
            New note
          </UiButton>
        ) : null}
      </div>
    </aside>
  );
}

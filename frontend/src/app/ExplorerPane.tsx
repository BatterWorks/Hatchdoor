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
import { FolderTree, RecentNotesList, SideHead } from "../components/Explorer";
import {
  BarChartIcon,
  Graph3Icon,
  InboxIcon,
  SettingsIcon,
} from "../components/icons";
import { ExplorerSkeleton, StateBlock, UiButton } from "../components/ui";
import { VaultAggregateSlot, VaultSlot } from "./vaultSlot";
import { deriveVaultAggregate, scopeName } from "./vaultSlotLogic";
import {
  expandedFoldersForVault,
  getStoredUnfoldedVault,
  isVaultUnfoldable,
  resolveInitialUnfoldedVault,
  resolveLandingVaultId,
  setStoredUnfoldedVault,
  withVaultFolderChange,
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
export type StartupProgress = { label: string; percent: number | null };

/** Reuses the per-Vault "indexing" slot's visual language (an animated bar)
 * rather than inventing a second one, adding only the percent when known. */
function StartupProgressSlot({ progress }: { progress: StartupProgress }) {
  return (
    <span
      className="vault-slot-indexing"
      role="status"
      aria-label={progress.label}
    >
      {progress.percent !== null ? (
        <span className="side-count">{progress.percent}%</span>
      ) : null}
      <span className="vault-slot-indexing-bar" aria-hidden="true" />
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
  demoMode = false,
}: {
  vaults: VaultSummary[];
  vaultTrees: VaultTree[];
  unfoldedVaultId: VaultId | undefined;
  onUnfold: (vaultId: VaultId) => void;
  noteCounts: Record<VaultId, number | undefined>;
  currentPath: string;
  expandedFolders: Record<string, boolean>;
  onExpandedFoldersChange: (next: Record<string, boolean>) => void;
  writeEnabled: boolean;
  onCreateNoteInFolder: (folderPath: string, vaultId: VaultId) => void;
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
                onExpandedFoldersChange={(next) =>
                  onExpandedFoldersChange(
                    withVaultFolderChange(
                      expandedFolders,
                      vault.vault_id,
                      next,
                    ),
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
            ) : null}
          </Fragment>
        );
      })}
    </>
  );
}

/**
 * Whole-vault destinations plus the changes panel. Lives inside the sidebar
 * rather than the topbar on purpose: the topbar's four mobile slots are the
 * hard constraint, and the rail sits inside the drawer, outside that budget.
 */
function ExplorerRail({
  changesOpen,
  onToggleChanges,
  settingsEnabled,
}: {
  changesOpen: boolean;
  onToggleChanges: () => void;
  settingsEnabled?: boolean;
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
      <button
        type="button"
        className="explorer-rail-item"
        data-open={changesOpen}
        aria-expanded={changesOpen}
        aria-controls="explorer-changes-panel"
        aria-label="Recently changed notes"
        title="Recently changed notes"
        onClick={onToggleChanges}
      >
        <InboxIcon />
      </button>
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
  loadingTree: boolean;
  treeError: string | null;
  tree: ExplorerFolder | null;
  vaultTrees: VaultTree[];
  expandedFolders: Record<string, boolean>;
  recentCollapsed: boolean;
  onRecentCollapsedChange: (next: boolean) => void;
  onExpandedFoldersChange: (next: Record<string, boolean>) => void;
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
  loadingTree,
  treeError,
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
}: ExplorerPaneProps) {
  // Local, not lifted: the shell already carries a large prop surface, and the
  // module map is explicit that this is a coordination seam rather than an
  // invitation to move feature state into it.
  const [changesOpen, setChangesOpen] = useState(false);

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
      <ExplorerRail
        changesOpen={changesOpen}
        onToggleChanges={() => setChangesOpen((prev) => !prev)}
        settingsEnabled={settingsEnabled}
      />

      {/* Only this middle zone scrolls; rail and footer stay put. */}
      <div
        className="explorer-nav"
        ref={explorerScrollRef as RefObject<HTMLDivElement | null>}
        onScroll={(event) => {
          onScrollTopChange(event.currentTarget.scrollTop);
        }}
      >
        {changesOpen ? (
          <ChangesPanel
            notes={modifiedNotes}
            onNavigate={onCloseDrawer}
            vaults={vaults}
            scope={scope}
            partial={modifiedNotesPartial}
            missingVaultNames={modifiedNotesMissingVaults}
          />
        ) : null}

        <RecentNotesList
          notes={recentNotes}
          onNavigate={onCloseDrawer}
          collapsed={recentCollapsed}
          onToggleCollapsed={() => onRecentCollapsedChange(!recentCollapsed)}
          vaults={vaults}
          scope={scope}
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
            demoMode={demoMode}
          />
        ) : (
          <>
            {tree ? (
              <SideHead
                label="Notes"
                count={narrowedVault ? undefined : countNotes(tree)}
                slot={
                  narrowedVault ? (
                    <VaultSlot
                      vault={narrowedVault}
                      noteCount={vaultNoteCounts[narrowedVault.vault_id]}
                      demoMode={demoMode}
                    />
                  ) : undefined
                }
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
      </div>

      {/* One action only. A footer with three things in it becomes the next
          grab-bag, which is what this redesign was fixing. */}
      {writeEnabled ? (
        <div className="explorer-footer">
          <UiButton
            className="close-note explorer-new-note"
            onClick={() => onCreateNoteInFolder("")}
          >
            New note
          </UiButton>
        </div>
      ) : null}
    </aside>
  );
}

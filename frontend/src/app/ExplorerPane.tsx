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

function countNotes(folder: ExplorerFolder): number {
  return (
    folder.notes.length +
    folder.folders.reduce((sum, f) => sum + countNotes(f), 0)
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
}) {
  const rowRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const rowIds: VaultScope[] = ["all", ...vaults.map((vault) => vault.vault_id)];
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

  if (vaults.length <= 1) {
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

  const viewingVault = vaults.find((vault) => vault.vault_id === viewingVaultId);
  // The collapsed head names the scope in the worst ink present across every
  // enabled Vault, not just the selected one, so narrowing scope never hides
  // trouble elsewhere (#116, amended by #117).
  const aggregate = deriveVaultAggregate(vaults, noteCounts);
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
            <VaultAggregateSlot vaults={vaults} counts={noteCounts} />
          </>
        ) : (
          <span className="side-count">
            {String(vaults.length).padStart(2, "0")}
          </span>
        )}
      </button>

      {collapsed && viewingVault ? (
        <p className="scope-zone-viewing-line">Viewing: {viewingVault.name}</p>
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
              <VaultAggregateSlot vaults={vaults} counts={noteCounts} />
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
                  <span className="scope-row-viewing">Viewing</span>
                ) : null}
                <VaultSlot vault={vault} noteCount={noteCounts[vault.vault_id]} />
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
  onCreateNoteInFolder: (folderPath: string) => void;
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
                onCreateNoteInFolder={onCreateNoteInFolder}
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
  onCreateNoteInFolder: (folderPath: string) => void;
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
    if (vaultId === unfoldedVaultId) {
      return;
    }
    setUnfoldedVaultId(vaultId);
    setStoredUnfoldedVault(vaultId);
  };

  const showAccordion = scope === "all" && vaults.length > 1;
  const narrowedVault =
    scope !== "all"
      ? vaults.find((vault) => vault.vault_id === scope)
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

        {loadingTree ? <ExplorerSkeleton /> : null}
        {!loadingTree && treeError && !tree ? (
          <StateBlock
            title="Explorer Unavailable"
            description={treeError}
            actionLabel="Retry"
            onAction={onRefreshTree}
          />
        ) : null}
        {showAccordion ? (
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
                onCreateNoteInFolder={onCreateNoteInFolder}
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

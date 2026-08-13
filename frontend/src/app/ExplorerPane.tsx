import { useState, type RefObject } from "react";
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
import type {
  ExplorerFolder,
  ModifiedNote,
  RecentNote,
  VaultId,
  VaultScope,
  VaultSummary,
} from "../types";

function countNotes(folder: ExplorerFolder): number {
  return (
    folder.notes.length +
    folder.folders.reduce((sum, f) => sum + countNotes(f), 0)
  );
}

function scopeName(scope: VaultScope, vaults: VaultSummary[]): string {
  if (scope === "all") {
    return "All Vaults";
  }
  return vaults.find((vault) => vault.vault_id === scope)?.name ?? "All Vaults";
}

/**
 * Vault scope, readable and changeable in exactly one place on the desktop
 * (#138): a collapsible zone pinned above the rail, never scrolling with the
 * notes. Absent at one enabled Vault or on mobile — narrowing scope has
 * nothing to offer there (mobile scope chrome is #145's ticket).
 */
function ScopeZone({
  vaults,
  scope,
  onScopeChange,
  viewingVaultId,
  collapsed,
  onToggleCollapsed,
}: {
  vaults: VaultSummary[];
  scope: VaultScope;
  onScopeChange: (next: VaultScope) => void;
  viewingVaultId: VaultId | undefined;
  collapsed: boolean;
  onToggleCollapsed: () => void;
}) {
  if (vaults.length <= 1) {
    return null;
  }

  const viewingVault = vaults.find((vault) => vault.vault_id === viewingVaultId);

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
        <span className="side-rule" />
        {collapsed ? (
          <span className="scope-zone-current">
            {scopeName(scope, vaults)}
          </span>
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
        <ul id="scope-zone-list" className="scope-zone-list">
          <li>
            <button
              type="button"
              className={`scope-row${scope === "all" ? " is-selected" : ""}`}
              onClick={() => onScopeChange("all")}
            >
              <span className="scope-row-label">All Vaults</span>
            </button>
          </li>
          {vaults.map((vault) => (
            <li key={vault.vault_id}>
              <button
                type="button"
                className={`scope-row${scope === vault.vault_id ? " is-selected" : ""}`}
                onClick={() => onScopeChange(vault.vault_id)}
              >
                <span className="scope-row-label">{vault.name}</span>
                {viewingVaultId === vault.vault_id ? (
                  <span className="scope-row-viewing">Viewing</span>
                ) : null}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
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
  loadingTree: boolean;
  treeError: string | null;
  tree: ExplorerFolder | null;
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
  loadingTree,
  treeError,
  tree,
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
}: ExplorerPaneProps) {
  // Local, not lifted: the shell already carries a large prop surface, and the
  // module map is explicit that this is a coordination seam rather than an
  // invitation to move feature state into it.
  const [changesOpen, setChangesOpen] = useState(false);

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
        {tree ? <SideHead label="Notes" count={countNotes(tree)} /> : null}
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

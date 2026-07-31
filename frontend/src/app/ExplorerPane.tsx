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
import type { ExplorerFolder, ModifiedNote, RecentNote } from "../types";

function countNotes(folder: ExplorerFolder): number {
  return (
    folder.notes.length +
    folder.folders.reduce((sum, f) => sum + countNotes(f), 0)
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
}: {
  changesOpen: boolean;
  onToggleChanges: () => void;
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
      {/*
       * Settings has no destination until issue #13 exists. It reserves the
       * slot so the layout does not shift when it becomes real.
       *
       * A real <button> with aria-disabled, not `disabled` and not a span: it
       * stays focusable, so a keyboard user can reach it and hear why it does
       * nothing. `disabled` would remove it from the tab order and make the
       * explanation mouse-only, which is how a dead control reads as a bug.
       */}
      <button
        type="button"
        className="explorer-rail-item is-disabled"
        aria-disabled="true"
        aria-label="Settings (not yet available)"
        title="Settings (not yet available)"
        onClick={(event) => event.preventDefault()}
      >
        <SettingsIcon />
      </button>
    </div>
  );
}

type ExplorerPaneProps = {
  explorerScrollRef: RefObject<HTMLElement | null>;
  drawerOpen: boolean;
  writeEnabled: boolean;
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
};

export function ExplorerPane({
  explorerScrollRef,
  drawerOpen,
  writeEnabled,
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
}: ExplorerPaneProps) {
  // Local, not lifted: the shell already carries a large prop surface, and the
  // module map is explicit that this is a coordination seam rather than an
  // invitation to move feature state into it.
  const [changesOpen, setChangesOpen] = useState(false);

  return (
    <aside className="explorer-pane" data-open={drawerOpen}>
      <ExplorerRail
        changesOpen={changesOpen}
        onToggleChanges={() => setChangesOpen((prev) => !prev)}
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
          <ChangesPanel notes={modifiedNotes} onNavigate={onCloseDrawer} />
        ) : null}

        <RecentNotesList
          notes={recentNotes}
          onNavigate={onCloseDrawer}
          collapsed={recentCollapsed}
          onToggleCollapsed={() => onRecentCollapsedChange(!recentCollapsed)}
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

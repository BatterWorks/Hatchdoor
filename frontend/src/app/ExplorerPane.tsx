import type { RefObject } from "react";
import { NavLink } from "react-router-dom";

import {
  FolderTree,
  LastModifiedNotesList,
  RecentNotesList,
} from "../components/Explorer";
import { ExplorerSkeleton, StateBlock, UiButton } from "../components/ui";
import type { ExplorerFolder, ModifiedNote, RecentNote } from "../types";

function countNotes(folder: ExplorerFolder): number {
  return (
    folder.notes.length +
    folder.folders.reduce((sum, f) => sum + countNotes(f), 0)
  );
}

type ExplorerPaneProps = {
  explorerPaneRef: RefObject<HTMLElement | null>;
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
  onExpandedFoldersChange: (next: Record<string, boolean>) => void;
  onCloseDrawer: () => void;
  onRefreshTree: () => void;
  onScrollTopChange: (top: number) => void;
};

export function ExplorerPane({
  explorerPaneRef,
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
  onExpandedFoldersChange,
  onCloseDrawer,
  onRefreshTree,
  onScrollTopChange,
}: ExplorerPaneProps) {
  return (
    <aside
      ref={explorerPaneRef}
      className="explorer-pane"
      data-open={drawerOpen}
      onScroll={(event) => {
        onScrollTopChange(event.currentTarget.scrollTop);
      }}
    >
      <header className="explorer-header">
        <p>Vault Explorer</p>
        <div className="explorer-actions">
          {writeEnabled ? (
            <UiButton
              className="close-note"
              onClick={() => onCreateNoteInFolder("")}
            >
              New
            </UiButton>
          ) : null}
          <UiButton className="close-note" onClick={onRefreshTree}>
            Refresh
          </UiButton>
        </div>
      </header>

      <div className="explorer-page-links">
        <NavLink
          className={({ isActive }) =>
            `explorer-page-link${isActive ? " active" : ""}`
          }
          to="/stats"
        >
          Stats
        </NavLink>
        <NavLink
          className={({ isActive }) =>
            `explorer-page-link${isActive ? " active" : ""}`
          }
          to="/graph"
        >
          Graph
        </NavLink>
      </div>

      <RecentNotesList
        notes={recentNotes}
        currentPath={locationPathname}
        onNavigate={onCloseDrawer}
      />

      <LastModifiedNotesList
        notes={modifiedNotes}
        currentPath={locationPathname}
        onNavigate={onCloseDrawer}
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
      {tree ? (
        <p className="explorer-notes-label">
          Notes{" "}
          <span className="explorer-notes-count">{countNotes(tree)}</span>
        </p>
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
    </aside>
  );
}

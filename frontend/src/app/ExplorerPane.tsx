import type { RefObject } from "react";

import {
  FolderTree,
  LastModifiedNotesList,
  RecentNotesList,
} from "../components/Explorer";
import { ExplorerSkeleton, StateBlock, UiButton } from "../components/ui";
import type { ExplorerFolder, ModifiedNote, RecentNote } from "../types";

type ExplorerPaneProps = {
  explorerPaneRef: RefObject<HTMLElement | null>;
  drawerOpen: boolean;
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
          <UiButton className="close-note" onClick={onRefreshTree}>
            Refresh
          </UiButton>
        </div>
      </header>

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
        <FolderTree
          root={tree}
          currentPath={locationPathname}
          expandedFolders={expandedFolders}
          onExpandedFoldersChange={onExpandedFoldersChange}
        />
      ) : null}
    </aside>
  );
}

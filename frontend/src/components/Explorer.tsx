import { NavLink } from "react-router-dom";
import { useMemo } from "react";

import type {
  ExplorerFolder,
  ExplorerNote,
  RecentNote,
  VaultScope,
  VaultSummary,
} from "../types";
import { AddIcon } from "./icons";
import { UiPanel, VaultPrefix } from "./ui";

/** Section header: `01 · RECENT · ──── · 04`, per §05 of the design system. */
export function SideHead({
  label,
  count,
  collapsible,
  open,
  controls,
  onToggle,
}: {
  label: string;
  count?: number;
  collapsible?: boolean;
  open?: boolean;
  controls?: string;
  onToggle?: () => void;
}) {
  const inner = (
    <>
      {collapsible ? <span className="side-caret" aria-hidden="true" /> : null}
      <span className="side-label">{label}</span>
      <span className="side-rule" />
      {count === undefined ? null : (
        <span className="side-count">{String(count).padStart(2, "0")}</span>
      )}
    </>
  );

  if (!collapsible) {
    return <div className="side-head">{inner}</div>;
  }

  return (
    <button
      type="button"
      className="side-head"
      data-open={open}
      aria-expanded={open}
      aria-controls={controls}
      onClick={onToggle}
    >
      {inner}
    </button>
  );
}

export function RecentNotesList({
  notes,
  onNavigate,
  collapsed,
  onToggleCollapsed,
  vaults,
  scope,
}: {
  notes: RecentNote[];
  onNavigate: () => void;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  vaults: VaultSummary[];
  scope: VaultScope;
}) {
  if (notes.length === 0) {
    return null;
  }
  const recent = notes.slice(0, 5);
  // Provenance only where the list can actually span Vaults (#140).
  const showVaultPrefix = scope === "all" && vaults.length > 1;

  return (
    <UiPanel className="recent-notes" data-testid="recent-notes">
      <SideHead
        label="Recently viewed"
        count={recent.length}
        collapsible
        open={!collapsed}
        controls="recent-notes-list"
        onToggle={onToggleCollapsed}
      />
      {collapsed ? null : (
        <ul id="recent-notes-list" className="tree root-tree">
          {recent.map((note, index) => (
            <li key={note.slug} className="note-item">
              {/* No active-note class here. The highlight is canonical in the
                  tree only; applying it in several lists at once is the bug
                  issue #12 reported. */}
              <NavLink
                className="note-link"
                to={`/v/${encodeURIComponent(note.vaultId)}/n/${note.slug}`}
                onClick={onNavigate}
                title={`${note.relativePath}.md`}
              >
                <span className="idx" aria-hidden="true">
                  {String(index + 1).padStart(3, "0")}
                </span>
                {showVaultPrefix ? (
                  <VaultPrefix
                    name={
                      vaults.find((vault) => vault.vault_id === note.vaultId)
                        ?.name ?? note.vaultId
                    }
                  />
                ) : null}
                <span className="note-label">{note.title}</span>
              </NavLink>
            </li>
          ))}
        </ul>
      )}
    </UiPanel>
  );
}

export function FolderTree({
  root,
  currentPath,
  expandedFolders,
  onExpandedFoldersChange,
  writeEnabled,
  onCreateNoteInFolder,
}: {
  root: ExplorerFolder;
  currentPath: string;
  expandedFolders: Record<string, boolean>;
  onExpandedFoldersChange: (expanded: Record<string, boolean>) => void;
  writeEnabled: boolean;
  onCreateNoteInFolder: (folderPath: string) => void;
}) {
  const current = pathToNoteIdentity(currentPath);
  const activePathFolders = useMemo(
    () => collectAncestorFolderPaths(root, current),
    [current, root],
  );

  return (
    <ul className="tree root-tree">
      {root.folders.map((folder) => (
        <FolderNode
          key={`folder-${folder.name}`}
          folder={folder}
          currentPath={currentPath}
          folderPath={folder.name}
          expandedFolders={expandedFolders}
          activePathFolders={activePathFolders}
          writeEnabled={writeEnabled}
          onCreateNoteInFolder={onCreateNoteInFolder}
          onToggleFolder={(path, open) =>
            onExpandedFoldersChange({ ...expandedFolders, [path]: open })
          }
        />
      ))}
      {root.notes.map((note, index) => (
        <NoteNode
          key={note.slug}
          note={note}
          currentPath={currentPath}
          index={index}
        />
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
  writeEnabled,
  onCreateNoteInFolder,
  onToggleFolder,
}: {
  folder: ExplorerFolder;
  currentPath: string;
  folderPath: string;
  expandedFolders: Record<string, boolean>;
  activePathFolders: Set<string>;
  writeEnabled: boolean;
  onCreateNoteInFolder: (folderPath: string) => void;
  onToggleFolder: (path: string, open: boolean) => void;
}) {
  const shouldOpen =
    activePathFolders.has(folderPath) || expandedFolders[folderPath] === true;

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
        <summary title={folderPath}>
          <span className="folder-label">{folder.name}</span>
          {writeEnabled ? (
            <button
              type="button"
              className="folder-new-note"
              aria-label={`New note in ${folderPath}`}
              title={`New note in ${folderPath}`}
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                onCreateNoteInFolder(folderPath);
              }}
            >
              <AddIcon />
            </button>
          ) : null}
        </summary>
        <ul className="tree">
          {folder.folders.map((child) => (
            <FolderNode
              key={`${folder.name}-${child.name}`}
              folder={child}
              currentPath={currentPath}
              folderPath={`${folderPath}/${child.name}`}
              expandedFolders={expandedFolders}
              activePathFolders={activePathFolders}
              writeEnabled={writeEnabled}
              onCreateNoteInFolder={onCreateNoteInFolder}
              onToggleFolder={onToggleFolder}
            />
          ))}
          {folder.notes.map((note, index) => (
            <NoteNode
              key={note.slug}
              note={note}
              currentPath={currentPath}
              index={index}
            />
          ))}
        </ul>
      </details>
    </li>
  );
}

function NoteNode({
  note,
  currentPath,
  index,
}: {
  note: ExplorerNote;
  currentPath: string;
  index: number;
}) {
  return (
    <li className="note-item">
      <NavLink
        className={
          currentPath ===
          `/v/${encodeURIComponent(note.vault_id)}/n/${note.slug}`
            ? "note-link active-note"
            : "note-link"
        }
        to={`/v/${encodeURIComponent(note.vault_id)}/n/${note.slug}`}
        title={`${note.title}.md`}
      >
        {/* §05: folders carry the caret, notes carry a mono index. This is what
            tells the two row kinds apart without changing size or weight. */}
        <span className="idx" aria-hidden="true">
          {String(index + 1).padStart(3, "0")}
        </span>
        <span className="note-label">{note.title}</span>
      </NavLink>
    </li>
  );
}

type NoteIdentity = { vaultId: string; slug: string };

function pathToNoteIdentity(pathname: string): NoteIdentity | null {
  const match = pathname.match(/^\/v\/([^/]+)\/n\/([^/]+)$/);
  if (!match) {
    return null;
  }

  return {
    vaultId: decodeURIComponent(match[1]),
    slug: decodeURIComponent(match[2]),
  };
}

function collectAncestorFolderPaths(
  root: ExplorerFolder,
  current: NoteIdentity | null,
): Set<string> {
  const paths = new Set<string>();
  if (!current) {
    return paths;
  }

  const visit = (folder: ExplorerFolder, folderPath: string): boolean => {
    if (
      folder.notes.some(
        (note) =>
          note.slug === current.slug && note.vault_id === current.vaultId,
      )
    ) {
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

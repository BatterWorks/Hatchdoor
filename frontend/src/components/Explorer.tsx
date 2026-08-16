import { NavLink } from "react-router-dom";
import { useMemo, type ReactNode } from "react";

import type {
  ExplorerFolder,
  ExplorerNote,
  RecentNote,
  VaultScope,
  VaultSummary,
} from "../types";
import { AddIcon } from "./icons";
import { UiPanel, VaultPrefix } from "./ui";
import { pathToNoteIdentity, type NoteIdentity } from "../lib/notePath";

/** Section header: `01 · RECENT · ──── · 04`, per §05 of the design system.
 * `slot`, when given, replaces the plain mono `count` with arbitrary
 * trailing content (the count-or-condition slot, #142); `disabled` marks a
 * collapsible head `aria-disabled` and inert without removing it from the
 * tab order (#116's precedent, same as the rail's Settings slot). */
export function SideHead({
  label,
  count,
  slot,
  collapsible,
  open,
  controls,
  onToggle,
  disabled,
  className,
}: {
  label: string;
  count?: number;
  slot?: ReactNode;
  collapsible?: boolean;
  open?: boolean;
  controls?: string;
  onToggle?: () => void;
  disabled?: boolean;
  className?: string;
}) {
  const inner = (
    <>
      {collapsible ? <span className="side-caret" aria-hidden="true" /> : null}
      <span className="side-label">{label}</span>
      <span className="side-rule" />
      {slot !== undefined ? (
        slot
      ) : count === undefined ? null : (
        <span className="side-count">{String(count).padStart(2, "0")}</span>
      )}
    </>
  );

  const classes = className ? `side-head ${className}` : "side-head";

  if (!collapsible) {
    return <div className={classes}>{inner}</div>;
  }

  return (
    <button
      type="button"
      className={classes}
      data-open={open}
      aria-expanded={disabled ? undefined : open}
      aria-disabled={disabled || undefined}
      aria-controls={controls}
      onClick={disabled ? undefined : onToggle}
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
  // A note whose Vault has left the collection cannot be opened — its link
  // resolves to "Vault definition was not found" — and it has no name to show
  // but its own raw UUID. Drop those rather than offer a dead row. Discovery
  // in flight leaves `vaults` a temporary `[]`, which would empty the list on
  // every load, so filter only once the real collection has arrived.
  const known =
    vaults.length === 0
      ? notes
      : notes.filter((note) =>
          vaults.some((vault) => vault.vault_id === note.vaultId),
        );
  if (known.length === 0) {
    return null;
  }
  const recent = known.slice(0, 5);
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

import { NavLink } from "react-router-dom";
import { useMemo } from "react";

import type {
  ExplorerFolder,
  ExplorerNote,
  ModifiedNote,
  RecentNote,
} from "../types";
import { UiPanel } from "./ui";

export function RecentNotesList({
  notes,
  currentPath,
  onNavigate,
}: {
  notes: RecentNote[];
  currentPath: string;
  onNavigate: () => void;
}) {
  if (notes.length === 0) {
    return null;
  }
  const recent = notes.slice(0, 5);

  return (
    <UiPanel className="recent-notes" data-testid="recent-notes">
      <p className="recent-notes-title">Recently Viewed</p>
      <ul className="tree root-tree">
        {recent.map((note) => (
          <li key={note.slug} className="note-item">
            <NavLink
              className={
                currentPath === `/n/${note.slug}`
                  ? "note-link active-note"
                  : "note-link"
              }
              to={`/n/${note.slug}`}
              onClick={onNavigate}
              title={`${note.relativePath}.md`}
            >
              <span className="note-label">{note.title}</span>
            </NavLink>
          </li>
        ))}
      </ul>
    </UiPanel>
  );
}

export function LastModifiedNotesList({
  notes,
  currentPath,
  onNavigate,
}: {
  notes: ModifiedNote[];
  currentPath: string;
  onNavigate: () => void;
}) {
  if (notes.length === 0) {
    return null;
  }

  return (
    <UiPanel className="recent-notes" data-testid="last-modified-notes">
      <p className="recent-notes-title">Last Modified</p>
      <ul className="tree root-tree">
        {notes.map((note) => (
          <li key={note.slug} className="note-item">
            <NavLink
              className={
                currentPath === `/n/${note.slug}`
                  ? "note-link active-note"
                  : "note-link"
              }
              to={`/n/${note.slug}`}
              onClick={onNavigate}
              title={`${note.relative_path}.md`}
            >
              <span className="note-label">{note.title}</span>
            </NavLink>
          </li>
        ))}
      </ul>
    </UiPanel>
  );
}

export function FolderTree({
  root,
  currentPath,
  expandedFolders,
  onExpandedFoldersChange,
}: {
  root: ExplorerFolder;
  currentPath: string;
  expandedFolders: Record<string, boolean>;
  onExpandedFoldersChange: (expanded: Record<string, boolean>) => void;
}) {
  const currentSlug = pathToNoteSlug(currentPath);
  const activePathFolders = useMemo(
    () => collectAncestorFolderPaths(root, currentSlug),
    [currentSlug, root],
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
          onToggleFolder={(path, open) =>
            onExpandedFoldersChange({ ...expandedFolders, [path]: open })
          }
        />
      ))}
      {root.notes.map((note) => (
        <NoteNode key={note.slug} note={note} currentPath={currentPath} />
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
  onToggleFolder,
}: {
  folder: ExplorerFolder;
  currentPath: string;
  folderPath: string;
  expandedFolders: Record<string, boolean>;
  activePathFolders: Set<string>;
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
              onToggleFolder={onToggleFolder}
            />
          ))}
          {folder.notes.map((note) => (
            <NoteNode key={note.slug} note={note} currentPath={currentPath} />
          ))}
        </ul>
      </details>
    </li>
  );
}

function NoteNode({
  note,
  currentPath,
}: {
  note: ExplorerNote;
  currentPath: string;
}) {
  return (
    <li className="note-item">
      <NavLink
        className={
          currentPath === `/n/${note.slug}`
            ? "note-link active-note"
            : "note-link"
        }
        to={`/n/${note.slug}`}
        title={`${note.title}.md`}
      >
        <span className="note-label">{note.title}</span>
      </NavLink>
    </li>
  );
}

function pathToNoteSlug(pathname: string): string | null {
  const match = pathname.match(/^\/n\/([^/]+)$/);
  if (!match) {
    return null;
  }

  return decodeURIComponent(match[1]);
}

function collectAncestorFolderPaths(
  root: ExplorerFolder,
  slug: string | null,
): Set<string> {
  const paths = new Set<string>();
  if (!slug) {
    return paths;
  }

  const visit = (folder: ExplorerFolder, folderPath: string): boolean => {
    if (folder.notes.some((note) => note.slug === slug)) {
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

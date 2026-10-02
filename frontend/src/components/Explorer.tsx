import { NavLink } from "react-router-dom";
import { useMemo, useState, type ReactNode } from "react";

import type {
  ExplorerFolder,
  ExplorerNote,
  RecentNote,
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
}: {
  notes: RecentNote[];
  onNavigate: () => void;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  vaults: VaultSummary[];
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
  // This is a viewing history, not a collection read: it spans Vaults
  // whatever the browsing scope is, so provenance follows the Vault count
  // alone. Keying it to `scope === "all"` stripped the prefix at a narrowed
  // scope while other Vaults' rows stayed listed (#334).
  const showVaultPrefix = vaults.length > 1;

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
            // A slug is unique only within its Vault (#137).
            <li key={`${note.vaultId}-${note.slug}`} className="note-item">
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

/** A change to the folder-open record, applied by its owner to the latest
 * state. Two folders toggling in one batch each get the other's write, which
 * a whole next record built from a render's snapshot would drop (#305). */
export type ExpandedFoldersUpdate = (
  previous: Record<string, boolean>,
) => Record<string, boolean>;

const NO_FOLDERS: ReadonlySet<string> = new Set();

type ClosedByReader = { currentPath: string; paths: ReadonlySet<string> };

/** The folders the reader closed while `currentPath` was the open note. A
 * record kept for another note no longer counts. */
function closedFor(
  closed: ClosedByReader,
  currentPath: string,
): ReadonlySet<string> {
  return closed.currentPath === currentPath ? closed.paths : NO_FOLDERS;
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
  onExpandedFoldersChange: (update: ExpandedFoldersUpdate) => void;
  writeEnabled: boolean;
  onCreateNoteInFolder: (folderPath: string) => void;
}) {
  // Keyed on the pathname, not on the parsed identity: `pathToNoteIdentity`
  // returns a fresh object every call, so a dependency on it changed on every
  // render and this walked the whole tree each time instead of never.
  const activePathFolders = useMemo(
    () => collectAncestorFolderPaths(root, pathToNoteIdentity(currentPath)),
    [currentPath, root],
  );
  // Folders above the open note that the reader closed while it was open.
  // Showing a note's folders is temporary and never saved (#365), so this
  // lives here rather than in the record, and belongs to one open note: when
  // the note changes it is empty again, and a folder holding the new note
  // opens to show it.
  const [closedByReader, setClosedByReader] = useState<ClosedByReader>(() => ({
    currentPath,
    paths: NO_FOLDERS,
  }));
  // Forget them as soon as the note changes, not merely while it differs:
  // returning to a note later is opening it again, and must show its folders.
  if (closedByReader.currentPath !== currentPath) {
    setClosedByReader({ currentPath, paths: NO_FOLDERS });
  }
  const closedForThisNote = closedFor(closedByReader, currentPath);
  const foldersShownForNote = useMemo(
    () =>
      new Set(
        [...activePathFolders].filter((path) => !closedForThisNote.has(path)),
      ),
    [activePathFolders, closedForThisNote],
  );

  const onToggleFolder = (path: string, open: boolean) => {
    setClosedByReader((previous) => {
      const paths = new Set(closedFor(previous, currentPath));
      if (open) {
        paths.delete(path);
      } else if (activePathFolders.has(path)) {
        paths.add(path);
      }
      return { currentPath, paths };
    });
    onExpandedFoldersChange((previous) => ({ ...previous, [path]: open }));
  };

  return (
    <ul className="tree root-tree">
      {root.folders.map((folder) => (
        <FolderNode
          key={`folder-${folder.name}`}
          folder={folder}
          currentPath={currentPath}
          folderPath={folder.name}
          expandedFolders={expandedFolders}
          foldersShownForNote={foldersShownForNote}
          writeEnabled={writeEnabled}
          onCreateNoteInFolder={onCreateNoteInFolder}
          onToggleFolder={onToggleFolder}
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
  foldersShownForNote,
  writeEnabled,
  onCreateNoteInFolder,
  onToggleFolder,
}: {
  folder: ExplorerFolder;
  currentPath: string;
  folderPath: string;
  expandedFolders: Record<string, boolean>;
  foldersShownForNote: ReadonlySet<string>;
  writeEnabled: boolean;
  onCreateNoteInFolder: (folderPath: string) => void;
  onToggleFolder: (path: string, open: boolean) => void;
}) {
  const shouldOpen =
    foldersShownForNote.has(folderPath) || expandedFolders[folderPath] === true;
  // What the element itself last reported. The browser opens a <details>
  // before any state hears about it, so the children follow this as well as
  // `shouldOpen`: a folder the reader sees open always has its contents
  // mounted, even if the record never caught up (#305).
  const [elementOpen, setElementOpen] = useState(shouldOpen);
  const showChildren = shouldOpen || elementOpen;

  return (
    <li className="folder-item">
      <details
        open={shouldOpen}
        onToggle={(event) => {
          // React dispatches `toggle` through every ancestor with an
          // `onToggle`, so an enclosing folder sees its descendants' toggles
          // too. Only this folder's own element speaks for this folder.
          if (event.target !== event.currentTarget) {
            return;
          }
          const open = event.currentTarget.open;
          setElementOpen(open);
          // The browser fires `toggle` when `open` changes for any reason,
          // and does not say why. When the element now matches what this
          // render asked for, React set it: the folder mounted open, or the
          // open note moved. Only the reader's own toggle disagrees with the
          // prop, and only that is saved (#365).
          if (open === shouldOpen) {
            return;
          }
          onToggleFolder(folderPath, open);
        }}
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
        {/* A closed folder renders nothing inside it. The browser hides a
            collapsed <details>' content either way, so mounting it bought
            nothing but DOM — on a whole Vault that is the difference between
            a handful of rows and one per note. The cost is that find-in-page
            no longer reaches a note in a collapsed folder on the browsers
            that looked inside one; the in-app search does. */}
        <ul className="tree">
          {showChildren ? (
            <>
              {folder.folders.map((child) => (
                <FolderNode
                  key={`${folder.name}-${child.name}`}
                  folder={child}
                  currentPath={currentPath}
                  folderPath={`${folderPath}/${child.name}`}
                  expandedFolders={expandedFolders}
                  foldersShownForNote={foldersShownForNote}
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
            </>
          ) : null}
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

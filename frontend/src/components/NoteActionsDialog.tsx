import { useEffect, useRef } from "react";

import { UiButton } from "./ui";

export type NoteActionDialogKind = "create" | "rename" | "move" | "delete";

const DIALOG_TITLE: Record<NoteActionDialogKind, string> = {
  create: "Create note",
  rename: "Rename note",
  move: "Move note",
  delete: "Delete note",
};

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function NoteActionsDialog({
  kind,
  error,
  folderPaths = [],
  initialFolder = "",
  onClose,
  onCreate,
  onRename,
  onMove,
  onDelete,
}: {
  kind: NoteActionDialogKind;
  error: string | null;
  folderPaths?: string[];
  initialFolder?: string;
  onClose: () => void;
  onCreate: (relativePath: string, content: string) => void;
  onRename: (newTitle: string) => void;
  onMove: (targetFolder: string) => void;
  onDelete: () => void;
}) {
  const backdropRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const root = backdropRef.current;
    if (!root) {
      return;
    }

    const focusable = () =>
      Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));

    focusable()[0]?.focus();

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const items = focusable();
      if (items.length === 0) {
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    root.addEventListener("keydown", onKeyDown);
    return () => root.removeEventListener("keydown", onKeyDown);
  }, [kind, onClose]);

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      ref={backdropRef}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <section
        className="modal-panel"
        role="dialog"
        aria-modal="true"
        aria-label={DIALOG_TITLE[kind]}
      >
        {kind === "create" ? (
          <CreateForm
            error={error}
            folderPaths={folderPaths}
            initialFolder={initialFolder}
            onClose={onClose}
            onCreate={onCreate}
          />
        ) : null}
        {kind === "rename" ? (
          <RenameForm error={error} onClose={onClose} onRename={onRename} />
        ) : null}
        {kind === "move" ? (
          <MoveForm
            error={error}
            folderPaths={folderPaths}
            onClose={onClose}
            onMove={onMove}
          />
        ) : null}
        {kind === "delete" ? (
          <DeleteForm error={error} onClose={onClose} onDelete={onDelete} />
        ) : null}
      </section>
    </div>
  );
}

function FolderDatalist({
  id,
  folderPaths,
}: {
  id: string;
  folderPaths: string[];
}) {
  return (
    <datalist id={id}>
      {folderPaths.map((path) => (
        <option key={path} value={path} />
      ))}
    </datalist>
  );
}

function CreateForm({
  error,
  folderPaths,
  initialFolder,
  onClose,
  onCreate,
}: {
  error: string | null;
  folderPaths: string[];
  initialFolder: string;
  onClose: () => void;
  onCreate: (relativePath: string, content: string) => void;
}) {
  const listId = "create-folder-options";
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        const folder = String(data.get("folder") ?? "").trim();
        const name = String(data.get("name") ?? "").trim();
        const relativePath = folder ? `${folder}/${name}` : name;
        onCreate(relativePath, String(data.get("content") ?? ""));
      }}
    >
      <h2>Create note</h2>
      <label>
        Folder
        <input
          name="folder"
          aria-label="Folder"
          list={listId}
          defaultValue={initialFolder}
          placeholder="Vault root"
        />
      </label>
      <FolderDatalist id={listId} folderPaths={folderPaths} />
      <label>
        Note name
        <input name="name" aria-label="Note name" placeholder="My Note" />
      </label>
      <label>
        Markdown content
        <textarea name="content" aria-label="Markdown content" />
      </label>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <div className="modal-actions">
        <UiButton type="submit">Create</UiButton>
        <UiButton type="button" className="close-note" onClick={onClose}>
          Cancel
        </UiButton>
      </div>
    </form>
  );
}

function RenameForm({
  error,
  onClose,
  onRename,
}: {
  error: string | null;
  onClose: () => void;
  onRename: (newTitle: string) => void;
}) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        onRename(String(data.get("newTitle") ?? ""));
      }}
    >
      <h2>Rename note</h2>
      <label>
        New title
        <input name="newTitle" aria-label="New title" />
      </label>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <div className="modal-actions">
        <UiButton type="submit">Rename</UiButton>
        <UiButton type="button" className="close-note" onClick={onClose}>
          Cancel
        </UiButton>
      </div>
    </form>
  );
}

function MoveForm({
  error,
  folderPaths,
  onClose,
  onMove,
}: {
  error: string | null;
  folderPaths: string[];
  onClose: () => void;
  onMove: (targetFolder: string) => void;
}) {
  const listId = "move-folder-options";
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        onMove(String(data.get("targetFolder") ?? ""));
      }}
    >
      <h2>Move note</h2>
      <label>
        Target folder
        <input
          name="targetFolder"
          aria-label="Target folder"
          list={listId}
          placeholder="Vault root"
        />
      </label>
      <FolderDatalist id={listId} folderPaths={folderPaths} />
      {error ? <p className="note-editor-error">{error}</p> : null}
      <div className="modal-actions">
        <UiButton type="submit">Move</UiButton>
        <UiButton type="button" className="close-note" onClick={onClose}>
          Cancel
        </UiButton>
      </div>
    </form>
  );
}

function DeleteForm({
  error,
  onClose,
  onDelete,
}: {
  error: string | null;
  onClose: () => void;
  onDelete: () => void;
}) {
  return (
    <div>
      <h2>Delete note</h2>
      <p>This moves the note to Hatchdoor trash using the current content hash.</p>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <div className="modal-actions">
        <UiButton onClick={onDelete}>Delete</UiButton>
        <UiButton className="close-note" onClick={onClose}>
          Cancel
        </UiButton>
      </div>
    </div>
  );
}

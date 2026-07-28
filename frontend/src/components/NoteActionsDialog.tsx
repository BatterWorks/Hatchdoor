import { useEffect, useRef, useState } from "react";

import { UiButton } from "./ui";
import {
  clearCreateDraft,
  loadCreateDraft,
  saveCreateDraft,
} from "../lib/writeDrafts";

export type NoteActionDialogKind =
  "create" | "rename" | "move" | "archive" | "delete";

const DIALOG_TITLE: Record<NoteActionDialogKind, string> = {
  create: "Create note",
  rename: "Rename note",
  move: "Move note",
  archive: "Archive note",
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
  onArchive,
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
  onArchive: () => void;
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
        {kind === "archive" ? (
          <ArchiveForm error={error} onClose={onClose} onArchive={onArchive} />
        ) : null}
        {kind === "delete" ? (
          <DeleteForm error={error} onClose={onClose} onDelete={onDelete} />
        ) : null}
      </section>
    </div>
  );
}

/** Sentinel option value; not a path, so it cannot collide with a real folder. */
const NEW_FOLDER = "//new-folder";

/**
 * Folder chooser, shared by create and move.
 *
 * Replaces a free-text box plus every folder rendered as a chip, which did not
 * scale and gave no hint that the field wanted a path. Folder names are shown
 * verbatim, numeric prefixes included: those prefixes are the real names and
 * encode a deliberate order.
 *
 * Typing a new folder name stays possible through the last option. The string
 * is passed straight through — `useNoteActions` runs `validateNotePath` on it
 * and the backend's `vault/write` path checks remain authoritative. Nothing
 * here is a safety boundary.
 */
function FolderPicker({
  label,
  folderPaths,
  value,
  onChange,
}: {
  label: string;
  folderPaths: string[];
  value: string;
  onChange: (next: string) => void;
}) {
  const [creating, setCreating] = useState(
    value !== "" && !folderPaths.includes(value),
  );

  return (
    <div className="field">
      <label className="field-label" htmlFor="folder-picker">
        {label}
      </label>
      <select
        id="folder-picker"
        className="field-input"
        aria-label={label}
        value={creating ? NEW_FOLDER : value}
        onChange={(event) => {
          if (event.target.value === NEW_FOLDER) {
            setCreating(true);
            onChange("");
            return;
          }
          setCreating(false);
          onChange(event.target.value);
        }}
      >
        <option value="">Vault root</option>
        {folderPaths.map((path) => (
          <option key={path} value={path}>
            {path}
          </option>
        ))}
        <option value={NEW_FOLDER}>New folder…</option>
      </select>
      {creating ? (
        <input
          className="field-input field-input-nested"
          aria-label="New folder name"
          placeholder="10-topics/subfolder"
          value={value}
          onChange={(event) => onChange(event.target.value)}
        />
      ) : null}
    </div>
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
  // Restore any draft persisted from a previous session (e.g. an in-progress
  // note wiped by a service-worker autoUpdate reload). Fall back to props.
  const [draft] = useState(() => loadCreateDraft());
  const [folder, setFolder] = useState(draft?.folder ?? initialFolder);
  const [name, setName] = useState(draft?.name ?? "");
  const [content, setContent] = useState(draft?.content ?? "");

  const persist = (next: { folder: string; name: string; content: string }) => {
    if (!next.folder && !next.name && !next.content) {
      clearCreateDraft();
      return;
    }
    saveCreateDraft({ ...next, savedAt: Date.now() });
  };

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const trimmedFolder = folder.trim();
        const trimmedName = name.trim();
        const relativePath = trimmedFolder
          ? `${trimmedFolder}/${trimmedName}`
          : trimmedName;
        onCreate(relativePath, content);
      }}
    >
      <h2>Create note</h2>
      <FolderPicker
        label="Folder"
        folderPaths={folderPaths}
        value={folder}
        onChange={(next) => {
          setFolder(next);
          persist({ folder: next, name, content });
        }}
      />
      <div className="field">
        <label className="field-label" htmlFor="create-note-name">
          Note name
        </label>
        <input
          id="create-note-name"
          className="field-input"
          name="name"
          aria-label="Note name"
          value={name}
          onChange={(event) => {
            setName(event.target.value);
            persist({ folder, name: event.target.value, content });
          }}
          placeholder="My Note"
        />
      </div>
      {/* Nothing previously said what you were about to make or where. This
          also surfaces the numeric folder prefixes at the moment they matter. */}
      <p className="field-path">
        {folder.trim() ? `${folder.trim()} / ` : ""}
        {name.trim() || "Untitled"}.md
      </p>
      <div className="field">
        <label className="field-label" htmlFor="create-note-content">
          Content
        </label>
        <textarea
          id="create-note-content"
          className="field-input"
          name="content"
          aria-label="Markdown content"
          dir="auto"
          value={content}
          onChange={(event) => {
            setContent(event.target.value);
            persist({ folder, name, content: event.target.value });
          }}
        />
      </div>
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
      <div className="field">
        <label className="field-label" htmlFor="rename-note-title">
          New title
        </label>
        <input
          id="rename-note-title"
          className="field-input"
          name="newTitle"
          aria-label="New title"
        />
      </div>
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
  const [targetFolder, setTargetFolder] = useState("");
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        onMove(targetFolder);
      }}
    >
      <h2>Move note</h2>
      <FolderPicker
        label="Target folder"
        folderPaths={folderPaths}
        value={targetFolder}
        onChange={setTargetFolder}
      />
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

function ArchiveForm({
  error,
  onClose,
  onArchive,
}: {
  error: string | null;
  onClose: () => void;
  onArchive: () => void;
}) {
  return (
    <div>
      <h2>Archive note</h2>
      <p>This moves the note to Hatchdoor's configured archive folder.</p>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <div className="modal-actions">
        <UiButton onClick={onArchive}>Archive</UiButton>
        <UiButton className="close-note" onClick={onClose}>
          Cancel
        </UiButton>
      </div>
    </div>
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
      <p>
        This moves the note to Hatchdoor trash using the current content hash.
      </p>
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

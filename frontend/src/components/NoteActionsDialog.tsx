import { useEffect, useRef, useState } from "react";

import { UiButton } from "./ui";
import type { VaultId } from "../types";
import {
  clearCreateDraft,
  loadCreateDraft,
  saveCreateDraft,
} from "../lib/writeDrafts";

/** The Vaults a note can be created in, in the order the picker lists them. */
export type DialogVault = { vaultId: VaultId; name: string };

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
  vaults = [],
  folderPathsByVault = {},
  initialVaultId,
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
  vaults?: DialogVault[];
  /** Each Vault's own folders. A Vault whose tree has not loaded is absent
   * rather than empty-listed; both offer the root and a new folder. */
  folderPathsByVault?: Record<VaultId, string[]>;
  /** Create: the Vault to start on — the one the writer clicked in, or the one
   * holding the open note. Move: the Vault the note already lives in, which is
   * the only one it can move within. */
  initialVaultId?: VaultId;
  initialFolder?: string;
  onClose: () => void;
  onCreate: (vaultId: VaultId, relativePath: string) => void;
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
            vaults={vaults}
            folderPathsByVault={folderPathsByVault}
            initialVaultId={initialVaultId}
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
            folderPaths={
              initialVaultId ? (folderPathsByVault[initialVaultId] ?? []) : []
            }
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
  vaults,
  folderPathsByVault,
  initialVaultId,
  initialFolder,
  onClose,
  onCreate,
}: {
  error: string | null;
  vaults: DialogVault[];
  folderPathsByVault: Record<VaultId, string[]>;
  initialVaultId?: VaultId;
  initialFolder: string;
  onClose: () => void;
  onCreate: (vaultId: VaultId, relativePath: string) => void;
}) {
  // Restore any draft persisted from a previous session (e.g. a half-typed
  // name wiped by a service-worker autoUpdate reload). Fall back to props.
  // A drafted Vault that has since been disabled is dropped rather than
  // preselected, since it can no longer be written to.
  const [draft] = useState(() => loadCreateDraft());
  const draftVaultId =
    draft?.vaultId && vaults.some((vault) => vault.vaultId === draft.vaultId)
      ? draft.vaultId
      : undefined;
  const [vaultId, setVaultId] = useState<VaultId | "">(
    draftVaultId ?? initialVaultId ?? vaults[0]?.vaultId ?? "",
  );
  const [folder, setFolder] = useState(draft?.folder ?? initialFolder);
  const [name, setName] = useState(draft?.name ?? "");

  const vaultName = vaults.find((vault) => vault.vaultId === vaultId)?.name;
  const folderPaths = vaultId ? (folderPathsByVault[vaultId] ?? []) : [];

  const persist = (next: {
    vaultId: VaultId | "";
    folder: string;
    name: string;
  }) => {
    if (!next.folder && !next.name) {
      clearCreateDraft();
      return;
    }
    saveCreateDraft({
      vaultId: next.vaultId || undefined,
      folder: next.folder,
      name: next.name,
      savedAt: Date.now(),
    });
  };

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        if (!vaultId) {
          return;
        }
        const trimmedFolder = folder.trim();
        const trimmedName = name.trim();
        const relativePath = trimmedFolder
          ? `${trimmedFolder}/${trimmedName}`
          : trimmedName;
        onCreate(vaultId, relativePath);
      }}
    >
      <h2>Create note</h2>
      {/* One Vault means no choice to make, so the field is absent rather than
          a select with a single option. */}
      {vaults.length > 1 ? (
        <div className="field">
          <label className="field-label" htmlFor="create-note-vault">
            Vault
          </label>
          <select
            id="create-note-vault"
            className="field-input"
            aria-label="Vault"
            value={vaultId}
            onChange={(event) => {
              const nextVaultId = event.target.value;
              setVaultId(nextVaultId);
              // Folders belong to one Vault. Carrying the chosen path across
              // would silently create a folder in the new Vault that only
              // looked like the one that was picked.
              setFolder("");
              persist({ vaultId: nextVaultId, folder: "", name });
            }}
          >
            {vaults.map((vault) => (
              <option key={vault.vaultId} value={vault.vaultId}>
                {vault.name}
              </option>
            ))}
          </select>
        </div>
      ) : null}
      <FolderPicker
        // Remounts with the Vault so the picker's own "new folder" state does
        // not outlive the folder list it was opened against.
        key={vaultId}
        label="Folder"
        folderPaths={folderPaths}
        value={folder}
        onChange={(next) => {
          setFolder(next);
          persist({ vaultId, folder: next, name });
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
            persist({ vaultId, folder, name: event.target.value });
          }}
          placeholder="My Note"
        />
      </div>
      {/* Nothing previously said what you were about to make or where. This
          also surfaces the numeric folder prefixes at the moment they matter,
          and names the Vault even when there is only one field above it. */}
      <p className="field-path">
        {vaultName ? `${vaultName} / ` : ""}
        {folder.trim() ? `${folder.trim()} / ` : ""}
        {name.trim() || "Untitled"}.md
      </p>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <div className="modal-actions">
        <UiButton type="submit">Create and open</UiButton>
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

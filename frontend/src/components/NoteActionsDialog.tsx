import { UiButton } from "./ui";

export type NoteActionDialogKind = "create" | "rename" | "move" | "delete";

export function NoteActionsDialog({
  kind,
  error,
  onClose,
  onCreate,
  onRename,
  onMove,
  onDelete,
}: {
  kind: NoteActionDialogKind;
  error: string | null;
  onClose: () => void;
  onCreate: (relativePath: string, content: string) => void;
  onRename: (newTitle: string) => void;
  onMove: (targetFolder: string) => void;
  onDelete: () => void;
}) {
  return (
    <div className="modal-backdrop" role="presentation">
      <section className="modal-panel" role="dialog" aria-modal="true">
        {kind === "create" ? (
          <CreateForm error={error} onClose={onClose} onCreate={onCreate} />
        ) : null}
        {kind === "rename" ? (
          <RenameForm error={error} onClose={onClose} onRename={onRename} />
        ) : null}
        {kind === "move" ? (
          <MoveForm error={error} onClose={onClose} onMove={onMove} />
        ) : null}
        {kind === "delete" ? (
          <DeleteForm error={error} onClose={onClose} onDelete={onDelete} />
        ) : null}
      </section>
    </div>
  );
}

function CreateForm({
  error,
  onClose,
  onCreate,
}: {
  error: string | null;
  onClose: () => void;
  onCreate: (relativePath: string, content: string) => void;
}) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        onCreate(
          String(data.get("relativePath") ?? ""),
          String(data.get("content") ?? ""),
        );
      }}
    >
      <h2>Create note</h2>
      <label>
        Note path
        <input name="relativePath" aria-label="Note path" />
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
  onClose,
  onMove,
}: {
  error: string | null;
  onClose: () => void;
  onMove: (targetFolder: string) => void;
}) {
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
        <input name="targetFolder" aria-label="Target folder" />
      </label>
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

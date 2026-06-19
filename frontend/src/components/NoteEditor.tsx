import {
  useId,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import { UiButton } from "./ui";

type NoteEditorProps = {
  content: string;
  saving: boolean;
  error: string | null;
  notice?: string | null;
  canReload?: boolean;
  onChange: (nextContent: string) => void;
  onSave: () => void | Promise<void>;
  onCancel: () => void;
  onReload?: () => void | Promise<void>;
  renderPreview?: (content: string) => ReactNode;
};

export function NoteEditor({
  content,
  saving,
  error,
  notice,
  canReload,
  onChange,
  onSave,
  onCancel,
  onReload,
  renderPreview,
}: NoteEditorProps) {
  const textareaId = useId();
  const [preview, setPreview] = useState(false);

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void onSave();
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLFormElement>) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      if (!saving) {
        void onSave();
      }
    }
  };

  return (
    <form className="note-editor" onSubmit={handleSubmit} onKeyDown={handleKeyDown}>
      <div className="note-editor-header">
        <label htmlFor={textareaId}>Markdown content</label>
        {renderPreview ? (
          <div className="note-editor-modes" role="tablist" aria-label="Editor mode">
            <UiButton
              type="button"
              className="close-note"
              role="tab"
              aria-selected={!preview}
              onClick={() => setPreview(false)}
            >
              Write
            </UiButton>
            <UiButton
              type="button"
              className="close-note"
              role="tab"
              aria-selected={preview}
              onClick={() => setPreview(true)}
            >
              Preview
            </UiButton>
          </div>
        ) : null}
      </div>
      {error ? (
        <p className="note-editor-error" role="alert">
          {error}
        </p>
      ) : null}
      {notice ? (
        <p className="note-editor-notice" role="status">
          {notice}
        </p>
      ) : null}
      {/* TODO(attachments): image paste/drag-drop needs a backend upload route
          (POST /api/attachment, multipart → vault, path-safety mirroring the MCP
          import_attachment tool). No HTTP endpoint exists yet; deferred. */}
      {preview && renderPreview ? (
        renderPreview(content)
      ) : (
        <textarea
          id={textareaId}
          className="note-editor-textarea"
          value={content}
          onChange={(event) => onChange(event.target.value)}
          disabled={saving}
        />
      )}
      <div className="note-editor-toolbar">
        <UiButton className="close-note" type="submit" disabled={saving}>
          {saving ? "Saving..." : "Save"}
        </UiButton>
        {canReload && onReload ? (
          <UiButton
            className="close-note"
            type="button"
            onClick={() => void onReload()}
            disabled={saving}
          >
            Reload latest
          </UiButton>
        ) : null}
        <UiButton
          className="close-note"
          type="button"
          onClick={onCancel}
          disabled={saving}
        >
          Cancel
        </UiButton>
        <span className="note-editor-hint" aria-hidden="true">
          ⌘S to save
        </span>
      </div>
    </form>
  );
}

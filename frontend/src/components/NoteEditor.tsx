import { useId, type FormEvent } from "react";

import { UiButton } from "./ui";

type NoteEditorProps = {
  content: string;
  saving: boolean;
  error: string | null;
  onChange: (nextContent: string) => void;
  onSave: () => void | Promise<void>;
  onCancel: () => void;
};

export function NoteEditor({
  content,
  saving,
  error,
  onChange,
  onSave,
  onCancel,
}: NoteEditorProps) {
  const textareaId = useId();

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void onSave();
  };

  return (
    <form className="note-editor" onSubmit={handleSubmit}>
      <label htmlFor={textareaId}>Markdown content</label>
      {error ? (
        <p className="note-editor-error" role="alert">
          {error}
        </p>
      ) : null}
      <textarea
        id={textareaId}
        className="note-editor-textarea"
        value={content}
        onChange={(event) => onChange(event.target.value)}
        disabled={saving}
      />
      <div className="note-editor-toolbar">
        <UiButton className="close-note" type="submit" disabled={saving}>
          {saving ? "Saving..." : "Save"}
        </UiButton>
        <UiButton
          className="close-note"
          type="button"
          onClick={onCancel}
          disabled={saving}
        >
          Cancel
        </UiButton>
      </div>
    </form>
  );
}

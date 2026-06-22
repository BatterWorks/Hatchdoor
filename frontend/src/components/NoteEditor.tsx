import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import type { ExplorerNote } from "../types";
import {
  applyWikilinkSelection,
  getWikilinkTrigger,
  matchNoteCandidates,
  type WikilinkTrigger,
} from "./note-page/autocomplete";
import { diffConflictLines } from "./note-page/conflictDiff";
import {
  buildContentWithFrontmatter,
  parseFrontmatterEntries,
  splitFrontmatter,
  type FrontmatterEntry,
} from "./note-page/frontmatter";
import { UiButton } from "./ui";

type NoteEditorProps = {
  content: string;
  saving: boolean;
  error: string | null;
  notice?: string | null;
  canReload?: boolean;
  noteCandidates?: ExplorerNote[];
  conflictReview?: {
    diskContent: string;
    draftContent: string;
    onUseDisk: () => void;
    onKeepDraft: () => void;
  } | null;
  onUploadAttachment?: (file: File) => Promise<string>;
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
  noteCandidates = [],
  conflictReview,
  onUploadAttachment,
  onChange,
  onSave,
  onCancel,
  onReload,
  renderPreview,
}: NoteEditorProps) {
  const textareaId = useId();
  const listboxId = useId();
  const [preview, setPreview] = useState(false);
  const [trigger, setTrigger] = useState<WikilinkTrigger | null>(null);
  const [attachmentNotice, setAttachmentNotice] = useState<string | null>(null);
  const [acIndex, setAcIndex] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const pendingCaretRef = useRef<number | null>(null);
  const lastQueryRef = useRef<string | null>(null);
  const frontmatter = useMemo(() => splitFrontmatter(content), [content]);
  const frontmatterFields = useMemo(
    () =>
      frontmatter.raw === null
        ? { editable: false, entries: [] as FrontmatterEntry[] }
        : parseFrontmatterEntries(frontmatter.raw),
    [frontmatter.raw],
  );
  const showFrontmatterFields =
    frontmatter.raw !== null &&
    frontmatterFields.editable &&
    frontmatterFields.entries.length > 0;
  const editorBody = showFrontmatterFields ? frontmatter.body : content;

  const acItems =
    trigger && noteCandidates.length > 0
      ? matchNoteCandidates(noteCandidates, trigger.query)
      : [];
  const showAutocomplete = !preview && trigger !== null && acItems.length > 0;

  // Restore the caret after a controlled update from selecting a suggestion.
  useEffect(() => {
    if (pendingCaretRef.current !== null && textareaRef.current) {
      const caret = pendingCaretRef.current;
      const textarea = textareaRef.current;
      textarea.focus();
      textarea.setSelectionRange(caret, caret);
      pendingCaretRef.current = null;
    }
  }, [content]);

  const syncTrigger = (value: string, caret: number) => {
    const next = getWikilinkTrigger(value, caret);
    setTrigger(next);
    const nextQuery = next?.query ?? null;
    if (nextQuery !== lastQueryRef.current) {
      setAcIndex(0);
      lastQueryRef.current = nextQuery;
    }
  };

  const selectCandidate = (note: ExplorerNote) => {
    const textarea = textareaRef.current;
    if (!textarea || !trigger) {
      return;
    }
    const result = applyWikilinkSelection(
      textarea.value,
      textarea.selectionStart,
      trigger.start,
      note.title,
    );
    pendingCaretRef.current = result.caret;
    updateContentBody(result.text);
    setTrigger(null);
    lastQueryRef.current = null;
  };

  const updateContentBody = (nextBody: string) => {
    if (showFrontmatterFields) {
      onChange(
        buildContentWithFrontmatter(frontmatterFields.entries, nextBody),
      );
    } else {
      onChange(nextBody);
    }
  };

  const updateFrontmatterField = (id: string, value: string) => {
    const nextEntries = frontmatterFields.entries.map((entry) =>
      entry.id === id ? { ...entry, value } : entry,
    );
    onChange(buildContentWithFrontmatter(nextEntries, frontmatter.body));
  };

  const insertAttachmentAtCaret = (
    relativePath: string,
    textarea: HTMLTextAreaElement,
  ) => {
    const embed = `![[${relativePath}]]`;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const nextBody =
      textarea.value.slice(0, start) + embed + textarea.value.slice(end);
    pendingCaretRef.current = start + embed.length;
    updateContentBody(nextBody);
    setAttachmentNotice(`Inserted attachment: ${relativePath}`);
  };

  const uploadEditorFile = async (
    file: File,
    textarea: HTMLTextAreaElement,
  ) => {
    if (!onUploadAttachment) {
      return;
    }
    setAttachmentNotice("Uploading attachment...");
    try {
      const relativePath = await onUploadAttachment(file);
      insertAttachmentAtCaret(relativePath, textarea);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Upload failed.";
      setAttachmentNotice(message);
    }
  };

  const firstImageFile = (files: FileList | File[]) =>
    Array.from(files).find((file) => file.type.startsWith("image/"));

  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    if (!onUploadAttachment) {
      return;
    }
    const file = firstImageFile(event.clipboardData.files);
    if (!file) {
      return;
    }
    event.preventDefault();
    void uploadEditorFile(file, event.currentTarget);
  };

  const handleDrop = (event: DragEvent<HTMLTextAreaElement>) => {
    if (!onUploadAttachment) {
      return;
    }
    const file = firstImageFile(event.dataTransfer.files);
    if (!file) {
      return;
    }
    event.preventDefault();
    void uploadEditorFile(file, event.currentTarget);
  };

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void onSave();
  };

  const handleFormKeyDown = (event: KeyboardEvent<HTMLFormElement>) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      if (!saving) {
        void onSave();
      }
    }
  };

  const handleTextareaKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (!showAutocomplete) {
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setAcIndex((i) => (i + 1) % acItems.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setAcIndex((i) => (i - 1 + acItems.length) % acItems.length);
    } else if (event.key === "Enter" || event.key === "Tab") {
      event.preventDefault();
      selectCandidate(acItems[Math.min(acIndex, acItems.length - 1)]);
    } else if (event.key === "Escape") {
      event.preventDefault();
      setTrigger(null);
    }
  };

  return (
    <form
      className="note-editor"
      onSubmit={handleSubmit}
      onKeyDown={handleFormKeyDown}
    >
      <div className="note-editor-header">
        <label htmlFor={textareaId}>Markdown content</label>
        {renderPreview ? (
          <div
            className="note-editor-modes"
            role="tablist"
            aria-label="Editor mode"
          >
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
      {attachmentNotice ? (
        <p className="note-editor-notice" role="status">
          {attachmentNotice}
        </p>
      ) : null}
      {conflictReview ? (
        <section className="note-editor-conflict" aria-label="Conflict review">
          <div className="note-editor-conflict-header">
            <h3>Conflict review</h3>
            <div className="note-editor-conflict-actions">
              <UiButton
                className="close-note"
                type="button"
                onClick={conflictReview.onUseDisk}
                disabled={saving}
              >
                Use disk version
              </UiButton>
              <UiButton
                className="close-note"
                type="button"
                onClick={conflictReview.onKeepDraft}
                disabled={saving}
              >
                Keep my draft
              </UiButton>
            </div>
          </div>
          <div className="note-editor-conflict-diff">
            {diffConflictLines(
              conflictReview.diskContent,
              conflictReview.draftContent,
            ).map((line, index) => (
              <div
                // Diff rows can repeat, so the index is the stable row identity.
                key={index}
                className={`note-editor-conflict-line ${line.kind}`}
              >
                <span className="note-editor-conflict-marker">
                  {line.kind === "same"
                    ? " "
                    : line.kind === "disk"
                      ? "disk"
                      : "draft"}
                </span>
                <code>{line.text}</code>
              </div>
            ))}
          </div>
        </section>
      ) : null}
      {preview && renderPreview ? (
        renderPreview(content)
      ) : (
        <div className="note-editor-input">
          {showFrontmatterFields ? (
            <div
              className="note-editor-properties"
              aria-label="Frontmatter properties"
            >
              {frontmatterFields.entries.map((entry) => (
                <label className="note-editor-property" key={entry.id}>
                  <span>{entry.key}</span>
                  <input
                    type="text"
                    value={entry.value}
                    aria-label={`Property ${entry.key}`}
                    disabled={saving}
                    onChange={(event) =>
                      updateFrontmatterField(entry.id, event.target.value)
                    }
                  />
                </label>
              ))}
            </div>
          ) : null}
          <textarea
            id={textareaId}
            ref={textareaRef}
            className="note-editor-textarea"
            value={editorBody}
            disabled={saving}
            aria-expanded={showAutocomplete}
            aria-controls={listboxId}
            aria-autocomplete="list"
            onChange={(event) => {
              updateContentBody(event.target.value);
              syncTrigger(event.target.value, event.target.selectionStart);
            }}
            onClick={(event) =>
              syncTrigger(
                event.currentTarget.value,
                event.currentTarget.selectionStart,
              )
            }
            onPaste={handlePaste}
            onDragOver={(event) => {
              if (onUploadAttachment) {
                event.preventDefault();
              }
            }}
            onDrop={handleDrop}
            onKeyDown={handleTextareaKeyDown}
          />
          {showAutocomplete ? (
            <ul
              id={listboxId}
              className="note-editor-autocomplete"
              role="listbox"
              aria-label="Link suggestions"
            >
              {acItems.map((note, index) => (
                <li key={note.slug} role="presentation">
                  <button
                    type="button"
                    role="option"
                    aria-selected={index === acIndex}
                    className={
                      index === acIndex
                        ? "note-editor-autocomplete-item active"
                        : "note-editor-autocomplete-item"
                    }
                    // Keep the textarea selection while clicking.
                    onMouseDown={(event) => {
                      event.preventDefault();
                      selectCandidate(note);
                    }}
                  >
                    {note.title}
                  </button>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
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
          ⌘S to save · [[ to link
        </span>
      </div>
    </form>
  );
}

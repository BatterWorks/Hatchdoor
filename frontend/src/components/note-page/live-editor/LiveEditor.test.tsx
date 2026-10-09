import { act, fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";

import { EditorView } from "@codemirror/view";

import { LiveEditor, type LiveEditorHandle } from "./LiveEditor";

function mount(
  overrides: Partial<React.ComponentProps<typeof LiveEditor>> = {},
) {
  const onChange = vi.fn();
  const onCommit = vi.fn();
  const ref = createRef<LiveEditorHandle>();
  const props = {
    value: "# Title\n\nFirst paragraph.\n",
    searchQuery: "",
    touch: false,
    noteCandidates: [
      { vault_id: "vault-1", title: "Home", slug: "home" },
      { vault_id: "vault-1", title: "Other", slug: "other" },
    ],
    formatNoteLink: (note: { title: string }) => `[[${note.title}]]`,
    onOpenNote: vi.fn(),
    resolveNote: () => ({ label: "Home", missing: false }),
    resolveImageSrc: (raw: string) => `/assets/${raw}`,
    onChange,
    onCommit,
    ...overrides,
  };
  const utils = render(<LiveEditor ref={ref} {...props} />);
  const content = screen.getByRole("textbox", { name: "Note body" });
  const view = EditorView.findFromDOM(content as HTMLElement);
  if (!view) {
    throw new Error("no editor view mounted");
  }
  return { ...utils, ref, view, content, onChange, onCommit, props };
}

describe("LiveEditor", () => {
  it("holds the body and reports every change, then commits on blur", () => {
    const { view, content, onChange, onCommit } = mount();
    expect(view.state.doc.toString()).toBe("# Title\n\nFirst paragraph.\n");

    act(() => {
      view.dispatch({
        changes: { from: view.state.doc.length, insert: "More." },
      });
    });
    expect(onChange).toHaveBeenCalledWith("# Title\n\nFirst paragraph.\nMore.");
    expect(onCommit).not.toHaveBeenCalled();

    fireEvent.blur(content);
    expect(onCommit).toHaveBeenCalledWith("# Title\n\nFirst paragraph.\nMore.");

    // Nothing changed since: leaving again commits nothing.
    fireEvent.blur(content);
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it("takes a new value from outside without reporting it as a change", () => {
    const { view, onChange, rerender, props } = mount();
    // An expression, not an attribute string: in JSX `"\n"` is two characters.
    rerender(<LiveEditor {...props} value={"Replaced from disk.\n"} />);
    expect(view.state.doc.toString()).toBe("Replaced from disk.\n");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("marks the search query and scrolls to a hit on request", () => {
    const onSearchHits = vi.fn();
    const { view, ref } = mount({
      value: "alpha beta\nalpha again\n",
      searchQuery: "alpha",
      onSearchHits,
    });
    expect(onSearchHits).toHaveBeenLastCalledWith(2);
    act(() => ref.current?.scrollToHit(1));
    expect(
      view.state.sliceDoc(
        view.state.selection.main.from,
        view.state.selection.main.to,
      ),
    ).toBe("alpha");
    expect(view.state.selection.main.from).toBe("alpha beta\n".length);
  });

  it("raises the floating toolbar over a one-line selection on desktop", () => {
    const { view } = mount();
    expect(document.querySelector(".live-editor-toolbar")).toBeNull();
    act(() => {
      view.dispatch({ selection: { anchor: 2, head: 7 } });
    });
    expect(document.querySelector(".live-editor-toolbar")).not.toBeNull();
    expect(
      screen.getByRole("button", { name: "Bold (Ctrl+B)" }),
    ).toBeInTheDocument();
    // Across lines it is a cut or a move, not a format.
    act(() => {
      view.dispatch({ selection: { anchor: 2, head: 12 } });
    });
    expect(document.querySelector(".live-editor-toolbar")).toBeNull();
  });

  it("shows the keyboard bar on touch while focused, and no floating toolbar", () => {
    const { view, content } = mount({ touch: true });
    act(() => {
      view.dispatch({ selection: { anchor: 2, head: 7 } });
    });
    expect(document.querySelector(".live-editor-toolbar")).toBeNull();
    expect(screen.queryByRole("toolbar", { name: "Formatting" })).toBeNull();
    fireEvent.focus(content);
    expect(
      screen.getByRole("toolbar", { name: "Formatting" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Bold" })).toBeInTheDocument();
    fireEvent.blur(content);
    expect(screen.queryByRole("toolbar", { name: "Formatting" })).toBeNull();
  });

  it("inserts an uploaded attachment on its own line after the caret's", async () => {
    const onUploadAttachment = vi
      .fn()
      .mockResolvedValue({ path: "Media/a.png", embed: "![[Media/a.png]]" });
    const { view, content, onChange } = mount({ onUploadAttachment });
    const file = new File(["png"], "a.png", { type: "image/png" });
    await act(async () => {
      fireEvent.paste(content, { clipboardData: { files: [file], items: [] } });
      await Promise.resolve();
    });
    expect(onUploadAttachment).toHaveBeenCalledWith(file);
    // The caret sits on the title line, so the embed goes on the line after it.
    expect(view.state.doc.toString()).toBe(
      "# Title\n![[Media/a.png]]\n\nFirst paragraph.\n",
    );
    expect(onChange).toHaveBeenLastCalledWith(
      "# Title\n![[Media/a.png]]\n\nFirst paragraph.\n",
    );
  });

  it("refuses a file that is not an attachment, with the reason", async () => {
    const onUploadAttachment = vi.fn();
    const onUploadNotice = vi.fn();
    const { content } = mount({ onUploadAttachment, onUploadNotice });
    const file = new File(["x"], "script.exe", {
      type: "application/x-msdownload",
    });
    await act(async () => {
      fireEvent.paste(content, { clipboardData: { files: [file], items: [] } });
    });
    expect(onUploadAttachment).not.toHaveBeenCalled();
    expect(onUploadNotice).toHaveBeenCalledTimes(1);
  });
});

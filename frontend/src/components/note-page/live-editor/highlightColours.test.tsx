import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

import { EditorView } from "@codemirror/view";

import { colourTableCells } from "./highlightColours";
import { LiveEditor } from "./LiveEditor";

function mount(value: string) {
  const onChange = vi.fn();
  const onCommit = vi.fn();
  render(
    <MemoryRouter>
      <LiveEditor
        value={value}
        searchQuery=""
        touch={false}
        noteCandidates={[]}
        formatNoteLink={(note) => `[[${note.title}]]`}
        onOpenNote={vi.fn()}
        resolveNote={async () => null}
        resolveAssetSrc={(raw) => `/assets/${raw}`}
        onChange={onChange}
        onCommit={onCommit}
      />
    </MemoryRouter>,
  );
  const content = screen.getByRole("textbox", { name: "Note body" });
  const view = EditorView.findFromDOM(content as HTMLElement);
  if (!view) {
    throw new Error("no editor view mounted");
  }
  return { view, content, onChange, onCommit };
}

/** Each coloured highlight the editor draws, as `[colour, drawn text]`. */
function colouredIn(root: ParentNode): Array<[string, string]> {
  return Array.from(
    root.querySelectorAll<HTMLElement>(".live-editor-highlight"),
  ).map((element) => {
    const colour = Array.from(element.classList)
      .find((name) => /^live-editor-highlight-[a-z]+$/.test(name))
      ?.slice("live-editor-highlight-".length);
    return [colour ?? "", element.textContent ?? ""];
  });
}

describe("highlight colours in the live editor (#577)", () => {
  it.each([
    ["🔴", "red"],
    ["🟠", "orange"],
    ["🟡", "yellow"],
    ["🟢", "green"],
    ["🔵", "blue"],
    ["🟣", "purple"],
  ])("draws %s as %s with the emoji hidden", (emoji, colour) => {
    const { view } = mount(`A ==${emoji}word== here.\n`);
    expect(colouredIn(view.contentDOM)).toEqual([[colour, "word"]]);
    expect(view.contentDOM.textContent).not.toContain(emoji);
  });

  it("leaves another emoji, a plain highlight and code alone", () => {
    const { view } = mount(
      "==🟤brown== and ==plain== and `==🔴code==` here.\n",
    );
    expect(colouredIn(view.contentDOM)).toEqual([]);
    expect(view.contentDOM.textContent).toContain("🟤brown");
    expect(view.contentDOM.textContent).toContain("==🔴code==");
  });

  it("shows the emoji while the caret is on its line", async () => {
    const value = "First ==🔴red== line.\n\nOther line.\n";
    const { view } = mount(value);
    act(() => {
      view.focus();
      view.dispatch({ selection: { anchor: value.indexOf("red") } });
    });
    await waitFor(() => expect(view.hasFocus).toBe(true));
    await waitFor(() => expect(view.contentDOM.textContent).toContain("🔴"));
    expect(colouredIn(view.contentDOM)).toEqual([["red", "🔴red"]]);

    act(() => {
      view.dispatch({ selection: { anchor: value.indexOf("Other") } });
    });
    await waitFor(() =>
      expect(view.contentDOM.textContent).not.toContain("🔴"),
    );
  });

  it("keeps the note byte for byte through open, an edit elsewhere and save", () => {
    const value =
      "# Note\n\nA ==🔴red== and ==🟣purple== word.\n\n| a |\n| - |\n| ==🔵cell== |\n\nEnd.\n";
    const { view, content, onChange, onCommit } = mount(value);
    expect(view.state.doc.toString()).toBe(value);
    expect(onChange).not.toHaveBeenCalled();

    act(() => {
      view.dispatch({
        changes: { from: value.indexOf("End"), insert: "The " },
      });
    });
    const expected = value.replace("End.", "The End.");
    expect(onChange).toHaveBeenLastCalledWith(expected);

    fireEvent.blur(content);
    expect(onCommit).toHaveBeenLastCalledWith(expected);
  });
});

describe("colourTableCells", () => {
  function cell(text: string): HTMLElement {
    const root = document.createElement("table");
    root.innerHTML =
      '<tbody><tr><td data-raw="x"><div class="cm-atomic-table-cell-source" contenteditable="true">' +
      '<span class="cm-atomic-highlight-wrap"><span class="cm-atomic-mark">==</span>' +
      `<span class="cm-atomic-highlight">${text}</span>` +
      '<span class="cm-atomic-mark">==</span></span></div></td></tr></tbody>';
    document.body.append(root);
    return root;
  }

  it("colours a cell's highlight and wraps its emoji, keeping the text", () => {
    const root = cell("🟢green cell");
    colourTableCells(root);
    const highlight = root.querySelector(".cm-atomic-highlight")!;
    expect(highlight).toHaveClass("live-editor-highlight-green");
    expect(
      root.querySelector(".live-editor-highlight-marker")?.textContent,
    ).toBe("🟢");
    // The widget reads the cell back from its text.
    expect(
      root.querySelector(".cm-atomic-table-cell-source")?.textContent,
    ).toBe("==🟢green cell==");

    // A second pass changes nothing.
    colourTableCells(root);
    expect(root.querySelectorAll(".live-editor-highlight-marker")).toHaveLength(
      1,
    );
    root.remove();
  });

  it("leaves another emoji plain", () => {
    const root = cell("🟤brown");
    colourTableCells(root);
    expect(root.querySelector(".live-editor-highlight")).toBeNull();
    expect(root.querySelector(".live-editor-highlight-marker")).toBeNull();
    root.remove();
  });

  it("keeps the colour and shows the emoji while the cell holds the caret", () => {
    const root = cell("🔴red");
    const source = root.querySelector<HTMLElement>(
      ".cm-atomic-table-cell-source",
    )!;
    source.focus();
    colourTableCells(root);
    expect(root.querySelector(".cm-atomic-highlight")).toHaveClass(
      "live-editor-highlight-red",
    );
    expect(root.querySelector(".live-editor-highlight-marker")).toBeNull();
    expect(source.textContent).toBe("==🔴red==");

    source.blur();
    colourTableCells(root);
    expect(
      root.querySelector(".live-editor-highlight-marker")?.textContent,
    ).toBe("🔴");
    expect(source.textContent).toBe("==🔴red==");
    root.remove();
  });
});

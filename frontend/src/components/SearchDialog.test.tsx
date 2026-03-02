import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { SearchResult } from "../types";
import { SearchDialog } from "./SearchDialog";

function renderDialog(overrides?: Partial<ComponentProps<typeof SearchDialog>>) {
  const onClose = vi.fn();
  const onQueryChange = vi.fn();
  const onIncludeContentChange = vi.fn();
  const onSelect = vi.fn();
  const inputRef = { current: null };
  const props: ComponentProps<typeof SearchDialog> = {
    query: "plan",
    includeContent: false,
    loading: false,
    error: null,
    results: [],
    inputRef,
    onClose,
    onQueryChange,
    onIncludeContentChange,
    onSelect,
    ...overrides,
  };
  const view = render(<SearchDialog {...props} />);
  return { ...view, props };
}

describe("SearchDialog", () => {
  afterEach(() => {
    cleanup();
  });

  it("closes on overlay click and Escape key", () => {
    const { props, container } = renderDialog();
    const overlay = container.querySelector(".search-overlay");
    expect(overlay).toBeTruthy();

    fireEvent.click(overlay!);
    fireEvent.keyDown(overlay!, { key: "Escape" });
    expect(props.onClose).toHaveBeenCalledTimes(2);
  });

  it("shows empty-state text when query is long enough with no results", () => {
    const { getAllByText } = renderDialog({ query: "home", results: [] });
    expect(getAllByText("No matching notes.")).toHaveLength(1);
  });

  it("renders loading and error states", () => {
    renderDialog({ loading: true, error: "boom" });
    expect(screen.getByText("Searching…")).toBeInTheDocument();
    expect(screen.getByText("boom")).toBeInTheDocument();
  });

  it("highlights literal query matches and emits selection/toggle events", () => {
    const results: SearchResult[] = [
      {
        title: "a.b",
        slug: "ab",
        relative_path: "Notes/a.b",
        match_kind: "title",
        snippet: "line a.b",
      },
    ];
    const { props, getByRole } = renderDialog({ query: ".", results });

    expect(screen.getAllByText(".").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: /a.b/ }));
    expect(props.onSelect).toHaveBeenCalledWith({
      slug: "ab",
      query: ".",
      matchKind: "title",
    });

    fireEvent.click(getByRole("checkbox"));
    expect(props.onIncludeContentChange).toHaveBeenCalledWith(true);
  });
});

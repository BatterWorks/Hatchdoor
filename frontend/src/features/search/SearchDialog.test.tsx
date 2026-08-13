import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SearchDialog, type SearchResult } from ".";
import { THREE_VAULTS } from "../../test/fixtures/vaults";

function renderDialog(
  overrides?: Partial<ComponentProps<typeof SearchDialog>>,
) {
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
    vaults: [],
    scope: "all",
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
        vault_id: "vault-1",
        chunk_id: 1,
        note_slug: "ab",
        note_title: "a.b",
        note_path: "Notes/a.b",
        heading_path: null,
        content: "line a.b",
        score: 0.9,
        layer: null,
        outbound_links: [],
      },
    ];
    const { props, getByRole } = renderDialog({ query: ".", results });

    expect(screen.getAllByText(".").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: /a.b/ }));
    expect(props.onSelect).toHaveBeenCalledWith({
      vaultId: "vault-1",
      slug: "ab",
      query: ".",
      matchKind: "",
    });

    fireEvent.click(getByRole("checkbox"));
    expect(props.onIncludeContentChange).toHaveBeenCalledWith(true);
  });
});

function resultFor(
  vaultId: string,
  overrides: Partial<SearchResult> = {},
): SearchResult {
  return {
    vault_id: vaultId,
    chunk_id: 1,
    note_slug: "plan",
    note_title: "Plan",
    note_path: "Projects/Plan",
    heading_path: null,
    content: "Plan body text",
    score: 0.9,
    layer: null,
    outbound_links: [],
    ...overrides,
  };
}

describe("SearchDialog Vault provenance (#140)", () => {
  afterEach(cleanup);

  it("shows the Vault prefix when scope is all and more than one Vault is enabled", () => {
    renderDialog({
      results: [resultFor(THREE_VAULTS[1].vault_id)],
      vaults: THREE_VAULTS,
      scope: "all",
    });

    expect(screen.getByText("Beta")).toBeInTheDocument();
  });

  it("hides the Vault prefix once scope is narrowed to one Vault", () => {
    renderDialog({
      results: [resultFor(THREE_VAULTS[1].vault_id)],
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
    });

    expect(screen.queryByText("Beta")).not.toBeInTheDocument();
  });

  it("hides the Vault prefix at one enabled Vault", () => {
    renderDialog({
      results: [resultFor(THREE_VAULTS[0].vault_id)],
      vaults: [THREE_VAULTS[0]],
      scope: "all",
    });

    expect(screen.queryByText("Alpha")).not.toBeInTheDocument();
  });

  it("renders the prefix as inert markup, not a nested click target", () => {
    renderDialog({
      results: [resultFor(THREE_VAULTS[1].vault_id)],
      vaults: THREE_VAULTS,
      scope: "all",
    });

    const prefix = screen.getByText("Beta").closest(".vault-prefix");
    expect(prefix?.tagName).toBe("SPAN");
    expect(prefix?.closest("button")).not.toBeNull();
    expect(prefix?.querySelector("button, a")).toBeNull();
  });

  it("keeps the path in its own eliding span, separate from the prefix", () => {
    const { container } = renderDialog({
      results: [resultFor(THREE_VAULTS[1].vault_id)],
      vaults: THREE_VAULTS,
      scope: "all",
    });

    const pathText = container.querySelector(".result-path-text");
    expect(pathText).toBeInTheDocument();
    expect(pathText?.textContent).toBe("Projects/Plan.md");
    expect(pathText?.closest(".vault-prefix")).toBeNull();
  });
});

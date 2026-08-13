import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SearchDialog, type SearchResult } from ".";
import { EIGHT_VAULTS, THREE_VAULTS } from "../../test/fixtures/vaults";

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
    partial: false,
    missingVaultNames: [],
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

describe("SearchDialog tells the truth about a partial read (#141)", () => {
  afterEach(cleanup);

  it("names only the missing Vaults in a trailing warn-ink line, at three Vaults, without changing ranking", () => {
    const missing = [THREE_VAULTS[2].name];
    const { container } = renderDialog({
      results: [resultFor(THREE_VAULTS[0].vault_id)],
      vaults: THREE_VAULTS,
      partial: true,
      missingVaultNames: missing,
    });

    const line = screen.getByText(`${missing[0]} did not answer.`);
    expect(line).toHaveClass("search-partial");
    // The result itself is still there, in the API's own ranking.
    expect(container.querySelector(".search-result--primary")).not.toBeNull();
  });

  it("names every missing Vault in a trailing line, at eight Vaults", () => {
    const missing = EIGHT_VAULTS.slice(6).map((vault) => vault.name);
    renderDialog({
      results: [resultFor(EIGHT_VAULTS[0].vault_id)],
      vaults: EIGHT_VAULTS,
      partial: true,
      missingVaultNames: missing,
    });

    expect(
      screen.getByText(`${missing[0]} and ${missing[1]} did not answer.`),
    ).toBeInTheDocument();
  });

  it("replaces 'No matching notes' with the documented error block when nothing is usable", () => {
    const missing = [THREE_VAULTS[0].name, THREE_VAULTS[1].name];
    const { container } = renderDialog({
      results: [],
      partial: true,
      missingVaultNames: missing,
    });

    expect(screen.queryByText("No matching notes.")).not.toBeInTheDocument();
    expect(container.querySelector(".state-block.error")).not.toBeNull();
    expect(screen.getByText("Nothing Found")).toBeInTheDocument();
    expect(
      screen.getByText(`${missing[0]} and ${missing[1]} did not answer.`),
    ).toBeInTheDocument();
  });

  it("shows the plain 'No matching notes' state, not the error block, when the read is not partial", () => {
    const { container } = renderDialog({ results: [], partial: false });

    expect(container.querySelector(".state-block.error")).toBeNull();
    expect(screen.getByText("No matching notes.")).toBeInTheDocument();
  });
});

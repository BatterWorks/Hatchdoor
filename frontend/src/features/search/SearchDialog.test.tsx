import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SearchDialog, type SearchResult } from ".";
import {
  EIGHT_VAULTS,
  participantFor,
  THREE_VAULTS,
} from "../../test/fixtures/vaults";

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
    participants: [],
    initialVaultFilter: undefined,
    vaults: [],
    scope: "all",
    inputRef,
    startupStatus: null,
    onRetryModelSetup: vi.fn(),
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

    const prefix = screen.getByText("Beta").closest(".path-vault");
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
    expect(pathText?.closest(".path-vault")).toBeNull();
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

const [ALPHA, BETA, GAMMA] = THREE_VAULTS;

/** Alpha contributed two results, Beta answered fresh with none, Gamma did
 * not answer at all — the three facet states #144 must render. */
const FACET_RESULTS: SearchResult[] = [
  resultFor(ALPHA.vault_id, { note_slug: "one", note_title: "One" }),
  resultFor(ALPHA.vault_id, { note_slug: "two", note_title: "Two" }),
];
const FACET_PARTICIPANTS = [
  participantFor(ALPHA, "fresh"),
  participantFor(BETA, "fresh"),
  participantFor(GAMMA, "unavailable"),
];

describe("SearchDialog's own Vault filter — never the browsing scope (#144)", () => {
  afterEach(cleanup);

  it("never exposes a way to change the browsing scope — the click handlers touch only local state", () => {
    const { props } = renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });

    fireEvent.click(screen.getByRole("button", { name: /^Alpha/ }));
    fireEvent.click(screen.getByRole("button", { name: /^Beta/ }));
    fireEvent.click(screen.getByRole("button", { name: /All results/ }));

    // The component receives no scope-changing prop at all; picking a facet
    // can only have touched the callbacks it was actually given, and none
    // of those is one.
    expect(props.onQueryChange).not.toHaveBeenCalled();
    expect(props.onIncludeContentChange).not.toHaveBeenCalled();
    expect(props.onClose).not.toHaveBeenCalled();
    expect(props.onSelect).not.toHaveBeenCalled();
  });

  it("lists every reached Vault in Vault-management order, with All results permanent and first", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });

    const rail = document.querySelector(".search-facet-rail");
    const labels = Array.from(
      rail?.querySelectorAll(".search-facet-label") ?? [],
    ).map((el) => el.textContent);
    expect(labels).toEqual(["All results", "Alpha", "Beta", "Gamma"]);
  });

  it("shows the contributed count for All results and for a Vault with matches", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });

    const all = screen.getByRole("button", { name: /All results/ });
    expect(all).toHaveClass("is-selected");
    expect(all.querySelector(".side-count")).toHaveTextContent("2");

    const alpha = screen.getByRole("button", { name: /^Alpha/ });
    expect(alpha.querySelector(".side-count")).toHaveTextContent("2");
  });

  it("shows 0, inert, for a Vault that answered with nothing — and it stays selectable", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });

    const beta = screen.getByRole("button", { name: /^Beta/ });
    expect(beta.querySelector(".side-count")).toHaveTextContent("0");
    expect(beta).not.toHaveAttribute("aria-disabled", "true");

    fireEvent.click(beta);
    expect(beta).toHaveClass("is-selected");
  });

  it("shows 'no answer' in error ink for a Vault that did not answer, and refuses the click", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });

    const gamma = screen.getByRole("button", { name: /^Gamma/ });
    expect(gamma).toHaveAttribute("aria-disabled", "true");
    const word = gamma.querySelector(".vault-slot-condition");
    expect(word).toHaveTextContent("no answer");
    expect(word).toHaveClass("vault-tier-error");

    fireEvent.click(gamma);
    expect(gamma).not.toHaveClass("is-selected");
  });

  it("filters the visible results without changing their order or re-fetching", () => {
    renderDialog({
      results: [
        ...FACET_RESULTS,
        resultFor(GAMMA.vault_id, { note_slug: "three", note_title: "Three" }),
      ],
      participants: [
        participantFor(ALPHA, "fresh"),
        participantFor(BETA, "fresh"),
        participantFor(GAMMA, "fresh"),
      ],
      vaults: THREE_VAULTS,
      scope: "all",
    });

    const titlesBefore = screen
      .getAllByText(/^(One|Two|Three)$/)
      .map((el) => el.textContent);
    expect(titlesBefore).toEqual(["One", "Two", "Three"]);

    fireEvent.click(screen.getByRole("button", { name: /^Alpha/ }));

    expect(screen.getByText("One")).toBeInTheDocument();
    expect(screen.getByText("Two")).toBeInTheDocument();
    expect(screen.queryByText("Three")).not.toBeInTheDocument();
  });

  it("shows a plain message, not the partial error block, when the filter narrows to nothing", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });

    fireEvent.click(screen.getByRole("button", { name: /^Beta/ }));

    expect(screen.getByText("No results in Beta.")).toBeInTheDocument();
    expect(document.querySelector(".state-block.error")).toBeNull();
    expect(screen.queryByText("One")).not.toBeInTheDocument();
  });

  it("is absent when scope is narrowed — the Scope zone is already on screen", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: ALPHA.vault_id,
    });

    expect(document.querySelector(".search-facet-rail")).toBeNull();
  });

  it("is absent at one enabled Vault", () => {
    renderDialog({
      results: [resultFor(ALPHA.vault_id)],
      participants: [participantFor(ALPHA, "fresh")],
      vaults: [ALPHA],
      scope: "all",
    });

    expect(document.querySelector(".search-facet-rail")).toBeNull();
  });

  it("pre-selects the facet a tag tap named, via initialVaultFilter", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
      initialVaultFilter: ALPHA.vault_id,
    });

    expect(screen.getByRole("button", { name: /^Alpha/ })).toHaveClass(
      "is-selected",
    );
    expect(screen.queryByText("Two")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /All results/ })).not.toHaveClass(
      "is-selected",
    );
  });

  it("starts fresh at All results on a later, unrelated open — the filter does not survive remounting", () => {
    const first = renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });
    fireEvent.click(screen.getByRole("button", { name: /^Alpha/ }));
    expect(screen.getByRole("button", { name: /^Alpha/ })).toHaveClass(
      "is-selected",
    );
    first.unmount();

    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });

    expect(screen.getByRole("button", { name: /All results/ })).toHaveClass(
      "is-selected",
    );
  });
});

describe("SearchDialog's mobile field strip (#144)", () => {
  afterEach(cleanup);

  it("offers Scope as a native select, sharing state with the desktop rail", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });

    const scopeSelect = screen.getByLabelText("Scope") as HTMLSelectElement;
    fireEvent.change(scopeSelect, { target: { value: ALPHA.vault_id } });

    expect(screen.getByRole("button", { name: /^Alpha/ })).toHaveClass(
      "is-selected",
    );
  });

  it("disables the option for a Vault that did not answer", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: "all",
    });

    const option = screen.getByRole("option", {
      name: "Gamma (no answer)",
    }) as HTMLOptionElement;
    expect(option.disabled).toBe(true);
  });

  it("omits the Scope field at one enabled Vault, but keeps Mode", () => {
    renderDialog({
      results: [],
      participants: [],
      vaults: [ALPHA],
      scope: "all",
    });

    expect(screen.queryByLabelText("Scope")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Mode")).toBeInTheDocument();
  });

  it("keeps the Scope field even when the browsing scope is narrowed", () => {
    renderDialog({
      results: FACET_RESULTS,
      participants: FACET_PARTICIPANTS,
      vaults: THREE_VAULTS,
      scope: ALPHA.vault_id,
    });

    expect(screen.getByLabelText("Scope")).toBeInTheDocument();
  });

  it("controls the same Mode state as the desktop toggle", () => {
    const { props } = renderDialog({ includeContent: false });

    fireEvent.change(screen.getByLabelText("Mode"), {
      target: { value: "keyword" },
    });

    expect(props.onIncludeContentChange).toHaveBeenCalledExactlyOnceWith(true);
  });
});

describe("SearchDialog surfaces the shrunk startup gate's state (#150)", () => {
  afterEach(cleanup);

  it("shows a work-in-flight block during a first index, with the current percentage", () => {
    renderDialog({
      startupStatus: { state: "indexing", percent: 42 } as never,
    });

    expect(screen.getByText("Could Not Load")).toBeVisible();
    expect(screen.getByText(/42%/)).toBeVisible();
  });

  it("shows a work-in-flight block during scanning with no percentage yet", () => {
    renderDialog({ startupStatus: { state: "scanning" } });

    expect(screen.getByText("Could Not Load")).toBeVisible();
  });

  it("keeps the query input enabled and typable during a first index", () => {
    const { props } = renderDialog({
      startupStatus: { state: "indexing", percent: 10 } as never,
    });

    const input = screen.getByPlaceholderText("Search notes…");
    expect(input).not.toBeDisabled();
    fireEvent.change(input, { target: { value: "plan b" } });
    expect(props.onQueryChange).toHaveBeenCalledWith("plan b");
  });

  it("shows the failed-model reason and a retry action instead of the ordinary empty state", () => {
    const { props } = renderDialog({
      query: "",
      startupStatus: {
        state: "failed",
        message: "The search model could not be downloaded or loaded.",
      },
    });

    expect(screen.getByText("Could Not Load")).toBeVisible();
    expect(
      screen.getByText("The search model could not be downloaded or loaded."),
    ).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Retry setup" }));
    expect(props.onRetryModelSetup).toHaveBeenCalledTimes(1);
  });

  it("does not show a work-in-flight or failed block once the gate has stepped aside", () => {
    renderDialog({ startupStatus: { state: "ready" } });

    expect(screen.queryByText("Could Not Load")).not.toBeInTheDocument();
  });
});

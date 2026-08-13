import { useState } from "react";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ExplorerPane } from "./ExplorerPane";
import {
  EIGHT_VAULTS,
  THREE_VAULTS,
  unavailableVault,
} from "../test/fixtures/vaults";
import type {
  ExplorerFolder,
  ModifiedNote,
  RecentNote,
  VaultSummary,
  VaultTree,
} from "../types";

const VAULT_ID = "vault-1";

const TREE: ExplorerFolder = {
  name: "Vault",
  folders: [
    {
      name: "10-topics",
      folders: [],
      notes: [{ vault_id: VAULT_ID, title: "Finance", slug: "finance" }],
    },
  ],
  notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
};

const RECENT: RecentNote[] = [
  {
    vaultId: VAULT_ID,
    title: "Home",
    slug: "home",
    relativePath: "Home",
    viewedAt: 1,
  },
];

const MODIFIED: ModifiedNote[] = [
  {
    vault_id: VAULT_ID,
    title: "Finance",
    slug: "finance",
    relative_path: "10-topics/Finance",
    mtime_ns: 2,
  },
];

function defaultPaneProps(): Parameters<typeof ExplorerPane>[0] {
  return {
    explorerScrollRef: { current: null },
    drawerOpen: false,
    isMobile: false,
    writeEnabled: true,
    settingsEnabled: true,
    onCreateNoteInFolder: vi.fn(),
    locationPathname: `/v/${VAULT_ID}/n/home`,
    recentNotes: RECENT,
    modifiedNotes: MODIFIED,
    modifiedNotesPartial: false,
    modifiedNotesMissingVaults: [],
    loadingTree: false,
    treeError: null,
    tree: TREE,
    vaultTrees: [],
    expandedFolders: {},
    recentCollapsed: false,
    onRecentCollapsedChange: vi.fn(),
    onExpandedFoldersChange: vi.fn(),
    onCloseDrawer: vi.fn(),
    onRefreshTree: vi.fn(),
    onScrollTopChange: vi.fn(),
    vaults: [],
    scope: "all" as const,
    onScopeChange: vi.fn(),
    viewingVaultId: undefined,
    vaultNoteCounts: {},
    scopeZoneCollapsed: false,
    onScopeZoneCollapsedChange: vi.fn(),
  };
}

function renderPane(
  overrides: Partial<Parameters<typeof ExplorerPane>[0]> = {},
) {
  const props = { ...defaultPaneProps(), ...overrides };

  render(
    <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
      <ExplorerPane {...props} />
    </MemoryRouter>,
  );

  return props;
}

/** A vault's own tree (#142), grouped rather than merged, as `useVaultTree`
 * now exposes it. Every fixture Vault gets an identically-named top folder
 * so tests can prove per-Vault folder-open memory doesn't leak across it. */
function vaultTreeFor(vault: VaultSummary, folderName = "Journal"): VaultTree {
  return {
    vault_id: vault.vault_id,
    vault_name: vault.name,
    tree: {
      name: vault.name,
      folders: [
        {
          name: folderName,
          folders: [],
          notes: [
            {
              vault_id: vault.vault_id,
              title: `${vault.name} entry`,
              slug: `${vault.vault_id}-entry`,
            },
          ],
        },
      ],
      notes: [],
    },
  };
}

function accordionHeads() {
  return screen
    .getAllByRole("button")
    .filter((button) => button.className.includes("vault-accordion-head"));
}

/** The accordion head for a given Vault name — never the Scope zone's row of
 * the same name, which renders every enabled Vault right alongside it. */
function headFor(name: string): HTMLElement {
  const head = accordionHeads().find((button) =>
    button.querySelector(".side-label")?.textContent?.includes(name),
  );
  if (!head) {
    throw new Error(`No accordion head found for "${name}"`);
  }
  return head;
}

/** Renders `ExplorerPane` with `scope` and `expandedFolders` as real,
 * round-tripping state — needed for the narrow/widen and per-Vault
 * folder-memory tests, which must observe state ExplorerPane itself owns
 * (`unfoldedVaultId`) surviving a controlled prop change. */
function renderStatefulPane(
  overrides: Partial<Parameters<typeof ExplorerPane>[0]> = {},
) {
  function Wrapper() {
    const initial = { ...defaultPaneProps(), ...overrides };
    const [scope, setScope] = useState(initial.scope);
    const [expandedFolders, setExpandedFolders] = useState(
      initial.expandedFolders,
    );
    return (
      <ExplorerPane
        {...initial}
        scope={scope}
        onScopeChange={setScope}
        expandedFolders={expandedFolders}
        onExpandedFoldersChange={setExpandedFolders}
      />
    );
  }

  render(
    <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
      <Wrapper />
    </MemoryRouter>,
  );
}

describe("ExplorerPane", () => {
  afterEach(cleanup);

  it("renders every rail destination", () => {
    renderPane();

    expect(screen.getByRole("link", { name: "Stats" })).toHaveAttribute(
      "href",
      "/stats",
    );
    expect(screen.getByRole("link", { name: "Graph" })).toHaveAttribute(
      "href",
      "/graph",
    );
    expect(
      screen.getByRole("button", { name: "Recently changed notes" }),
    ).toBeInTheDocument();
    const settings = screen.getByRole("link", { name: "Settings" });
    expect(settings).toHaveAttribute("href", "/settings");
    expect(settings).toHaveClass("explorer-rail-settings");
  });

  it("hides the footer create action when write mode is off", () => {
    renderPane({ writeEnabled: false });

    expect(
      screen.queryByRole("button", { name: "New note" }),
    ).not.toBeInTheDocument();
  });

  it("highlights the active note in the tree but not in recently viewed", () => {
    renderPane();

    // The multi-highlight bug in issue #12 was every list applying this at
    // once. The tree is the single canonical place for it.
    const recent = screen.getByTestId("recent-notes");
    expect(within(recent).getByRole("link", { name: "Home" })).not.toHaveClass(
      "active-note",
    );

    const links = screen.getAllByRole("link", { name: "Home" });
    const active = links.filter((link) =>
      link.className.includes("active-note"),
    );
    expect(active).toHaveLength(1);
  });

  it("opens the changes panel from the rail", () => {
    renderPane();

    expect(
      screen.queryByRole("region", { name: "Recently changed notes" }),
    ).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "Recently changed notes" }),
    );

    const panel = screen.getByRole("region", {
      name: "Recently changed notes",
    });
    expect(
      within(panel).getByRole("link", { name: "Finance" }),
    ).toBeInTheDocument();
  });
});

/** Scopes a query to the Scope zone alone: with the accordion (#142) also
 * rendered under `all` at more than one Vault, a Vault's name now appears
 * twice on screen by design — once as a Scope zone row, once as an
 * accordion head — so an unscoped query is ambiguous. */
function scopeZone() {
  return within(document.querySelector(".scope-zone") as HTMLElement);
}

describe("ExplorerPane Scope zone", () => {
  afterEach(cleanup);

  it("is absent at one enabled Vault", () => {
    renderPane({ vaults: [THREE_VAULTS[0]] });

    expect(screen.queryByText("Scope")).not.toBeInTheDocument();
  });

  it("is absent on mobile", () => {
    renderPane({ vaults: THREE_VAULTS, isMobile: true });

    expect(screen.queryByText("Scope")).not.toBeInTheDocument();
  });

  it("lists All Vaults plus every enabled Vault in Vault-management order", () => {
    renderPane({ vaults: THREE_VAULTS });

    const rows = screen
      .getAllByRole("button")
      .filter((button) => button.className.includes("scope-row"));
    expect(
      rows.map((row) => row.querySelector(".scope-row-label")?.textContent),
    ).toEqual(["All Vaults", "Alpha", "Beta", "Gamma"]);
  });

  it("gives the current scope the active treatment", () => {
    renderPane({ vaults: THREE_VAULTS, scope: THREE_VAULTS[1].vault_id });

    const selected = screen.getByRole("button", { name: /^Beta/ });
    expect(selected).toHaveClass("is-selected");
    expect(
      screen.getByRole("button", { name: /^All Vaults/ }),
    ).not.toHaveClass("is-selected");
  });

  it("picking a row changes scope and nothing else", () => {
    const onScopeChange = vi.fn();
    renderPane({ vaults: THREE_VAULTS, onScopeChange });

    fireEvent.click(scopeZone().getByRole("button", { name: /^Beta/ }));

    expect(onScopeChange).toHaveBeenCalledExactlyOnceWith(
      THREE_VAULTS[1].vault_id,
    );
  });

  it("folds and unfolds, calling back with the next collapsed state", () => {
    const onScopeZoneCollapsedChange = vi.fn();
    renderPane({ vaults: THREE_VAULTS, onScopeZoneCollapsedChange });

    fireEvent.click(screen.getByRole("button", { name: /Scope/ }));

    expect(onScopeZoneCollapsedChange).toHaveBeenCalledExactlyOnceWith(true);
  });

  it("defaults to expanded", () => {
    renderPane({ vaults: THREE_VAULTS });

    expect(
      screen.getByRole("button", { name: /Scope/ }),
    ).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getByRole("button", { name: /^All Vaults/ }),
    ).toBeVisible();
  });

  it("collapsed head names the current scope in place of the count", () => {
    renderPane({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
      scopeZoneCollapsed: true,
    });

    const head = screen.getByRole("button", { name: /Scope/ });
    expect(head).toHaveTextContent("Beta");
    expect(
      screen.queryByRole("button", { name: "All Vaults" }),
    ).not.toBeInTheDocument();
  });

  it("marks the open note's Vault with the viewing marker when expanded", () => {
    renderPane({
      vaults: THREE_VAULTS,
      scope: "all",
      viewingVaultId: THREE_VAULTS[2].vault_id,
    });

    const row = scopeZone().getByRole("button", { name: /Gamma/ });
    expect(within(row).getByText("Viewing")).toBeInTheDocument();
  });

  it("names the viewed Vault on a second line when collapsed, even when it is also the selected scope", () => {
    renderPane({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[2].vault_id,
      viewingVaultId: THREE_VAULTS[2].vault_id,
      scopeZoneCollapsed: true,
    });

    expect(screen.getByText("Viewing: Gamma")).toBeInTheDocument();
  });

  it("does not show a viewing line when no note is open", () => {
    renderPane({
      vaults: THREE_VAULTS,
      scopeZoneCollapsed: true,
      viewingVaultId: undefined,
    });

    expect(screen.queryByText(/^Viewing:/)).not.toBeInTheDocument();
  });

  it("wires each row's note count from vaultNoteCounts (#139)", () => {
    // THREE_VAULTS[0] (Alpha) is the fixture set's healthy Vault; Beta is
    // indexing and Gamma is stale, so only Alpha's row shows a count.
    renderPane({
      vaults: THREE_VAULTS,
      vaultNoteCounts: { [THREE_VAULTS[0].vault_id]: 42 },
    });

    const row = scopeZone().getByRole("button", { name: /^Alpha/ });
    expect(within(row).getByText("42")).toBeInTheDocument();
  });

  it("wires a Vault's condition word into its row from live status fields", () => {
    renderPane({
      vaults: [
        THREE_VAULTS[0],
        { ...THREE_VAULTS[1], activation: "unavailable" as const },
        THREE_VAULTS[2],
      ],
    });

    const row = scopeZone().getByRole("button", { name: /^Beta/ });
    expect(within(row).getByText("unavailable")).toHaveClass(
      "vault-tier-error",
    );
  });

  it("gives the collapsed head the worst ink present and the same aggregate as the All Vaults row", () => {
    renderPane({
      vaults: [
        THREE_VAULTS[0],
        { ...THREE_VAULTS[1], activation: "unavailable" as const },
        THREE_VAULTS[2],
      ],
      scopeZoneCollapsed: true,
    });

    const head = screen.getByRole("button", { name: /Scope/ });
    expect(within(head).getByText("All Vaults")).toHaveClass(
      "vault-tier-error",
    );
    expect(within(head).getByText("1 of 3")).toBeInTheDocument();
  });
});

describe("Vault provenance on Recently viewed and Changed on disk (#140)", () => {
  afterEach(cleanup);

  const recentAcrossVaults: RecentNote[] = [
    {
      vaultId: THREE_VAULTS[1].vault_id,
      title: "Beta note",
      slug: "beta-note",
      relativePath: "Beta note",
      viewedAt: 1,
    },
  ];

  const modifiedAcrossVaults: ModifiedNote[] = [
    {
      vault_id: THREE_VAULTS[2].vault_id,
      title: "Gamma note",
      slug: "gamma-note",
      relative_path: "Gamma note",
      mtime_ns: 1,
    },
  ];

  function openChanges() {
    fireEvent.click(
      screen.getByRole("button", { name: "Recently changed notes" }),
    );
  }

  it("shows the Vault prefix on Recently viewed when scope is all and multiple Vaults are enabled", () => {
    renderPane({ vaults: THREE_VAULTS, scope: "all", recentNotes: recentAcrossVaults });

    const recent = screen.getByTestId("recent-notes");
    expect(within(recent).getByText("Beta")).toBeInTheDocument();
  });

  it("hides the Vault prefix on Recently viewed once scope is narrowed", () => {
    renderPane({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
      recentNotes: recentAcrossVaults,
    });

    const recent = screen.getByTestId("recent-notes");
    expect(within(recent).queryByText("Beta")).not.toBeInTheDocument();
  });

  it("hides the Vault prefix on Recently viewed at one enabled Vault", () => {
    renderPane({
      vaults: [THREE_VAULTS[1]],
      scope: "all",
      recentNotes: recentAcrossVaults,
    });

    const recent = screen.getByTestId("recent-notes");
    expect(within(recent).queryByText("Beta")).not.toBeInTheDocument();
  });

  it("shows the Vault prefix on Changed on disk when scope is all and multiple Vaults are enabled", () => {
    renderPane({
      vaults: THREE_VAULTS,
      scope: "all",
      modifiedNotes: modifiedAcrossVaults,
    });
    openChanges();

    const panel = screen.getByRole("region", { name: "Recently changed notes" });
    expect(within(panel).getByText("Gamma")).toBeInTheDocument();
  });

  it("hides the Vault prefix on Changed on disk once scope is narrowed", () => {
    renderPane({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[2].vault_id,
      modifiedNotes: modifiedAcrossVaults,
    });
    openChanges();

    const panel = screen.getByRole("region", { name: "Recently changed notes" });
    expect(within(panel).queryByText("Gamma")).not.toBeInTheDocument();
  });
});

describe("Changed on disk tells the truth about a partial read (#141)", () => {
  afterEach(cleanup);

  function openChanges() {
    fireEvent.click(
      screen.getByRole("button", { name: "Recently changed notes" }),
    );
  }

  it("names only the missing Vaults in a trailing warn-ink line, at three Vaults, without changing ranking", () => {
    const missing = [THREE_VAULTS[2].name];
    renderPane({
      vaults: THREE_VAULTS,
      modifiedNotes: MODIFIED,
      modifiedNotesPartial: true,
      modifiedNotesMissingVaults: missing,
    });
    openChanges();

    const panel = screen.getByRole("region", {
      name: "Recently changed notes",
    });
    expect(
      within(panel).getByText(`${missing[0]} did not answer.`),
    ).toHaveClass("explorer-changes-partial");
    // The note row itself is still there, in the API's own order.
    expect(
      within(panel).getByRole("link", { name: /Finance/ }),
    ).toBeInTheDocument();
  });

  it("names every missing Vault in a trailing line, at eight Vaults", () => {
    const missing = EIGHT_VAULTS.slice(6).map((vault) => vault.name);
    renderPane({
      vaults: EIGHT_VAULTS,
      modifiedNotes: MODIFIED,
      modifiedNotesPartial: true,
      modifiedNotesMissingVaults: missing,
    });
    openChanges();

    const panel = screen.getByRole("region", {
      name: "Recently changed notes",
    });
    expect(
      within(panel).getByText(
        `${missing[0]} and ${missing[1]} did not answer.`,
      ),
    ).toBeInTheDocument();
  });

  it("replaces the empty state with the documented error block when nothing is usable", () => {
    const missing = [THREE_VAULTS[0].name, THREE_VAULTS[1].name];
    renderPane({
      vaults: THREE_VAULTS,
      modifiedNotes: [],
      modifiedNotesPartial: true,
      modifiedNotesMissingVaults: missing,
    });
    openChanges();

    const panel = screen.getByRole("region", {
      name: "Recently changed notes",
    });
    expect(
      within(panel).queryByText("Nothing has changed on disk yet."),
    ).not.toBeInTheDocument();
    expect(document.querySelector(".state-block.error")).not.toBeNull();
    expect(
      within(panel).getByText(
        `${missing[0]} and ${missing[1]} did not answer.`,
      ),
    ).toBeInTheDocument();
  });

  it("shows the plain empty state, not the error block, when the read is not partial", () => {
    renderPane({
      vaults: THREE_VAULTS,
      modifiedNotes: [],
      modifiedNotesPartial: false,
      modifiedNotesMissingVaults: [],
    });
    openChanges();

    expect(document.querySelector(".state-block.error")).toBeNull();
    expect(
      screen.getByText("Nothing has changed on disk yet."),
    ).toBeInTheDocument();
  });
});

describe("ExplorerPane accordion (#142)", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    cleanup();
    localStorage.clear();
  });

  it("shows every Vault a one-line head in Vault-management order, and exactly one tree", () => {
    const vaultTrees = EIGHT_VAULTS.map((vault) => vaultTreeFor(vault));
    renderPane({
      vaults: EIGHT_VAULTS,
      vaultTrees,
      scope: "all",
      locationPathname: `/v/${EIGHT_VAULTS[0].vault_id}/n/x`,
    });

    const heads = accordionHeads();
    expect(heads.map((head) => head.querySelector(".side-label")?.textContent)).toEqual(
      EIGHT_VAULTS.map((vault) => vault.name),
    );
    expect(heads.filter((head) => head.getAttribute("data-open") === "true")).toHaveLength(1);
    expect(document.querySelectorAll(".explorer-nav > .tree.root-tree")).toHaveLength(1);
  });

  it("carries the Vault name and the count-or-condition slot on each head", () => {
    const vaultTrees = THREE_VAULTS.map((vault) => vaultTreeFor(vault));
    renderPane({
      vaults: THREE_VAULTS,
      vaultTrees,
      vaultNoteCounts: { [THREE_VAULTS[0].vault_id]: 7 },
      scope: "all",
      locationPathname: "/",
    });

    expect(within(headFor("Alpha")).getByText("7")).toBeInTheDocument();
    expect(within(headFor("Gamma")).getByText("stale")).toBeInTheDocument();
  });

  it("unfolding a head never calls onScopeChange", () => {
    const onScopeChange = vi.fn();
    const vaultTrees = THREE_VAULTS.map((vault) => vaultTreeFor(vault));
    renderPane({
      vaults: THREE_VAULTS,
      vaultTrees,
      scope: "all",
      onScopeChange,
      locationPathname: "/",
    });

    fireEvent.click(accordionHeads()[1]);

    expect(onScopeChange).not.toHaveBeenCalled();
  });

  it("clicking a head unfolds it and folds whichever was open", () => {
    const vaultTrees = THREE_VAULTS.map((vault) => vaultTreeFor(vault));
    renderPane({
      vaults: THREE_VAULTS,
      vaultTrees,
      scope: "all",
      locationPathname: `/v/${THREE_VAULTS[0].vault_id}/n/x`,
    });

    expect(accordionHeads()[0]).toHaveAttribute("data-open", "true");

    fireEvent.click(accordionHeads()[2]);

    const heads = accordionHeads();
    expect(heads[0]).toHaveAttribute("data-open", "false");
    expect(heads[2]).toHaveAttribute("data-open", "true");
    expect(document.querySelectorAll(".explorer-nav > .tree.root-tree")).toHaveLength(1);
  });

  it("marks an unavailable Vault's head aria-disabled and refuses to unfold it", () => {
    const vaults = [THREE_VAULTS[0], unavailableVault("Down"), THREE_VAULTS[2]];
    const vaultTrees = vaults.map((vault) => vaultTreeFor(vault));
    renderPane({ vaults, vaultTrees, scope: "all", locationPathname: "/" });

    const head = headFor("Down");
    expect(head).toHaveAttribute("aria-disabled", "true");

    fireEvent.click(head);

    expect(head).toHaveAttribute("data-open", "false");
    expect(document.querySelectorAll(".explorer-nav > .tree.root-tree")).toHaveLength(0);
  });

  describe("landing defaults", () => {
    it("unfolds the open note's own Vault", () => {
      const vaultTrees = THREE_VAULTS.map((vault) => vaultTreeFor(vault));
      renderPane({
        vaults: THREE_VAULTS,
        vaultTrees,
        scope: "all",
        locationPathname: `/v/${THREE_VAULTS[2].vault_id}/n/some-note`,
      });

      expect(headFor("Gamma")).toHaveAttribute("data-open", "true");
    });

    it("falls back to the last persisted Vault when landing at the root with no note open", () => {
      localStorage.setItem(
        "hatchdoor.lastUnfoldedVault",
        THREE_VAULTS[1].vault_id,
      );
      const vaultTrees = THREE_VAULTS.map((vault) => vaultTreeFor(vault));
      renderPane({
        vaults: THREE_VAULTS,
        vaultTrees,
        scope: "all",
        locationPathname: "/",
      });

      expect(headFor("Beta")).toHaveAttribute("data-open", "true");
    });

    it("unfolds nothing when landing at the root with neither a note nor a persisted Vault", () => {
      const vaultTrees = THREE_VAULTS.map((vault) => vaultTreeFor(vault));
      renderPane({
        vaults: THREE_VAULTS,
        vaultTrees,
        scope: "all",
        locationPathname: "/",
      });

      expect(accordionHeads().every((head) => head.getAttribute("data-open") === "false")).toBe(
        true,
      );
      expect(document.querySelectorAll(".explorer-nav > .tree.root-tree")).toHaveLength(0);
    });
  });

  it("remembers folder-open state per Vault across unfolding another Vault and back", () => {
    const vaultTrees = THREE_VAULTS.map((vault) => vaultTreeFor(vault));
    renderStatefulPane({
      vaults: THREE_VAULTS,
      vaultTrees,
      scope: "all",
      locationPathname: `/v/${THREE_VAULTS[0].vault_id}/n/x`,
    });

    // Alpha starts unfolded; open its "Journal" folder. jsdom does not
    // implement <details>'s native click-to-toggle activation behavior, so
    // the toggle is driven directly, same as a real browser's own toggle
    // would reach FolderNode's onToggle handler.
    const openJournal = () => {
      const details = screen
        .getByText("Journal", { selector: ".folder-label" })
        .closest("details") as HTMLDetailsElement;
      details.open = true;
      fireEvent(details, new Event("toggle"));
    };
    openJournal();
    expect(
      screen.getByText("Journal", { selector: ".folder-label" }).closest("details"),
    ).toHaveAttribute("open");

    // Switch to Gamma, whose own "Journal" starts closed (not shared).
    fireEvent.click(accordionHeads()[2]);
    expect(
      screen.getByText("Journal", { selector: ".folder-label" }).closest("details"),
    ).not.toHaveAttribute("open");

    // Back to Alpha: its folder is still open.
    fireEvent.click(accordionHeads()[0]);
    expect(
      screen.getByText("Journal", { selector: ".folder-label" }).closest("details"),
    ).toHaveAttribute("open");
  });

  it("narrowing to one Vault renders exactly today's explorer, with the count-or-condition slot on the Notes head", () => {
    renderPane({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
      vaultNoteCounts: { [THREE_VAULTS[1].vault_id]: 12 },
      tree: TREE,
    });

    expect(accordionHeads()).toHaveLength(0);
    const notesHead = screen.getByText("Notes").closest(".side-head");
    expect(notesHead).not.toBeNull();
    // Beta is indexing in the THREE_VAULTS fixture set, so its slot shows the
    // indexing placeholder rather than a plain count.
    expect(notesHead?.querySelector(".vault-slot-indexing")).not.toBeNull();
  });

  it("widening restores the accordion with the Vault just left unfolded", () => {
    const vaultTrees = THREE_VAULTS.map((vault) => vaultTreeFor(vault));
    renderStatefulPane({
      vaults: THREE_VAULTS,
      vaultTrees,
      scope: THREE_VAULTS[2].vault_id,
      locationPathname: "/",
    });

    fireEvent.click(scopeZone().getByRole("button", { name: /^All Vaults/ }));

    expect(headFor("Gamma")).toHaveAttribute("data-open", "true");
  });
});

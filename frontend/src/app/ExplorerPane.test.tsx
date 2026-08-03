import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ExplorerPane } from "./ExplorerPane";
import type { ExplorerFolder, ModifiedNote, RecentNote } from "../types";

const TREE: ExplorerFolder = {
  name: "Vault",
  folders: [
    {
      name: "10-topics",
      folders: [],
      notes: [{ title: "Finance", slug: "finance" }],
    },
  ],
  notes: [{ title: "Home", slug: "home" }],
};

const RECENT: RecentNote[] = [
  {
    title: "Home",
    slug: "home",
    relativePath: "Home",
    viewedAt: 1,
  },
];

const MODIFIED: ModifiedNote[] = [
  {
    title: "Finance",
    slug: "finance",
    relative_path: "10-topics/Finance",
    mtime_ns: 2,
  },
];

function renderPane(
  overrides: Partial<Parameters<typeof ExplorerPane>[0]> = {},
) {
  const props = {
    explorerScrollRef: { current: null },
    drawerOpen: false,
    writeEnabled: true,
    settingsEnabled: true,
    onCreateNoteInFolder: vi.fn(),
    locationPathname: "/n/home",
    recentNotes: RECENT,
    modifiedNotes: MODIFIED,
    loadingTree: false,
    treeError: null,
    tree: TREE,
    expandedFolders: {},
    recentCollapsed: false,
    onRecentCollapsedChange: vi.fn(),
    onExpandedFoldersChange: vi.fn(),
    onCloseDrawer: vi.fn(),
    onRefreshTree: vi.fn(),
    onScrollTopChange: vi.fn(),
    ...overrides,
  };

  render(
    <MemoryRouter initialEntries={["/n/home"]}>
      <ExplorerPane {...props} />
    </MemoryRouter>,
  );

  return props;
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
    expect(screen.getByRole("link", { name: "Settings" })).toHaveAttribute(
      "href",
      "/settings",
    );
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

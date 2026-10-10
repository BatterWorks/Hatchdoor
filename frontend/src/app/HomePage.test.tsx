import { cleanup, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { THREE_VAULTS } from "../test/fixtures/vaults";
import type { ModifiedNote, RecentNote } from "../types";
import { HomePage } from "./HomePage";

const [ALPHA, BETA] = THREE_VAULTS;

function changed(vaultId: string, n: number): ModifiedNote[] {
  return Array.from({ length: n }, (_, i) => ({
    vault_id: vaultId,
    title: `Changed ${i}`,
    slug: `changed-${i}`,
    relative_path: `changed-${i}`,
    mtime_ns: (Date.now() - i * 60_000) * 1_000_000,
  }));
}

function viewed(vaultId: string, n: number): RecentNote[] {
  return Array.from({ length: n }, (_, i) => ({
    vaultId,
    title: `Viewed ${i}`,
    slug: `viewed-${i}`,
    relativePath: `viewed-${i}`,
    viewedAt: Date.now() - i * 60_000,
  }));
}

function renderHome(overrides: Partial<Parameters<typeof HomePage>[0]> = {}) {
  const props = {
    vaults: THREE_VAULTS,
    scope: "all" as const,
    noteCounts: {},
    modifiedNotes: [],
    recentNotes: [],
    writeEnabled: true,
    onNewNote: vi.fn(),
    onOpenSearch: vi.fn(),
    ...overrides,
  };
  render(
    <MemoryRouter>
      <HomePage {...props} />
    </MemoryRouter>,
  );
  return props;
}

describe("HomePage (#530)", () => {
  afterEach(cleanup);

  it("states the note total only once every Vault in scope has reported one", () => {
    renderHome({
      noteCounts: { [ALPHA.vault_id]: 5, [BETA.vault_id]: 7 },
    });
    expect(screen.getByText("3 Vaults")).toBeInTheDocument();

    cleanup();
    renderHome({
      noteCounts: Object.fromEntries(
        THREE_VAULTS.map((vault, i) => [vault.vault_id, i + 1]),
      ),
    });
    expect(screen.getByText("3 Vaults, 6 notes")).toBeInTheDocument();
  });

  it("names the narrowed Vault and counts within it", () => {
    renderHome({
      scope: BETA.vault_id,
      noteCounts: { [BETA.vault_id]: 1 },
    });
    expect(screen.getByText(BETA.name)).toBeInTheDocument();
    expect(screen.getByText("1 Vault, 1 note")).toBeInTheDocument();
  });

  it("shows six rows per list while the head counts the whole list", () => {
    renderHome({
      modifiedNotes: changed(ALPHA.vault_id, 9),
      recentNotes: viewed(ALPHA.vault_id, 8),
    });
    const changedList = screen.getByRole("region", {
      name: "Changed on disk",
    });
    expect(within(changedList).getAllByRole("link")).toHaveLength(6);
    expect(within(changedList).getByText("09")).toBeInTheDocument();
    const recent = screen.getByRole("region", { name: "Recently viewed" });
    expect(within(recent).getAllByRole("link")).toHaveLength(6);
    expect(within(recent).getByText("08")).toBeInTheDocument();
  });

  it("reads Recently viewed through the browsing scope and drops departed Vaults", () => {
    renderHome({
      scope: BETA.vault_id,
      recentNotes: [
        ...viewed(ALPHA.vault_id, 2),
        ...viewed(BETA.vault_id, 1),
        ...viewed("gone", 1),
      ],
    });
    const recent = screen.getByRole("region", { name: "Recently viewed" });
    expect(within(recent).getAllByRole("link")).toHaveLength(1);
  });

  it("carries the Vault prefix only when more than one Vault is enabled", () => {
    renderHome({ modifiedNotes: changed(ALPHA.vault_id, 1) });
    expect(document.querySelector(".path-vault")).toHaveTextContent(ALPHA.name);

    cleanup();
    renderHome({
      vaults: [ALPHA],
      modifiedNotes: changed(ALPHA.vault_id, 1),
    });
    expect(document.querySelector(".path-vault")).toBeNull();
  });

  it("offers New note only with write mode, and Search always", () => {
    const props = renderHome({ writeEnabled: false });
    expect(
      screen.queryByRole("button", { name: "New note" }),
    ).not.toBeInTheDocument();
    screen.getByRole("button", { name: "Search" }).click();
    expect(props.onOpenSearch).toHaveBeenCalledTimes(1);

    cleanup();
    const withWrite = renderHome();
    screen.getByRole("button", { name: "New note" }).click();
    expect(withWrite.onNewNote).toHaveBeenCalledTimes(1);
  });

  it("says so when a list is empty", () => {
    renderHome();
    expect(
      screen.getByText("Nothing has changed on disk yet."),
    ).toBeInTheDocument();
    expect(screen.getByText(/Notes you open show up here/)).toBeInTheDocument();
  });
});

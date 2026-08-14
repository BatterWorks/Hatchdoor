import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppTopbar } from "./AppTopbar";
import { THREE_VAULTS } from "../test/fixtures/vaults";
import type { ActiveNoteMeta } from "../types";

function renderTopbar(
  overrides: Partial<Parameters<typeof AppTopbar>[0]> = {},
) {
  const props = {
    activeNote: null,
    vaults: [] as typeof THREE_VAULTS,
    scope: "all" as const,
    writeEnabled: false,
    isMobile: false,
    isOnline: true,
    actionsMenuOpen: false,
    theme: "auto" as const,
    onToggleDrawer: vi.fn(),
    onOpenSearch: vi.fn(),
    onToggleActionsMenu: vi.fn(),
    onCloseActionsMenu: vi.fn(),
    onCopyPageContent: vi.fn(),
    onCopyNoteLink: vi.fn(),
    onDownloadMarkdown: vi.fn(),
    onEditNote: vi.fn(),
    onNewNote: vi.fn(),
    onRenameNote: vi.fn(),
    onMoveNote: vi.fn(),
    onArchiveNote: vi.fn(),
    onDeleteNote: vi.fn(),
    onCycleTheme: vi.fn(),
    onScopeChange: vi.fn(),
    viewingVaultId: undefined,
    vaultNoteCounts: {},
    scopeSheetOpen: false,
    onToggleScopeSheet: vi.fn(),
    onCloseScopeSheet: vi.fn(),
    scopeFocusRequestId: 0,
    onRestoreScopeFocus: vi.fn(),
    ...overrides,
  };

  render(<AppTopbar {...props} />);
  return props;
}

describe("AppTopbar scope echo", () => {
  afterEach(cleanup);

  it("shows no echo and no scope control at one enabled Vault", () => {
    renderTopbar({ vaults: [THREE_VAULTS[0]], scope: "all" });

    expect(screen.queryByText("All Vaults")).not.toBeInTheDocument();
    expect(screen.queryByText(THREE_VAULTS[0].name)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /scope/i }),
    ).not.toBeInTheDocument();
  });

  it("echoes the selected scope when no note is open", () => {
    renderTopbar({ vaults: THREE_VAULTS, scope: "all" });

    expect(screen.getByText("All Vaults")).toBeInTheDocument();
  });

  it("echoes a narrowed scope by Vault name", () => {
    renderTopbar({ vaults: THREE_VAULTS, scope: THREE_VAULTS[1].vault_id });

    expect(screen.getByText("Beta")).toBeInTheDocument();
  });

  it("echoes the open note's own Vault, even when scope is narrowed elsewhere", () => {
    const activeNote: ActiveNoteMeta = {
      vaultId: THREE_VAULTS[2].vault_id,
      title: "A note",
      slug: "a-note",
      relativePath: "a-note",
    };
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
      activeNote,
    });

    expect(screen.getByText("Gamma")).toBeInTheDocument();
    expect(screen.queryByText("Beta")).not.toBeInTheDocument();
  });

  it("carries no scope control — the echo is not a button", () => {
    renderTopbar({ vaults: THREE_VAULTS, scope: "all" });

    const echo = screen.getByText("All Vaults");
    expect(echo.tagName).not.toBe("BUTTON");
    expect(echo.closest("button")).toBeNull();
  });
});

function scopeTrigger(): HTMLElement | null {
  return document.querySelector(".topbar-scope-trigger");
}

function scopeSheet(): HTMLElement {
  const sheet = document.querySelector(".scope-sheet");
  if (!sheet) {
    throw new Error("scope sheet not found");
  }
  return sheet as HTMLElement;
}

describe("AppTopbar mobile scope row (#145)", () => {
  afterEach(cleanup);

  it("is absent on desktop", () => {
    renderTopbar({ vaults: THREE_VAULTS, scope: "all", isMobile: false });

    expect(scopeTrigger()).toBeNull();
  });

  it("is absent at one enabled Vault, same as the desktop echo", () => {
    renderTopbar({
      vaults: [THREE_VAULTS[0]],
      scope: "all",
      isMobile: true,
    });

    expect(scopeTrigger()).toBeNull();
  });

  it("shows the browsing scope's name and slot at all times, with no interaction", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      vaultNoteCounts: { [THREE_VAULTS[0].vault_id]: 5 },
    });

    const trigger = scopeTrigger();
    expect(trigger).not.toBeNull();
    expect(
      within(trigger as HTMLElement).getByText("All Vaults"),
    ).toBeInTheDocument();
  });

  it("names a narrowed scope by Vault name", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
      isMobile: true,
    });

    expect(
      within(scopeTrigger() as HTMLElement).getByText("Beta"),
    ).toBeInTheDocument();
  });

  it("shows the viewing marker when the exact read's Vault differs from a narrowed scope", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[0].vault_id,
      viewingVaultId: THREE_VAULTS[2].vault_id,
      isMobile: true,
    });

    const trigger = within(scopeTrigger() as HTMLElement);
    expect(trigger.getByText(/viewing/i)).toBeInTheDocument();
    expect(trigger.getByText(/gamma/i)).toBeInTheDocument();
  });

  it("omits the viewing marker when the exact read matches the narrowed scope", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[0].vault_id,
      viewingVaultId: THREE_VAULTS[0].vault_id,
      isMobile: true,
    });

    expect(
      within(scopeTrigger() as HTMLElement).queryByText(/viewing/i),
    ).not.toBeInTheDocument();
  });

  it("omits the viewing marker at all scope, even with a note open elsewhere", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      viewingVaultId: THREE_VAULTS[2].vault_id,
      isMobile: true,
    });

    expect(
      within(scopeTrigger() as HTMLElement).queryByText(/viewing/i),
    ).not.toBeInTheDocument();
  });

  it("omits the viewing marker with no note open", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[0].vault_id,
      viewingVaultId: undefined,
      isMobile: true,
    });

    expect(
      within(scopeTrigger() as HTMLElement).queryByText(/viewing/i),
    ).not.toBeInTheDocument();
  });

  it("tapping the row toggles the sheet", () => {
    const onToggleScopeSheet = vi.fn();
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      onToggleScopeSheet,
    });

    fireEvent.click(scopeTrigger() as HTMLElement);
    expect(onToggleScopeSheet).toHaveBeenCalledTimes(1);
  });
});

describe("AppTopbar mobile scope sheet (#145)", () => {
  afterEach(cleanup);

  it("lists All Vaults first, then every Vault in Vault-management order", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      scopeSheetOpen: true,
    });

    const rows = within(scopeSheet()).getAllByRole("radio");
    expect(rows.map((row) => row.textContent)).toEqual([
      expect.stringContaining("All Vaults"),
      expect.stringContaining("Alpha"),
      expect.stringContaining("Beta"),
      expect.stringContaining("Gamma"),
    ]);
  });

  it("marks the current scope's row selected", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
      isMobile: true,
      scopeSheetOpen: true,
    });

    const betaRow = within(scopeSheet()).getByText("Beta").closest("button");
    expect(betaRow?.className).toMatch(/is-selected/);
  });

  it("picking a Vault row sets scope and dismisses the sheet", () => {
    const onScopeChange = vi.fn();
    const onCloseScopeSheet = vi.fn();
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      scopeSheetOpen: true,
      onScopeChange,
      onCloseScopeSheet,
    });

    fireEvent.click(within(scopeSheet()).getByText("Beta"));
    expect(onScopeChange).toHaveBeenCalledWith(THREE_VAULTS[1].vault_id);
    expect(onCloseScopeSheet).toHaveBeenCalledTimes(1);
  });

  it("picking All Vaults sets scope to all and dismisses the sheet", () => {
    const onScopeChange = vi.fn();
    const onCloseScopeSheet = vi.fn();
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
      isMobile: true,
      scopeSheetOpen: true,
      onScopeChange,
      onCloseScopeSheet,
    });

    fireEvent.click(within(scopeSheet()).getByText("All Vaults"));
    expect(onScopeChange).toHaveBeenCalledWith("all");
    expect(onCloseScopeSheet).toHaveBeenCalledTimes(1);
  });

  it("closes when the backdrop is clicked", () => {
    const onCloseScopeSheet = vi.fn();
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      scopeSheetOpen: true,
      onCloseScopeSheet,
    });

    fireEvent.click(
      document.querySelector(".scope-sheet-backdrop") as HTMLElement,
    );
    expect(onCloseScopeSheet).toHaveBeenCalledTimes(1);
  });

  it("restores focus to the shortcut origin when the backdrop closes it without picking", () => {
    const onRestoreScopeFocus = vi.fn();
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      scopeSheetOpen: true,
      onRestoreScopeFocus,
    });

    fireEvent.click(
      document.querySelector(".scope-sheet-backdrop") as HTMLElement,
    );
    expect(onRestoreScopeFocus).toHaveBeenCalledTimes(1);
  });

  it("is a pick-exactly-one radiogroup, one tab stop for the whole group", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
      isMobile: true,
      scopeSheetOpen: true,
    });

    const rows = within(scopeSheet()).getAllByRole("radio");
    expect(rows.map((row) => row.getAttribute("tabindex"))).toEqual([
      "-1",
      "-1",
      "0",
      "-1",
    ]);
    expect(rows[2]).toHaveAttribute("aria-checked", "true");
  });

  it("focuses the current row the instant the sheet opens", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: THREE_VAULTS[1].vault_id,
      isMobile: true,
      scopeSheetOpen: true,
    });

    expect(
      within(scopeSheet()).getByRole("radio", { name: /^Beta/ }),
    ).toHaveFocus();
  });

  it("moves focus between rows with the arrow keys", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      scopeSheetOpen: true,
    });

    const allVaultsRow = within(scopeSheet()).getByRole("radio", {
      name: /^All Vaults/,
    });
    fireEvent.keyDown(allVaultsRow, { key: "ArrowDown" });

    expect(
      within(scopeSheet()).getByRole("radio", { name: /^Alpha/ }),
    ).toHaveFocus();
  });

  it("closes and restores focus on Escape", () => {
    const onCloseScopeSheet = vi.fn();
    const onRestoreScopeFocus = vi.fn();
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      scopeSheetOpen: true,
      onCloseScopeSheet,
      onRestoreScopeFocus,
    });

    fireEvent.keyDown(document, { key: "Escape" });

    expect(onCloseScopeSheet).toHaveBeenCalledTimes(1);
    expect(onRestoreScopeFocus).toHaveBeenCalledTimes(1);
  });

  it("focuses the topbar trigger after picking a row, not the shortcut origin", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      scopeSheetOpen: true,
    });

    fireEvent.click(within(scopeSheet()).getByText("Beta"));

    expect(scopeTrigger()).toHaveFocus();
  });

  it("traps Tab within the sheet", () => {
    renderTopbar({
      vaults: THREE_VAULTS,
      scope: "all",
      isMobile: true,
      scopeSheetOpen: true,
    });

    const rows = within(scopeSheet()).getAllByRole("radio");
    const first = rows[0];
    const last = rows[rows.length - 1];
    last.focus();

    fireEvent.keyDown(document, { key: "Tab" });
    expect(first).toHaveFocus();

    fireEvent.keyDown(document, { key: "Tab", shiftKey: true });
    expect(last).toHaveFocus();
  });
});

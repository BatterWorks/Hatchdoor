import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppTopbar } from "./AppTopbar";
import {
  conflictVault,
  healthyVault,
  staleVault,
  syncFailedVault,
  THREE_VAULTS,
} from "../test/fixtures/vaults";
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
    onRenameNote: vi.fn(),
    onMoveNote: vi.fn(),
    onArchiveNote: vi.fn(),
    onDeleteNote: vi.fn(),
    onSetTheme: vi.fn(),
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

describe("AppTopbar single-Vault condition row (#334)", () => {
  afterEach(cleanup);

  it.each([
    ["conflict", conflictVault("Solo")],
    ["sync failed", syncFailedVault("Solo")],
    ["stale", staleVault("Solo")],
  ])(
    "shows %s on mobile at one enabled Vault, with nothing to press",
    (word, vault) => {
      renderTopbar({ vaults: [vault], scope: "all", isMobile: true });

      const row = document.querySelector(".topbar-mobile-meta");
      expect(row).not.toBeNull();
      expect(within(row as HTMLElement).getByText("Solo")).toBeInTheDocument();
      expect(within(row as HTMLElement).getByText(word)).toBeInTheDocument();
      expect(within(row as HTMLElement).queryByRole("button")).toBeNull();
    },
  );

  it("stays absent for a healthy single Vault", () => {
    renderTopbar({ vaults: [healthyVault("Solo")], isMobile: true });

    expect(document.querySelector(".topbar-mobile-meta")).toBeNull();
  });

  it("stays absent on desktop, where the explorer head reports it", () => {
    renderTopbar({ vaults: [conflictVault("Solo")], isMobile: false });

    expect(document.querySelector(".topbar-mobile-meta")).toBeNull();
  });
});

describe("AppTopbar Help entry (#417)", () => {
  afterEach(cleanup);

  it("shows a Help button beside the theme toggle on wide screens", () => {
    const props = renderTopbar({ onToggleHelp: vi.fn() });

    const help = screen.getByRole("button", { name: "Help" });
    expect(help).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(help);
    expect(props.onToggleHelp).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menuitem", { name: "Help" })).toBeNull();
  });

  it("marks the button while Help is open", () => {
    renderTopbar({ helpOpen: true });

    expect(screen.getByRole("button", { name: "Help" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("keeps Help out of the top bar and the … menu on phones; the drawer's rail carries it (#530)", () => {
    renderTopbar({
      isMobile: true,
      actionsMenuOpen: true,
      writeEnabled: true,
      onToggleHelp: vi.fn(),
    });

    expect(screen.queryByRole("button", { name: "Help" })).toBeNull();
    const items = within(screen.getByRole("menu")).queryAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).not.toContain("Help");
  });
});

describe("AppTopbar theme menu (#530)", () => {
  afterEach(cleanup);

  it("opens a three-option menu, focuses the current choice, and picks by name", () => {
    const props = renderTopbar({ theme: "light" });
    fireEvent.click(screen.getByRole("button", { name: "Theme: Light" }));

    const items = screen.getAllByRole("menuitemradio");
    expect(items.map((item) => item.textContent)).toEqual([
      "Systemfollows the device",
      "Light",
      "Dark",
    ]);
    expect(items[1]).toHaveAttribute("aria-checked", "true");
    expect(items[1]).toHaveFocus();

    fireEvent.keyDown(document, { key: "ArrowDown" });
    expect(items[2]).toHaveFocus();
    fireEvent.keyDown(document, { key: "ArrowDown" });
    expect(items[0]).toHaveFocus();

    fireEvent.click(items[2]);
    expect(props.onSetTheme).toHaveBeenCalledExactlyOnceWith("dark");
    expect(
      screen.getByRole("button", { name: "Theme: Light" }),
    ).toHaveAttribute("aria-expanded", "false");
  });

  it("closes on Escape and gives focus back to the button", () => {
    renderTopbar({ theme: "auto" });
    const button = screen.getByRole("button", { name: "Theme: System" });
    fireEvent.click(button);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(button).toHaveAttribute("aria-expanded", "false");
    expect(button).toHaveFocus();
  });
});

describe("AppTopbar On this page chip (#530)", () => {
  afterEach(cleanup);
  const headings = [
    { level: 2, text: "Role", id: "role", sourceLine: 3 },
    { level: 3, text: "Ports", id: "ports", sourceLine: 5 },
  ];

  it("renders below 920px on a note with headings, even at one Vault, and opens a sheet", () => {
    const onJumpToHeading = vi.fn();
    renderTopbar({
      isMobile: true,
      vaults: [THREE_VAULTS[0]],
      tocHeadings: headings,
      onJumpToHeading,
    });
    const chip = screen.getByRole("button", { name: /On this page/ });
    expect(chip).toHaveTextContent("2");
    fireEvent.click(chip);
    const sheet = screen.getByRole("dialog", { name: "On this page" });
    expect(sheet).toHaveAttribute("data-open", "true");
    fireEvent.click(within(sheet).getByRole("button", { name: "Ports" }));
    expect(onJumpToHeading).toHaveBeenCalledExactlyOnceWith("ports");
    expect(sheet).toHaveAttribute("data-open", "false");
  });

  it("is absent on wide screens and without headings", () => {
    renderTopbar({ isMobile: false, tocHeadings: headings });
    expect(
      screen.queryByRole("button", { name: /On this page/ }),
    ).not.toBeInTheDocument();
    cleanup();
    renderTopbar({ isMobile: true, tocHeadings: [] });
    expect(
      screen.queryByRole("button", { name: /On this page/ }),
    ).not.toBeInTheDocument();
  });
});

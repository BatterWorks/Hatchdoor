import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppTopbar } from "./AppTopbar";
import { THREE_VAULTS } from "../test/fixtures/vaults";
import type { ActiveNoteMeta } from "../types";

function renderTopbar(overrides: Partial<Parameters<typeof AppTopbar>[0]> = {}) {
  const props = {
    activeNote: null,
    vaults: [] as typeof THREE_VAULTS,
    scope: "all" as const,
    writeEnabled: false,
    isMobile: false,
    isOnline: true,
    treeIsStale: false,
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

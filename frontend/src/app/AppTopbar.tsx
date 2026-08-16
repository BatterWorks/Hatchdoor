import { useEffect, useRef, type ReactElement, type Ref } from "react";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

import { StatusBadge, UiButton } from "../components/ui";
import {
  ContrastIcon,
  DarkModeIcon,
  LightModeIcon,
  MenuIcon,
  MoreHorizIcon,
  SearchIcon,
} from "../components/icons";
import { VaultAggregateSlot, VaultSlot } from "./vaultSlot";
import { scopeName } from "./vaultSlotLogic";
import type {
  ActiveNoteMeta,
  VaultId,
  VaultScope,
  VaultSummary,
} from "../types";
import type { Theme } from "../hooks/useTheme";

// One icon per theme state, mirroring the three-way cycle. The icon shows the
// theme that is currently active, which is what the previous glyphs did too.
//
// `contrast` for auto, not `brightness_auto`: at 18px the latter's sun-with-A
// reads as a circle with radiating spikes, near-identical to the settings gear
// sitting in the sidebar rail just below it. This half-filled circle is also
// closer to the ◑ it replaces.
const THEME_ICON: Record<Theme, ReactElement> = {
  auto: <ContrastIcon />,
  light: <LightModeIcon />,
  dark: <DarkModeIcon />,
};
const THEME_LABEL: Record<Theme, string> = {
  auto: "Theme: System",
  light: "Theme: Light",
  dark: "Theme: Dark",
};

type TopbarProps = {
  activeNote: ActiveNoteMeta | null;
  vaults: VaultSummary[];
  scope: VaultScope;
  writeEnabled: boolean;
  isMobile: boolean;
  isOnline: boolean;
  actionsMenuOpen: boolean;
  topbarRef?: Ref<HTMLElement>;
  theme: Theme;
  onToggleDrawer: () => void;
  onOpenSearch: () => void;
  onToggleActionsMenu: () => void;
  onCloseActionsMenu: () => void;
  onCopyPageContent: () => void;
  onCopyNoteLink: () => void;
  onDownloadMarkdown: () => void;
  onEditNote: () => void;
  onNewNote: () => void;
  onRenameNote: () => void;
  onMoveNote: () => void;
  onArchiveNote: () => void;
  onDeleteNote: () => void;
  onCycleTheme: () => void;
  onScopeChange: (next: VaultScope) => void;
  viewingVaultId: VaultId | undefined;
  vaultNoteCounts: Record<VaultId, number | undefined>;
  scopeSheetOpen: boolean;
  onToggleScopeSheet: () => void;
  onCloseScopeSheet: () => void;
  scopeFocusRequestId: number;
  onRestoreScopeFocus: () => void;
  /** Clamps every condition slot to the amber tier (#152). */
  demoMode?: boolean;
};

export function AppTopbar({
  activeNote,
  vaults,
  scope,
  writeEnabled,
  isMobile,
  isOnline,
  actionsMenuOpen,
  topbarRef,
  theme,
  onToggleDrawer,
  onOpenSearch,
  onToggleActionsMenu,
  onCloseActionsMenu,
  onCopyPageContent,
  onCopyNoteLink,
  onDownloadMarkdown,
  onEditNote,
  onNewNote,
  onRenameNote,
  onMoveNote,
  onArchiveNote,
  onDeleteNote,
  onCycleTheme,
  onScopeChange,
  viewingVaultId,
  vaultNoteCounts,
  scopeSheetOpen,
  onToggleScopeSheet,
  onCloseScopeSheet,
  scopeFocusRequestId,
  onRestoreScopeFocus,
  demoMode = false,
}: TopbarProps) {
  const actionsMenuRef = useRef<HTMLDivElement>(null);
  const scopeHostRef = useRef<HTMLDivElement>(null);
  const scopeTriggerRef = useRef<HTMLButtonElement>(null);
  const scopeSheetRef = useRef<HTMLDivElement>(null);
  const scopeRowRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const crumbText = activeNote
    ? activeNote.relativePath.replace(/\//g, " / ")
    : "Notes Explorer";
  // Read-only: no scope control lives in the topbar (#138). Echoes the open
  // note's own Vault where one is open, else the selected scope — same
  // precedence the sidebar's collapsed Scope zone head uses. Absent at one
  // enabled Vault, where scope has nothing to say.
  const scopeEcho =
    vaults.length > 1
      ? (vaults.find((vault) => vault.vault_id === activeNote?.vaultId)?.name ??
        (scope === "all"
          ? "All Vaults"
          : (vaults.find((vault) => vault.vault_id === scope)?.name ??
            "All Vaults")))
      : null;
  // The menu's three groups are mutate / utility / destructive, so Archive and
  // Delete land last where a reader expects them. Both dividers sit next to a
  // group that only renders with a note and write mode, so without one they
  // would be stray rules against nothing.
  const showMenuDividers = Boolean(activeNote) && writeEnabled;

  // Below 920px, col 2's breadcrumb is CSS-hidden, and this second row takes
  // over as the only place scope is legible (#145). Absent below two enabled
  // Vaults, same as the desktop echo and the sidebar Scope zone — narrowing
  // has nothing to offer there.
  const showScopeRow = isMobile && vaults.length > 1;
  const narrowedScopeVault =
    scope === "all"
      ? undefined
      : vaults.find((vault) => vault.vault_id === scope);
  const scopeSlot =
    scope === "all" ? (
      <VaultAggregateSlot
        vaults={vaults}
        counts={vaultNoteCounts}
        demoMode={demoMode}
      />
    ) : narrowedScopeVault ? (
      <VaultSlot
        vault={narrowedScopeVault}
        noteCount={vaultNoteCounts[narrowedScopeVault.vault_id]}
        demoMode={demoMode}
      />
    ) : null;
  // The slot above always names the browsing scope, never the open note's
  // Vault — this marker is the one exception, and only earns its place when
  // an exact read disagrees with a *narrowed* scope. At `all` every open note
  // is already within scope, so there is nothing to flag.
  const viewingVault = vaults.find(
    (vault) => vault.vault_id === viewingVaultId,
  );
  const showViewingMarker =
    scope !== "all" && viewingVault !== undefined && viewingVaultId !== scope;

  useEffect(() => {
    if (!actionsMenuOpen) {
      return;
    }

    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && actionsMenuRef.current?.contains(target)) {
        return;
      }
      onCloseActionsMenu();
    };

    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [actionsMenuOpen, onCloseActionsMenu]);

  // Rows are a pick-exactly-one radiogroup (#146), mirroring the desktop
  // Scope zone: `all` first, then every Vault in Vault-management order.
  const scopeRowIds: VaultScope[] = [
    "all",
    ...vaults.map((vault) => vault.vault_id),
  ];
  const selectedScopeRowIndex = Math.max(0, scopeRowIds.indexOf(scope));

  const closeScopeSheetWithoutPicking = () => {
    onCloseScopeSheet();
    onRestoreScopeFocus();
  };

  const pickScope = (next: VaultScope) => {
    onScopeChange(next);
    onCloseScopeSheet();
    scopeTriggerRef.current?.focus();
  };

  const focusScopeRow = (index: number) => {
    const wrapped = (index + scopeRowIds.length) % scopeRowIds.length;
    scopeRowRefs.current[wrapped]?.focus();
  };

  useEffect(() => {
    if (!scopeSheetOpen) {
      return;
    }

    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && scopeHostRef.current?.contains(target)) {
        return;
      }
      closeScopeSheetWithoutPicking();
    };

    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scopeSheetOpen]);

  // Focus lands on the current scope row the instant the sheet opens (#146)
  // — whether opened by `v` or by tapping the trigger — and the sheet traps
  // Tab within itself and closes on `Escape`, restoring focus like any other
  // dialog (#146's general "traps focus and returns it on close").
  useEffect(() => {
    if (!scopeSheetOpen) {
      return;
    }

    scopeRowRefs.current[selectedScopeRowIndex]?.focus();

    const root = scopeSheetRef.current;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeScopeSheetWithoutPicking();
        return;
      }
      if (event.key !== "Tab" || !root) {
        return;
      }
      const items = Array.from(
        root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      );
      if (items.length === 0) {
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
    // Refocusing the selected row on every render (e.g. a scope-independent
    // rerender) would fight the user's own arrow-key navigation; only the
    // sheet opening or a fresh `v` press should re-home focus.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scopeSheetOpen, scopeFocusRequestId]);

  return (
    <>
      <div className="hotbar" aria-hidden="true" />
      <header className="app-topbar" ref={topbarRef}>
        {/* Col 1 — Brand */}
        <div className="topbar-brand">
          {isMobile ? (
            <button
              type="button"
              className="icon-button"
              onClick={onToggleDrawer}
              aria-label="Toggle explorer"
            >
              <MenuIcon />
            </button>
          ) : null}
          <svg
            className="brand-wordmark"
            viewBox="0 0 340 60"
            aria-label="Hatchdoor"
            role="img"
            focusable="false"
          >
            {/* Left bracket */}
            <rect x="4" y="4" width="9" height="52" fill="currentColor" />
            <rect x="4" y="4" width="16" height="9" fill="currentColor" />
            <rect x="4" y="47" width="16" height="9" fill="currentColor" />
            {/* Right bracket */}
            <rect x="47" y="4" width="9" height="52" fill="currentColor" />
            <rect x="40" y="4" width="16" height="9" fill="currentColor" />
            <rect x="40" y="47" width="16" height="9" fill="currentColor" />
            {/* Accent square */}
            <rect x="24" y="24" width="12" height="12" fill="var(--hot)" />
            {/* Wordmark text */}
            <text
              className="brand-wordmark-text"
              x="76"
              y="47"
              aria-hidden="true"
            >
              HATCHDOOR
            </text>
          </svg>
        </div>

        {/* Col 2 — Breadcrumb */}
        <div className="topbar-crumb">
          {scopeEcho ? (
            <>
              <span className="topbar-crumb-scope">{scopeEcho}</span>
              <span className="topbar-crumb-sep" aria-hidden="true">
                /
              </span>
            </>
          ) : null}
          <span className="topbar-crumb-here">{crumbText}</span>
          {!isOnline && (
            <span style={{ marginLeft: "0.5rem" }}>
              <StatusBadge tone="error" text="Offline" />
            </span>
          )}
        </div>

        {/* Col 3 — Actions */}
        <div className="topbar-actions">
          {!isMobile ? (
            <button
              type="button"
              className="topbar-search-trigger"
              onClick={onOpenSearch}
            >
              <span>Search</span>
              <span className="shortcut-hint" aria-hidden="true">
                ⌘K
              </span>
            </button>
          ) : (
            <button
              type="button"
              className="icon-button"
              onClick={onOpenSearch}
              aria-label="Search notes"
            >
              <SearchIcon />
            </button>
          )}
          <button
            type="button"
            className="icon-button"
            onClick={onCycleTheme}
            aria-label={THEME_LABEL[theme]}
            title={THEME_LABEL[theme]}
          >
            {THEME_ICON[theme]}
          </button>
          <div className="topbar-menu-host" ref={actionsMenuRef}>
            <button
              type="button"
              className="icon-button"
              onClick={onToggleActionsMenu}
              aria-haspopup="menu"
              aria-expanded={actionsMenuOpen}
              aria-label="More actions"
            >
              <MoreHorizIcon />
            </button>
            <div
              className="topbar-menu"
              role="menu"
              aria-hidden={!actionsMenuOpen}
              data-open={actionsMenuOpen}
            >
              {writeEnabled ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onNewNote();
                  }}
                >
                  New note
                </UiButton>
              ) : null}
              {activeNote && writeEnabled ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onEditNote();
                  }}
                >
                  Edit note
                </UiButton>
              ) : null}
              {activeNote && writeEnabled ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onRenameNote();
                  }}
                >
                  Rename note
                </UiButton>
              ) : null}
              {activeNote && writeEnabled ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onMoveNote();
                  }}
                >
                  Move note
                </UiButton>
              ) : null}
              {showMenuDividers ? (
                <div className="topbar-menu-divider" role="separator" />
              ) : null}
              {activeNote ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onCopyPageContent();
                  }}
                >
                  Copy page content
                </UiButton>
              ) : null}
              {activeNote ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onDownloadMarkdown();
                  }}
                >
                  Download .md
                </UiButton>
              ) : null}
              {activeNote ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onCopyNoteLink();
                  }}
                >
                  Copy note link
                </UiButton>
              ) : null}
              {showMenuDividers ? (
                <div className="topbar-menu-divider" role="separator" />
              ) : null}
              {activeNote && writeEnabled ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onArchiveNote();
                  }}
                >
                  Archive note
                </UiButton>
              ) : null}
              {activeNote && writeEnabled ? (
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onDeleteNote();
                  }}
                >
                  Delete note
                </UiButton>
              ) : null}
            </div>
          </div>
        </div>
      </header>

      {showScopeRow ? (
        <div className="topbar-mobile-meta" ref={scopeHostRef}>
          <button
            type="button"
            ref={scopeTriggerRef}
            className="topbar-scope-trigger"
            onClick={onToggleScopeSheet}
            aria-haspopup="dialog"
            aria-expanded={scopeSheetOpen}
          >
            <span className="topbar-scope-name">
              {scopeName(scope, vaults)}
            </span>
            <span className="topbar-scope-rule" aria-hidden="true">
              /
            </span>
            {showViewingMarker ? (
              <span className="topbar-scope-viewing">
                viewing {viewingVault?.name}
              </span>
            ) : null}
            <span className="topbar-scope-slot">{scopeSlot}</span>
          </button>

          {scopeSheetOpen ? (
            <div
              className="scope-sheet-backdrop"
              onClick={closeScopeSheetWithoutPicking}
              aria-hidden="true"
            />
          ) : null}
          <div
            ref={scopeSheetRef}
            className="scope-sheet"
            role="dialog"
            aria-modal="true"
            aria-label="Choose Vault scope"
            aria-hidden={!scopeSheetOpen}
            data-open={scopeSheetOpen}
          >
            <ul
              className="scope-sheet-list"
              role="radiogroup"
              aria-label="Vault scope"
            >
              <li>
                <button
                  type="button"
                  role="radio"
                  aria-checked={scope === "all"}
                  tabIndex={selectedScopeRowIndex === 0 ? 0 : -1}
                  ref={(el) => {
                    scopeRowRefs.current[0] = el;
                  }}
                  className={`scope-row${scope === "all" ? " is-selected" : ""}`}
                  onClick={() => pickScope("all")}
                  onKeyDown={(event) => {
                    if (event.key === "ArrowDown") {
                      event.preventDefault();
                      focusScopeRow(1);
                    } else if (event.key === "ArrowUp") {
                      event.preventDefault();
                      focusScopeRow(-1);
                    }
                  }}
                >
                  <span className="scope-row-label">All Vaults</span>
                  <VaultAggregateSlot
                    vaults={vaults}
                    counts={vaultNoteCounts}
                    demoMode={demoMode}
                  />
                </button>
              </li>
              {vaults.map((vault, index) => (
                <li key={vault.vault_id}>
                  <button
                    type="button"
                    role="radio"
                    aria-checked={scope === vault.vault_id}
                    tabIndex={selectedScopeRowIndex === index + 1 ? 0 : -1}
                    ref={(el) => {
                      scopeRowRefs.current[index + 1] = el;
                    }}
                    className={`scope-row${scope === vault.vault_id ? " is-selected" : ""}`}
                    onClick={() => pickScope(vault.vault_id)}
                    onKeyDown={(event) => {
                      if (event.key === "ArrowDown") {
                        event.preventDefault();
                        focusScopeRow(index + 2);
                      } else if (event.key === "ArrowUp") {
                        event.preventDefault();
                        focusScopeRow(index);
                      }
                    }}
                  >
                    <span className="scope-row-label">{vault.name}</span>
                    <VaultSlot
                      vault={vault}
                      noteCount={vaultNoteCounts[vault.vault_id]}
                      demoMode={demoMode}
                    />
                  </button>
                </li>
              ))}
            </ul>
          </div>
        </div>
      ) : null}
    </>
  );
}

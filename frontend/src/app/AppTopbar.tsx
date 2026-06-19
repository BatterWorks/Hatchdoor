import type { Ref } from "react";

import { StatusBadge, UiButton } from "../components/ui";
import type { ActiveNoteMeta } from "../types";
import type { Theme } from "./useTheme";

const THEME_ICON: Record<Theme, string> = { auto: "◑", light: "○", dark: "●" };
const THEME_LABEL: Record<Theme, string> = {
  auto: "Theme: System",
  light: "Theme: Light",
  dark: "Theme: Dark",
};

type TopbarProps = {
  activeNote: ActiveNoteMeta | null;
  writeEnabled: boolean;
  isMobile: boolean;
  isOnline: boolean;
  treeIsStale: boolean;
  actionsMenuOpen: boolean;
  topbarRef?: Ref<HTMLElement>;
  theme: Theme;
  onToggleDrawer: () => void;
  onOpenSearch: () => void;
  onToggleActionsMenu: () => void;
  onCloseActionsMenu: () => void;
  onRefreshVault: () => void;
  onCopyPageContent: () => void;
  onCopyNoteLink: () => void;
  onDownloadMarkdown: () => void;
  onEditNote: () => void;
  onNewNote: () => void;
  onRenameNote: () => void;
  onMoveNote: () => void;
  onDeleteNote: () => void;
  onToggleProperties: () => void;
  onCycleTheme: () => void;
};

export function AppTopbar({
  activeNote,
  writeEnabled,
  isMobile,
  isOnline,
  treeIsStale,
  actionsMenuOpen,
  topbarRef,
  theme,
  onToggleDrawer,
  onOpenSearch,
  onToggleActionsMenu,
  onCloseActionsMenu,
  onRefreshVault,
  onCopyPageContent,
  onCopyNoteLink,
  onDownloadMarkdown,
  onEditNote,
  onNewNote,
  onRenameNote,
  onMoveNote,
  onDeleteNote,
  onToggleProperties,
  onCycleTheme,
}: TopbarProps) {
  const crumbText = activeNote
    ? activeNote.relativePath.replace(/\//g, " / ")
    : "Notes Explorer";

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
              style={{ marginRight: "0.5rem" }}
            >
              ☰
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
            <text className="brand-wordmark-text" x="76" y="47" aria-hidden="true">
              HATCHDOOR
            </text>
          </svg>
        </div>

        {/* Col 2 — Breadcrumb */}
        <div className="topbar-crumb">
          <span className="topbar-crumb-here">{crumbText}</span>
          {!isOnline && (
            <span style={{ marginLeft: "0.5rem" }}>
              <StatusBadge tone="error" text="Offline" />
            </span>
          )}
          {treeIsStale && (
            <span style={{ marginLeft: "0.5rem" }}>
              <StatusBadge tone="warn" text="Tree Stale" />
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
              ⌕
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
          <div style={{ position: "relative" }}>
            <button
              type="button"
              className="icon-button"
              onClick={onToggleActionsMenu}
              aria-haspopup="menu"
              aria-expanded={actionsMenuOpen}
              aria-label="More actions"
            >
              ···
            </button>
            {actionsMenuOpen ? (
              <div className="topbar-menu" role="menu">
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onOpenSearch();
                  }}
                >
                  Search
                </UiButton>
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onRefreshVault();
                  }}
                >
                  Refresh vault
                </UiButton>
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
                <UiButton
                  className="close-note"
                  role="menuitem"
                  onClick={() => {
                    onCloseActionsMenu();
                    onToggleProperties();
                  }}
                >
                  Toggle properties
                </UiButton>
              </div>
            ) : null}
          </div>
        </div>
      </header>

      {isMobile ? (
        <div className="topbar-mobile-meta">
          <button
            type="button"
            className="topbar-mobile-path"
            onClick={onOpenSearch}
            title={
              activeNote ? `${activeNote.relativePath}.md` : "Notes Explorer"
            }
          >
            {activeNote ? `${activeNote.relativePath}.md` : "Notes Explorer"}
          </button>
        </div>
      ) : null}
    </>
  );
}

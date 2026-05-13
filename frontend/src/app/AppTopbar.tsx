import { StatusBadge, UiButton } from "../components/ui";
import type { ActiveNoteMeta } from "../types";

type TopbarProps = {
  activeNote: ActiveNoteMeta | null;
  isMobile: boolean;
  isOnline: boolean;
  treeIsStale: boolean;
  actionsMenuOpen: boolean;
  onToggleDrawer: () => void;
  onOpenSearch: () => void;
  onToggleActionsMenu: () => void;
  onCloseActionsMenu: () => void;
  onRefreshVault: () => void;
  onCopyPageContent: () => void;
  onCopyNoteLink: () => void;
  onDownloadMarkdown: () => void;
  onToggleProperties: () => void;
};

export function AppTopbar({
  activeNote,
  isMobile,
  isOnline,
  treeIsStale,
  actionsMenuOpen,
  onToggleDrawer,
  onOpenSearch,
  onToggleActionsMenu,
  onCloseActionsMenu,
  onRefreshVault,
  onCopyPageContent,
  onCopyNoteLink,
  onDownloadMarkdown,
  onToggleProperties,
}: TopbarProps) {
  return (
    <>
      <header className="app-topbar">
        <div className="topbar-left">
          {isMobile ? (
            <UiButton
              className="icon-button"
              onClick={onToggleDrawer}
              aria-label="Toggle explorer"
            >
              ☰
            </UiButton>
          ) : null}
          <div className="topbar-context">
            <h1>{activeNote?.title ?? "Hatchdoor"}</h1>
            <p className="topbar-subtitle">
              {activeNote ? `${activeNote.relativePath}.md` : "Notes Explorer"}
            </p>
          </div>
        </div>

        <div className="topbar-center">
          {!isMobile ? (
            <UiButton className="topbar-search-trigger" onClick={onOpenSearch}>
              Search
              <span className="shortcut-hint" aria-hidden="true">
                ⌘K
              </span>
            </UiButton>
          ) : null}
          {!isOnline ? <StatusBadge tone="error" text="Offline" /> : null}
          {treeIsStale ? <StatusBadge tone="warn" text="Tree Stale" /> : null}
        </div>

        <div className="topbar-right">
          {isMobile ? (
            <UiButton
              className="icon-button"
              onClick={onOpenSearch}
              aria-label="Search notes"
            >
              <span className="topbar-search-icon" aria-hidden="true">
                ⌕
              </span>
            </UiButton>
          ) : null}
          <UiButton
            className="icon-button"
            onClick={onToggleActionsMenu}
            aria-haspopup="menu"
            aria-expanded={actionsMenuOpen}
            aria-label="More actions"
          >
            ...
          </UiButton>
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

export const SIDEBAR_WIDTH_KEY = "hatchdoor.sidebarWidth";
export const DRAWER_OPEN_KEY = "hatchdoor.drawerOpen";
export const RECENT_NOTES_KEY = "hatchdoor.recentNotes";
export const EXPANDED_FOLDERS_KEY = "hatchdoor.expandedFolders";
export const EXPLORER_SCROLL_TOP_KEY = "hatchdoor.explorerScrollTop";
export const RECENT_NOTES_COLLAPSED_KEY = "hatchdoor.recentNotesCollapsed";
export const SCOPE_ZONE_COLLAPSED_KEY = "hatchdoor.scopeZoneCollapsed";
export const LAST_NOTE_KEY = "hatchdoor.lastNote";
export const NOTE_PROPERTIES_COLLAPSED_KEY =
  "hatchdoor.notePropertiesCollapsed";
export const THEME_KEY = "hatchdoor.theme";
// Selected Vault scope (one Vault ID, or "all"): state/URL/storage only per
// #137 — no Scope zone or other chrome reads or writes this yet.
export const VAULT_SCOPE_KEY = "hatchdoor.vaultScope";
// The explorer accordion's last-unfolded Vault under `all` (#142), persisted
// per browser the same way VAULT_SCOPE_KEY is.
export const LAST_UNFOLDED_VAULT_KEY = "hatchdoor.lastUnfoldedVault";
// Set once the one-time post-#137 browser-state cleanup
// (clearLegacyNoteScopedBrowserState) has run, so it never repeats and wipe
// state a returning user has legitimately rebuilt since (#151).
export const LEGACY_BROWSER_STATE_CLEARED_KEY =
  "hatchdoor.legacyBrowserStateCleared";

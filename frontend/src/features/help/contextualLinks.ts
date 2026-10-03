// Where each "How does this work?" link opens Help (#423). Every screen and
// condition that links to the manual reads its target from this one table,
// and `contextualLinks.test.ts` checks each page and heading against
// `docs/user-vault`, so renaming either fails the tests rather than leaving
// the reader on a page that is not there.

import type { VaultSummary } from "../../types";
import type { HelpLocation } from "./helpPages";

const TROUBLESHOOTING = "guides/how-to-troubleshoot-common-problems";
const MULTIPLE_VAULTS = "guides/how-to-manage-multiple-vaults";

/** The sync status code for a merge conflict (ADR-30). */
const SYNC_CONFLICT_CODE = "managed_git_conflict";

/** The activation codes for a Vault folder Hatchdoor cannot use. The same
 * two codes point agents at the same section (`src/mcp/docs_pointers.rs`). */
const FOLDER_CODES = new Set([
  "vault_path_unavailable",
  "vault_path_unreadable",
]);

export const CONTEXTUAL_HELP = {
  /** The "No Vaults Yet" screen. */
  noVaults: { page: "get-started/connect-your-first-vault" },
  /** The workspace could not load the Vault list. */
  vaultsUnavailable: {
    page: TROUBLESHOOTING,
    heading: "the-workspace-says-vaults-unavailable",
  },
  /** The registry recovery screen, either kind. */
  registryRecovery: {
    page: "concepts/vault-lifecycle-states",
    heading: "two-different-things-both-called-recovery",
  },
  /** The model-choice screen in `StartupGate`. */
  modelChoice: {
    page: "get-started/install-hatchdoor-with-docker-compose",
    heading: "choose-a-search-model",
  },
  /** Settings sections. */
  notesSettings: {
    page: "reference/settings-and-environment-variables-reference",
    heading: "live-settings",
  },
  agentSettings: { page: "get-started/connect-your-agent" },
  agentWrites: { page: "get-started/search-and-change-notes-with-your-agent" },
  uploadSettings: {
    page: "guides/how-to-import-and-work-with-attachments",
    heading: "what-may-be-uploaded",
  },
  /** A Vault's own Settings page while nothing is wrong. */
  vaultSettings: { page: MULTIPLE_VAULTS },
  /** Vault conditions. */
  vaultPaused: { page: MULTIPLE_VAULTS, heading: "pause-and-resume-a-vault" },
  vaultFolder: {
    page: TROUBLESHOOTING,
    heading: "permission-denied-reading-or-writing-the-vault",
  },
  vaultUnavailable: {
    page: TROUBLESHOOTING,
    heading: "a-vault-wont-index-or-stays-in-a-bad-state",
  },
  /** A Vault's Git console. */
  gitSetup: { page: "guides/how-to-set-up-a-git-backed-vault" },
  gitFailing: { page: TROUBLESHOOTING, heading: "git-sync-is-failing" },
  gitConflict: { page: TROUBLESHOOTING, heading: "resolving-a-sync-conflict" },
} as const satisfies Record<string, HelpLocation>;

/**
 * Where a Vault's condition line links, in the slot's own priority order
 * (`deriveVaultSlot`). A Git condition gets `null`: the Git console beside
 * that line already links to the matching page, see {@link gitConsoleHelp}.
 */
export function vaultConditionHelp(
  vault: VaultSummary,
  paused: boolean,
): HelpLocation | null {
  if (paused) {
    return CONTEXTUAL_HELP.vaultPaused;
  }
  if (vault.activation === "unavailable") {
    return FOLDER_CODES.has(vault.activation_error?.code ?? "")
      ? CONTEXTUAL_HELP.vaultFolder
      : CONTEXTUAL_HELP.vaultUnavailable;
  }
  if (vault.git === "unavailable") {
    return null;
  }
  if (vault.search === "stale" || vault.search_error) {
    return CONTEXTUAL_HELP.vaultUnavailable;
  }
  return CONTEXTUAL_HELP.vaultSettings;
}

/** Where a Vault's Git console links: the failure's own section, or the
 * Git-backed Vault guide while sync is healthy. */
export function gitConsoleHelp(vault: VaultSummary): HelpLocation {
  if (vault.git !== "unavailable") {
    return CONTEXTUAL_HELP.gitSetup;
  }
  return vault.git_error?.code === SYNC_CONFLICT_CODE
    ? CONTEXTUAL_HELP.gitConflict
    : CONTEXTUAL_HELP.gitFailing;
}

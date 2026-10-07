// Where each "How does this work?" link opens Help (#423). Every screen and
// condition that links to the manual reads its target from this one table,
// and `contextualLinks.test.ts` checks each page and heading against
// `docs/user-vault`, so renaming either fails the tests rather than leaving
// the reader on a page that is not there.
//
// Every link shows the same words, so each target also carries a `topic`: a
// few plain words naming what the link explains. `ContextualHelpLink` adds it
// to the link's accessible name, which is how a screen reader tells two links
// on one screen apart (#460). Write it as a noun phrase for someone who is not
// technical, and keep it different from every other topic in the table.

import type { VaultSummary } from "../../types";
import type { HelpLocation } from "./helpPages";

/** A place in the manual a "How does this work?" link opens, plus the words
 * that name it to a screen reader. */
export type ContextualHelp = HelpLocation & { topic: string };

/** What every contextual help link reads on screen. */
export const HELP_LINK_TEXT = "How does this work?";

/** A help link's accessible name: the words on screen, then the topic. The
 * visible words come first so voice control can still be told what is on
 * screen (WCAG 2.5.3, Label in Name). */
export function helpLinkName(to: ContextualHelp): string {
  return `${HELP_LINK_TEXT} ${to.topic}`;
}

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
  noVaults: {
    page: "get-started/connect-your-first-vault",
    topic: "Adding your notes",
  },
  /** Add a Vault's folder picker: a folder outside the Vault mount (#430). */
  folderOutsideMount: {
    page: "get-started/connect-your-first-vault",
    heading: "add-a-folder-hatchdoor-cannot-see-yet",
    topic: "Adding a folder Hatchdoor cannot see",
  },
  /** The folder picker's "New folder": a folder Hatchdoor could not make
   * (#494). */
  newFolderRefused: {
    page: "get-started/connect-your-first-vault",
    heading: "start-with-a-new-empty-folder",
    topic: "Making a new folder",
  },
  /** The workspace could not load the Vault list. */
  vaultsUnavailable: {
    page: TROUBLESHOOTING,
    heading: "the-workspace-says-vaults-unavailable",
    topic: "The Vaults Unavailable message",
  },
  /** The registry recovery screen. */
  registryRecovery: {
    page: "concepts/vault-lifecycle-states",
    heading: "registry-recovery",
    topic: "A damaged list of Vaults",
  },
  /** The model-choice screen in `StartupGate`. */
  modelChoice: {
    page: "get-started/install-hatchdoor-with-docker-compose",
    heading: "choose-a-search-model",
    topic: "Choosing a search model",
  },
  /** Settings sections. */
  notesSettings: {
    page: "reference/settings-and-environment-variables-reference",
    heading: "live-settings",
    topic: "Changing settings without a restart",
  },
  agentSettings: {
    page: "get-started/connect-your-agent",
    topic: "Connecting your agent",
  },
  agentWrites: {
    page: "get-started/search-and-change-notes-with-your-agent",
    topic: "Letting your agent change notes",
  },
  uploadSettings: {
    page: "guides/how-to-import-and-work-with-attachments",
    heading: "what-may-be-uploaded",
    topic: "Which files can be uploaded",
  },
  /** Settings' Updates section, its update check switch, and the update
   * banner's "How to upgrade" (#425). */
  upgrade: {
    page: "guides/how-to-upgrade-hatchdoor",
    topic: "Upgrading Hatchdoor",
  },
  updateCheck: {
    page: "guides/how-to-upgrade-hatchdoor",
    heading: "hear-about-new-releases",
    topic: "Hearing about new releases",
  },
  /** A Vault's own Settings page while nothing is wrong. */
  vaultSettings: { page: MULTIPLE_VAULTS, topic: "Managing your Vaults" },
  /** Vault conditions. */
  vaultPaused: {
    page: MULTIPLE_VAULTS,
    heading: "pause-and-resume-a-vault",
    topic: "Pausing and resuming a Vault",
  },
  vaultFolder: {
    page: TROUBLESHOOTING,
    heading: "permission-denied-reading-or-writing-the-vault",
    topic: "A Vault folder Hatchdoor cannot open",
  },
  vaultUnavailable: {
    page: TROUBLESHOOTING,
    heading: "a-vault-wont-index-or-stays-in-a-bad-state",
    topic: "A Vault that is not working",
  },
  /** A Vault's Git console. */
  gitSetup: {
    page: "guides/how-to-set-up-a-git-backed-vault",
    topic: "Setting up a Git-backed Vault",
  },
  gitFailing: {
    page: TROUBLESHOOTING,
    heading: "git-sync-is-failing",
    topic: "Git sync that is failing",
  },
  gitConflict: {
    page: TROUBLESHOOTING,
    heading: "resolving-a-sync-conflict",
    topic: "Resolving a sync conflict",
  },
} as const satisfies Record<string, ContextualHelp>;

/**
 * Where a Vault's condition line links, in the slot's own priority order
 * (`deriveVaultSlot`). A Git condition gets `null`: the Git console beside
 * that line already links to the matching page, see {@link gitConsoleHelp}.
 */
export function vaultConditionHelp(
  vault: VaultSummary,
  paused: boolean,
): ContextualHelp | null {
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
export function gitConsoleHelp(vault: VaultSummary): ContextualHelp {
  if (vault.git !== "unavailable") {
    return CONTEXTUAL_HELP.gitSetup;
  }
  return vault.git_error?.code === SYNC_CONFLICT_CODE
    ? CONTEXTUAL_HELP.gitConflict
    : CONTEXTUAL_HELP.gitFailing;
}

/**
 * Pure Git-behaviour logic for a Vault's settings page (issue #149), plus the
 * client-only "changed but did not restart" recovery marker (issue #121).
 * Split out of `VaultSettingsIndex.tsx` because a file that exports anything
 * besides React components breaks Fast Refresh
 * (`react-refresh/only-export-components`).
 */

import { apiFetch } from "../../api/api";
import type {
  VaultDiscoveryResponse,
  VaultId,
  VaultRuntimeError,
  VaultRuntimeErrorDetail,
  VaultSource,
  VaultSummary,
} from "../../types";

/** `apiFetch` throws on a network failure, a timeout, or an aborted request
 * — not just on a non-2xx response. The identity round trip below needs to
 * tell "the server refused this step" and "this step never reached the
 * server" apart from every OTHER step's perspective (so it knows whether to
 * roll back or leave the Vault's already-changed state alone), which is only
 * possible if a network failure is turned into an ordinary `ok: false`
 * result rather than an uncaught rejection that would otherwise skip the
 * rest of the round trip's rollback/recovery logic entirely. */
export async function requestJson(
  path: string,
  init?: RequestInit,
): Promise<{ ok: boolean; payload: Record<string, unknown> }> {
  try {
    const response = await apiFetch(path, init);
    const payload = (await response.json().catch(() => ({}))) as Record<
      string,
      unknown
    >;
    return { ok: response.ok, payload };
  } catch {
    return {
      ok: false,
      payload: {
        message: "Could not reach the server. Check the connection and try again.",
      },
    };
  }
}

/** The choice this page's segmented control offers, independent of
 * `VaultGitMode`: `no_git` is Local Git behaviour, not a `VaultSource` type
 * of its own. */
export type GitBehavior = "no_git" | "local_history" | "pull_only" | "two_way";

export const DEFAULT_POLL_MINUTES = 1440;
export const MAX_POLL_MINUTES = 1440;
export const MIN_POLL_MINUTES = 1;

export function sourceLabel(source: VaultSource | undefined): string {
  if (!source) return "Vault source";
  if (source.type === "local") return "A folder on this server";
  if (source.type === "existing_git") return "An existing Git folder";
  return "A managed Git checkout";
}

/** Issue #121: what the record stores is two facts (where the folder came
 * from, and what Git does with it), not one four-valued field. This reads
 * the second fact back off a source. */
export function behaviorOf(source: VaultSource): GitBehavior {
  if (source.type === "local") return "no_git";
  return source.mode;
}

/** The behaviours legal for this Vault: four on a folder you own (`local`/
 * `existing_git`), two on one Hatchdoor cloned (`managed_git`) — a cloned
 * folder always has a remote, so the registry rejects `local_history` for it
 * (`normalize_structural_source` in `src/vault_registry.rs`). Illegal
 * options are absent, not greyed (issue #121). */
export function behaviorOptions(
  source: VaultSource,
): { id: GitBehavior; label: string }[] {
  const remote = [
    { id: "pull_only" as const, label: "Pull-only" },
    { id: "two_way" as const, label: "Two-way" },
  ];
  if (source.type === "managed_git") return remote;
  return [
    { id: "no_git" as const, label: "No Git" },
    { id: "local_history" as const, label: "Local history" },
    ...remote,
  ];
}

/** Rebuilds `source` for a chosen behaviour. Moving among the three Git
 * behaviours on the same folder (or between Pull-only and Two-way on a
 * cloned one) only ever swaps `mode`. Moving between No Git and any Git
 * behaviour swaps the source `type` itself — the one boundary the registry
 * treats as a different Vault (`same_source_identity`,
 * `src/vault_registry.rs`) — carrying the folder's own location across as
 * the new type's identifying field. */
export function buildSourceForBehavior(
  current: VaultSource,
  behavior: GitBehavior,
): VaultSource {
  if (current.type === "managed_git") {
    return behavior === "pull_only" || behavior === "two_way"
      ? { ...current, mode: behavior }
      : current;
  }
  if (behavior === "no_git") {
    const path =
      current.type === "local" ? current.path : current.repository_path;
    return { type: "local", path };
  }
  if (current.type === "local") {
    return {
      type: "existing_git",
      repository_path: current.path,
      repository_url: undefined,
      branch: undefined,
      vault_subdirectory: undefined,
      mode: behavior,
      poll_interval_secs: DEFAULT_POLL_MINUTES * 60,
    };
  }
  return { ...current, mode: behavior };
}

/** The three identity-bearing fields the plaque's affordance opens into
 * fields, applied to whichever source `buildSourceForBehavior` produced.
 * `repository_path` (an existing-Git Vault's own folder) is never edited
 * here — it is the identity fact the plaque states, not a choice. */
export function withIdentityFields(
  base: VaultSource,
  fields: {
    repositoryUrl: string;
    branch: string;
    subdirectory: string;
    pollMinutes: number;
  },
): VaultSource {
  if (base.type === "local") return base;
  const branch = fields.branch.trim() || undefined;
  const vault_subdirectory = fields.subdirectory.trim() || undefined;
  const poll_interval_secs = clampPollMinutes(String(fields.pollMinutes)) * 60;
  if (base.type === "managed_git") {
    return {
      ...base,
      repository_url: fields.repositoryUrl.trim(),
      branch,
      vault_subdirectory,
      poll_interval_secs,
    };
  }
  if (base.mode === "local_history") return base;
  return {
    ...base,
    repository_url: fields.repositoryUrl.trim() || undefined,
    branch,
    vault_subdirectory,
    poll_interval_secs,
  };
}

/** The fields `src/vault_registry.rs`'s `same_source_identity` compares —
 * everything but `mode` and `poll_interval_secs`. Two sources with the same
 * identity never need the Vault paused or a confirmation to move between. */
function sourceIdentity(source: VaultSource): readonly unknown[] {
  if (source.type === "local") return ["local", source.path];
  if (source.type === "existing_git")
    return [
      "existing_git",
      source.repository_path,
      source.repository_url ?? null,
      source.branch ?? null,
      source.vault_subdirectory ?? null,
    ];
  return [
    "managed_git",
    source.repository_url,
    source.branch ?? null,
    source.vault_subdirectory ?? null,
  ];
}

export function sameSourceIdentity(a: VaultSource, b: VaultSource): boolean {
  const first = sourceIdentity(a);
  const second = sourceIdentity(b);
  return (
    first.length === second.length &&
    first.every((value, index) => value === second[index])
  );
}

/** Whether the chosen behaviour talks to a remote at all — the schedule,
 * sign-in and sync console only apply then (issue #121: "the schedule and
 * sync stop depending on which kind of folder a Vault is"). */
export function isRemoteBacked(behavior: GitBehavior | null): boolean {
  return behavior === "pull_only" || behavior === "two_way";
}

export function clampPollMinutes(raw: string): number {
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed)) return DEFAULT_POLL_MINUTES;
  return Math.min(MAX_POLL_MINUTES, Math.max(MIN_POLL_MINUTES, parsed));
}

export type GitFailureTier = "warn" | "error";
export type GitFailureDescription = {
  label: string;
  tier: GitFailureTier;
  sentence: string;
  files?: string[];
  filesTotal?: number;
};

/** Issue #121's nine failures, verbatim from its resolution: "the
 * destination is invalid, the remote is unreachable, authentication failed,
 * validation failed, the install failed, the working copy is dirty, a
 * pull-only checkout has local commits, there is a content conflict, and a
 * push race was exhausted." Each sentence says what happened, what was not
 * lost, and the single thing that clears it — this page owns the words, the
 * server sends only the code (matching this page's own reindex/git-init
 * confirmation copy in `SettingsPage.tsx`). */
const GIT_FAILURE_COPY: Record<
  string,
  {
    label: string;
    tier: GitFailureTier;
    files: boolean;
    sentence: (detail: VaultRuntimeErrorDetail | undefined) => string;
  }
> = {
  managed_git_destination_invalid: {
    label: "invalid destination",
    tier: "error",
    files: false,
    sentence: () =>
      "The folder or subfolder configured for this Vault is not usable. Nothing in this Vault was touched. Fix the repository, branch or folder above, then press Try again.",
  },
  managed_git_remote_unreachable: {
    label: "remote unreachable",
    tier: "warn",
    files: false,
    sentence: () =>
      "Hatchdoor could not reach this Vault's remote. Nothing was lost — this often clears on its own. Press Try again.",
  },
  managed_git_authentication_failed: {
    label: "sign-in failed",
    tier: "error",
    files: false,
    sentence: () =>
      "This Vault's remote rejected its sign-in. Nothing was lost. Save a new access token below, then press Try again.",
  },
  managed_git_validation_failed: {
    label: "validation failed",
    tier: "error",
    files: false,
    sentence: () =>
      "This Vault's Git setup failed a check before anything was touched. Nothing was lost. Review the repository, branch and folder above, then press Try again.",
  },
  managed_git_install_failed: {
    label: "install failed",
    tier: "error",
    files: false,
    sentence: () =>
      "Hatchdoor could not put this Vault's freshly cloned files in place. Nothing already in this Vault was touched. Press Try again.",
  },
  managed_git_dirty_working_copy: {
    label: "local edits",
    tier: "error",
    files: true,
    sentence: () =>
      "The files below are edited locally in a way Hatchdoor is not sure how to reconcile. Nothing was lost — the edits are still there. Deal with the listed files, then press Try again.",
  },
  managed_git_pull_only_local_commits: {
    label: "unpushed commits",
    tier: "warn",
    files: false,
    sentence: (detail) => {
      const ahead =
        detail?.kind === "local_commits_ahead" ? detail.ahead : undefined;
      const count =
        ahead === undefined
          ? "Local commits"
          : `${ahead} local commit${ahead === 1 ? "" : "s"}`;
      return `This pull-only Vault has ${count} Hatchdoor is not allowed to push. Nothing was lost. Push or discard them outside Hatchdoor, then press Try again.`;
    },
  },
  managed_git_conflict: {
    label: "conflict",
    tier: "error",
    files: true,
    sentence: () =>
      "The files below conflict between this Vault and its remote. Nothing was lost — both versions still exist. Resolve the listed files, then press Try again.",
  },
  managed_git_push_race_exhausted: {
    label: "push race",
    tier: "warn",
    files: false,
    sentence: () =>
      "Too many pushes landed on this Vault's remote at once and Hatchdoor gave up retrying. Nothing was lost. Press Try again.",
  },
};

export function describeGitFailure(
  error: VaultRuntimeError,
): GitFailureDescription {
  const copy = GIT_FAILURE_COPY[error.code];
  if (!copy) {
    return {
      label: "sync failed",
      tier: "error",
      sentence: `Something unexpected stopped this Vault's sync: ${error.message} Press Try again.`,
    };
  }
  const description: GitFailureDescription = {
    label: copy.label,
    tier: copy.tier,
    sentence: copy.sentence(error.detail),
  };
  if (copy.files && error.detail?.kind === "affected_paths") {
    description.files = error.detail.paths;
    description.filesTotal = error.detail.total;
  }
  return description;
}

/** The one condition allowed to persist across visits (issue #121): a Vault
 * whose final un-pause failed after an identity edit already went through is
 * changed and invisible until someone brings it back. There is no backend
 * flag for this — the registry has no concept of "wanted enabled but
 * couldn't" — so this page remembers it itself. */
const RECOVERY_KEY_PREFIX = "hatchdoor:vault-recovery:";

function recoveryKey(vaultId: VaultId): string {
  return `${RECOVERY_KEY_PREFIX}${vaultId}`;
}

export function markRecoveryPending(vaultId: VaultId): void {
  try {
    localStorage.setItem(recoveryKey(vaultId), "1");
  } catch {
    // Ignore storage failures — the in-memory state still drives this visit.
  }
}

export function clearRecoveryPending(vaultId: VaultId): void {
  try {
    localStorage.removeItem(recoveryKey(vaultId));
  } catch {
    // Ignore.
  }
}

export function isRecoveryPending(vaultId: VaultId): boolean {
  try {
    return localStorage.getItem(recoveryKey(vaultId)) === "1";
  } catch {
    return false;
  }
}

/** The single recovery action: re-enable with whatever revision is current
 * right now, since the marker may be read on a visit long after the failure.
 * Shared by the index's management entry and a Vault's own page. */
export async function recoverPausedVault(
  vaultId: VaultId,
): Promise<{ ok: boolean; message: string; vault?: VaultSummary }> {
  const discovery = await requestJson("/api/v1/vaults");
  const registryRevision = (discovery.payload as VaultDiscoveryResponse)
    .registry_revision;
  if (!discovery.ok || registryRevision === undefined)
    return {
      ok: false,
      message: "Could not check this Vault's status. Try again.",
    };
  const result = await requestJson(
    `/api/v1/vaults/${vaultId}/enable?expected_registry_revision=${registryRevision}`,
    { method: "POST" },
  );
  const payload = result.payload as { vault?: VaultSummary; message?: string };
  if (!result.ok)
    return {
      ok: false,
      message: payload.message ?? "This Vault is still paused. Try again.",
    };
  clearRecoveryPending(vaultId);
  return { ok: true, message: "This Vault is back.", vault: payload.vault };
}

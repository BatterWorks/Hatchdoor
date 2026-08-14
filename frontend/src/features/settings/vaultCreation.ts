/**
 * Pure logic and the API call for the browser Vault-creation flow (issue
 * #153). Split out of `VaultCreation.tsx` for the same reason
 * `vaultGitBehavior.ts` is split from `VaultSettingsIndex.tsx`: a file that
 * exports anything besides React components breaks Fast Refresh
 * (`react-refresh/only-export-components`).
 */

import type { VaultSource, VaultSummary } from "../../types";
import {
  DEFAULT_POLL_MINUTES,
  missingRequiredRepositoryUrl,
  REPOSITORY_URL_REQUIRED_MESSAGE,
  requestJson,
} from "./vaultGitBehavior";

/** The top-level creation choice: a folder this server already has on disk
 * (which `buildSourceForBehavior` — reused unchanged from the edit flow —
 * turns into `local` or `existing_git` depending on the chosen Git
 * behaviour), or a fresh clone Hatchdoor manages itself. */
export type CreateVaultKind = "own" | "managed";

/** The starting point `buildSourceForBehavior`/`withIdentityFields` (both
 * reused from `vaultGitBehavior.ts`) then transform as the form's behaviour
 * and identity fields are edited — the same two-step composition the edit
 * flow already uses, just starting from an empty source instead of an
 * existing Vault's. */
export function baseSourceForKind(
  kind: CreateVaultKind,
  path: string,
): VaultSource {
  if (kind === "managed") {
    return {
      type: "managed_git",
      repository_url: "",
      branch: undefined,
      vault_subdirectory: undefined,
      mode: "pull_only",
      poll_interval_secs: DEFAULT_POLL_MINUTES * 60,
    };
  }
  return { type: "local", path: path.trim() };
}

/** The one field each source kind cannot be created without. Everything else
 * `POST /api/v1/vaults` accepts is optional or already defaulted. The
 * remote-URL rule is shared with `VaultSettingsDetail.handleSave`'s
 * equivalent edit-flow guard via `missingRequiredRepositoryUrl`, rather than
 * duplicated, since the backend enforces one rule for both. */
export function validateCreateSource(source: VaultSource): string | null {
  if (source.type === "local")
    return source.path.trim() ? null : "Enter the folder path.";
  if (source.type === "existing_git" && !source.repository_path.trim())
    return "Enter the folder path.";
  if (missingRequiredRepositoryUrl(source))
    return REPOSITORY_URL_REQUIRED_MESSAGE;
  return null;
}

export type CreateVaultResult =
  | { ok: true; vault: VaultSummary }
  | { ok: false; code?: string; message?: string };

/** `POST /api/v1/vaults` — the existing frozen contract (issue #101),
 * unmodified. `credentials` is omitted entirely rather than sent empty, so a
 * Vault created with no sign-in has `credential_configured: false` rather
 * than a stored blank token. */
export async function createVault(params: {
  expectedRegistryRevision: number;
  name: string;
  source: VaultSource;
  credentials?: { token: string };
}): Promise<CreateVaultResult> {
  const body: Record<string, unknown> = {
    expected_registry_revision: params.expectedRegistryRevision,
    name: params.name,
    source: params.source,
  };
  if (params.credentials) body.https_credentials = params.credentials;
  const { ok, payload } = await requestJson("/api/v1/vaults", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const typed = payload as {
    vault?: VaultSummary;
    code?: string;
    message?: string;
  };
  if (!ok || !typed.vault)
    return { ok: false, code: typed.code, message: typed.message };
  return { ok: true, vault: typed.vault };
}

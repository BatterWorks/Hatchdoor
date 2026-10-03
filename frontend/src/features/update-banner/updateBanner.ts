// What the update banner (#425) offers: the newer release the opt-in update
// check found (ADR-39), as `GET /api/settings` reports it, less a version this
// browser already dismissed.

import { apiFetch } from "../../api/api";
import { safeGetItem, safeSetItem } from "../../lib/storage";

/** The last version this browser dismissed the banner for. */
export const DISMISSED_VERSION_KEY = "hatchdoor_update_dismissed";

export type AvailableRelease = {
  /** A plain version such as `2.9.0`. */
  version: string;
  /** Its release page on GitHub. */
  release_url: string;
};

/** The `update_check` part of the settings response. */
export type UpdateCheckStatus = {
  enabled: boolean;
  checked_at: string | null;
  /** `null` when the check is off, or found nothing newer. */
  update_available: AvailableRelease | null;
};

/** The release to show, or `null` when there is none or it was dismissed
 * in this browser. */
export function releaseToShow(
  status: UpdateCheckStatus | null,
  dismissed: string | null,
): AvailableRelease | null {
  const release = status?.update_available;
  if (!release || release.version === dismissed) {
    return null;
  }
  return release;
}

export function readDismissed(): string | null {
  return safeGetItem(DISMISSED_VERSION_KEY);
}

/** Remember the dismissal for `version` only: a later release shows again.
 * Blocked storage forgets it at the next load, which is the safe side. */
export function markDismissed(version: string): void {
  safeSetItem(DISMISSED_VERSION_KEY, version);
}

/** The server's status, or `null` when it could not be read. */
export async function fetchUpdateStatus(
  signal?: AbortSignal,
): Promise<UpdateCheckStatus | null> {
  try {
    const response = await apiFetch("/api/settings", { signal });
    if (!response.ok) {
      return null;
    }
    const payload = (await response.json()) as {
      update_check?: UpdateCheckStatus;
    };
    return payload.update_check ?? null;
  } catch {
    return null;
  }
}

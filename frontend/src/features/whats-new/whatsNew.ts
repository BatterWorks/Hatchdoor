// What the What's new pop-up (#418) shows: the release highlights from
// `GET /api/v1/whats-new` (ADR-42), less whatever this browser already saw.

import { apiFetch } from "../../api/api";
import { safeSetItem } from "../../lib/storage";

/** The last version this browser showed What's new for, as a base version. */
export const SEEN_VERSION_KEY = "hatchdoor_whats_new_seen";

export type HighlightLink = {
  label: string;
  /** A manual page name, as Help and `read_docs` take it. */
  page: string;
  /** A heading anchor in the note slug rule. */
  heading: string | null;
};

export type Highlight = {
  /** Markdown, without its action-needed marker or its link. */
  text: string;
  action_needed: boolean;
  link: HighlightLink | null;
};

export type Release = {
  version: string;
  /** `YYYY-MM-DD`. */
  date: string;
  highlights: Highlight[];
};

export type WhatsNewResponse = {
  /** The running version, with ` (dev <commit>)` on a development build. */
  version: string;
  previous_version: string | null;
  fresh_install: boolean;
  /** Newest first, after `previous_version` up to the running version. */
  releases: Release[];
};

/** What this browser remembers. `available` is false when storage throws. */
export type SeenRecord = { available: boolean; seen: string | null };

/** `2.8.0 (dev abc123)` → `2.8.0`. */
export function baseVersion(version: string): string {
  const dev = version.indexOf(" (dev");
  return (dev === -1 ? version : version.slice(0, dev)).trim();
}

/** `major.minor.patch` as numbers. */
type Semver = [number, number, number];

function parseVersion(version: string): Semver | null {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null;
}

function compareVersions(a: Semver, b: Semver): number {
  return a[0] - b[0] || a[1] - b[1] || a[2] - b[2];
}

/**
 * The releases to show, newest first, or none. Nothing shows on a fresh
 * install, or when storage is blocked: a dismissal that cannot be remembered
 * would bring the pop-up back on every load. With no record the server's own
 * list stands, which starts after the version it ran before.
 */
export function releasesToShow(
  response: WhatsNewResponse,
  record: SeenRecord,
): Release[] {
  if (response.fresh_install || !record.available) {
    return [];
  }
  const seen = record.seen ? parseVersion(record.seen) : null;
  return response.releases
    .flatMap((release) => {
      const version = parseVersion(release.version);
      if (!version || (seen && compareVersions(version, seen) <= 0)) {
        return [];
      }
      return [{ release, version }];
    })
    .sort((a, b) => compareVersions(b.version, a.version))
    .map(({ release }) => release);
}

export function readSeen(): SeenRecord {
  try {
    return {
      available: true,
      seen: window.localStorage.getItem(SEEN_VERSION_KEY),
    };
  } catch {
    return { available: false, seen: null };
  }
}

/** Remember that this browser has seen What's new for `version`. */
export function markSeen(version: string): void {
  safeSetItem(SEEN_VERSION_KEY, baseVersion(version));
}

/** The server's answer, or `null` when it could not be read. */
export async function fetchWhatsNew(
  signal?: AbortSignal,
): Promise<WhatsNewResponse | null> {
  try {
    const response = await apiFetch("/api/v1/whats-new", { signal });
    if (!response.ok) {
      return null;
    }
    return (await response.json()) as WhatsNewResponse;
  } catch {
    return null;
  }
}

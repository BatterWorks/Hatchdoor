// Whether the sidebar shows Changed on disk and Recently viewed (#684). Two
// instance settings, on unless the operator turns them off. The shell reads
// them here and hides the lists in the sidebar and on Home; what feeds the
// lists keeps running, so a list switched back on is current.

import { useEffect, useState } from "react";
import { apiFetch } from "../../api/api";
import { safeGetItem, safeSetItem } from "../../lib/storage";
import { SETTINGS_SAVED_EVENT } from "./settingsApi";

const SIDEBAR_CHANGED_ON_DISK_SETTING = "HATCHDOOR_SIDEBAR_CHANGED_ON_DISK";
const SIDEBAR_RECENTLY_VIEWED_SETTING = "HATCHDOOR_SIDEBAR_RECENTLY_VIEWED";

/** The last answer this browser had, so a reload does not draw a list the
 * operator turned off and then take it away. A copy of the server's value,
 * not a preference of the browser's. */
const CACHE_KEY = "hatchdoor.sidebarSections";

export type SidebarSections = {
  changedOnDisk: boolean;
  recentlyViewed: boolean;
};

export const ALL_SIDEBAR_SECTIONS: SidebarSections = {
  changedOnDisk: true,
  recentlyViewed: true,
};

type SettingsPayload = {
  settings?: { key: string; value: string | null }[];
};

// Anything but the server's own "false" leaves a list on: a setting the
// answer leaves out, or one it cannot read, never hides one.
function shown(payload: SettingsPayload, key: string): boolean {
  const value = payload.settings?.find((item) => item.key === key)?.value;
  return (value ?? "").trim().toLowerCase() !== "false";
}

/** The two switches as `GET /api/settings` reports them. */
export function sidebarSectionsFrom(payload: SettingsPayload): SidebarSections {
  return {
    changedOnDisk: shown(payload, SIDEBAR_CHANGED_ON_DISK_SETTING),
    recentlyViewed: shown(payload, SIDEBAR_RECENTLY_VIEWED_SETTING),
  };
}

function cached(): SidebarSections {
  try {
    const raw = safeGetItem(CACHE_KEY);
    if (!raw) return ALL_SIDEBAR_SECTIONS;
    const parsed = JSON.parse(raw) as Partial<SidebarSections>;
    return {
      changedOnDisk: parsed.changedOnDisk !== false,
      recentlyViewed: parsed.recentlyViewed !== false,
    };
  } catch {
    return ALL_SIDEBAR_SECTIONS;
  }
}

/**
 * Which of the two lists to show. Read once Settings exist (`settingsEnabled`,
 * the shell's own flag), again after any save of settings from this tab, and
 * whenever the tab comes back into view, so a change made elsewhere arrives
 * without a reload. A demo instance has no settings, so it shows both.
 */
export function useSidebarSections({
  settingsEnabled,
  demoMode,
}: {
  settingsEnabled: boolean;
  demoMode: boolean;
}): SidebarSections {
  const [sections, setSections] = useState<SidebarSections>(cached);

  useEffect(() => {
    if (!settingsEnabled) return;
    let controller: AbortController | null = null;
    const read = () => {
      controller?.abort();
      const current = new AbortController();
      controller = current;
      void apiFetch("/api/settings", { signal: current.signal })
        .then(async (response) => {
          if (!response.ok) return;
          const next = sidebarSectionsFrom(
            (await response.json()) as SettingsPayload,
          );
          if (current.signal.aborted) return;
          safeSetItem(CACHE_KEY, JSON.stringify(next));
          setSections((prev) =>
            prev.changedOnDisk === next.changedOnDisk &&
            prev.recentlyViewed === next.recentlyViewed
              ? prev
              : next,
          );
        })
        .catch(() => {
          // Unreadable: keep what is shown.
        });
    };
    const onVisible = () => {
      if (document.visibilityState === "visible") read();
    };
    read();
    window.addEventListener(SETTINGS_SAVED_EVENT, read);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      controller?.abort();
      window.removeEventListener(SETTINGS_SAVED_EVENT, read);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [settingsEnabled]);

  return demoMode ? ALL_SIDEBAR_SECTIONS : sections;
}

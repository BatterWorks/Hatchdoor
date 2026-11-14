// The Layout section's instance settings, read by the shell: whether the
// sidebar shows Changed on disk and Recently viewed (#684), and how wide a
// note's text may run (#698). The operator sets them once for everyone. The
// sidebar lists hide in the sidebar and on Home while what feeds them keeps
// running, so a list switched back on is current.

import { useEffect, useState } from "react";
import { apiFetch } from "../../api/api";
import { safeGetItem, safeSetItem } from "../../lib/storage";
import { SETTINGS_SAVED_EVENT } from "./settingsApi";

const SIDEBAR_CHANGED_ON_DISK_SETTING = "HATCHDOOR_SIDEBAR_CHANGED_ON_DISK";
const SIDEBAR_RECENTLY_VIEWED_SETTING = "HATCHDOOR_SIDEBAR_RECENTLY_VIEWED";
const NOTE_WIDTH_SETTING = "HATCHDOOR_NOTE_WIDTH";

/** The last answer this browser had, so a reload does not draw a list the
 * operator turned off and then take it away, or a note at one width and then
 * another. Copies of the server's values, not preferences of the browser's. */
const SECTIONS_CACHE_KEY = "hatchdoor.sidebarSections";
const NOTE_WIDTH_CACHE_KEY = "hatchdoor.noteWidth";

export type SidebarSections = {
  changedOnDisk: boolean;
  recentlyViewed: boolean;
};

export const ALL_SIDEBAR_SECTIONS: SidebarSections = {
  changedOnDisk: true,
  recentlyViewed: true,
};

/** The cap on a note's text: the reading measure, a longer line, or none.
 * The values are the setting's own (`HATCHDOOR_NOTE_WIDTH`). */
export const NOTE_WIDTHS = ["default", "wide", "full"] as const;
export type NoteWidth = (typeof NOTE_WIDTHS)[number];
export const DEFAULT_NOTE_WIDTH: NoteWidth = "default";

export type LayoutSettings = {
  sidebarSections: SidebarSections;
  noteWidth: NoteWidth;
};

const DEFAULT_LAYOUT: LayoutSettings = {
  sidebarSections: ALL_SIDEBAR_SECTIONS,
  noteWidth: DEFAULT_NOTE_WIDTH,
};

type SettingsPayload = {
  settings?: { key: string; value: string | null }[];
};

function valueOf(payload: SettingsPayload, key: string): string {
  return payload.settings?.find((item) => item.key === key)?.value ?? "";
}

// Anything but the server's own "false" leaves a list on: a setting the
// answer leaves out, or one it cannot read, never hides one.
function shown(payload: SettingsPayload, key: string): boolean {
  return valueOf(payload, key).trim().toLowerCase() !== "false";
}

/** Anything but one of the three widths is the default. */
export function asNoteWidth(value: unknown): NoteWidth {
  return NOTE_WIDTHS.find((width) => width === value) ?? DEFAULT_NOTE_WIDTH;
}

/** The two switches as `GET /api/settings` reports them. */
export function sidebarSectionsFrom(payload: SettingsPayload): SidebarSections {
  return {
    changedOnDisk: shown(payload, SIDEBAR_CHANGED_ON_DISK_SETTING),
    recentlyViewed: shown(payload, SIDEBAR_RECENTLY_VIEWED_SETTING),
  };
}

/** The note width as `GET /api/settings` reports it. */
export function noteWidthFrom(payload: SettingsPayload): NoteWidth {
  return asNoteWidth(valueOf(payload, NOTE_WIDTH_SETTING));
}

function cached(): LayoutSettings {
  let sidebarSections = ALL_SIDEBAR_SECTIONS;
  try {
    const raw = safeGetItem(SECTIONS_CACHE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<SidebarSections>;
      sidebarSections = {
        changedOnDisk: parsed.changedOnDisk !== false,
        recentlyViewed: parsed.recentlyViewed !== false,
      };
    }
  } catch {
    // Unreadable: show both.
  }
  return {
    sidebarSections,
    noteWidth: asNoteWidth(safeGetItem(NOTE_WIDTH_CACHE_KEY)),
  };
}

function sameSections(a: SidebarSections, b: SidebarSections): boolean {
  return (
    a.changedOnDisk === b.changedOnDisk && a.recentlyViewed === b.recentlyViewed
  );
}

/**
 * The Layout settings. Read once Settings exist (`settingsEnabled`, the
 * shell's own flag), again after any save of settings from this tab, and
 * whenever the tab comes back into view, so a change made elsewhere arrives
 * without a reload. A demo instance has no settings, so it shows both lists
 * and the default width.
 */
export function useLayoutSettings({
  settingsEnabled,
  demoMode,
}: {
  settingsEnabled: boolean;
  demoMode: boolean;
}): LayoutSettings {
  const [layout, setLayout] = useState<LayoutSettings>(cached);

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
          const payload = (await response.json()) as SettingsPayload;
          if (current.signal.aborted) return;
          const sidebarSections = sidebarSectionsFrom(payload);
          const noteWidth = noteWidthFrom(payload);
          safeSetItem(SECTIONS_CACHE_KEY, JSON.stringify(sidebarSections));
          safeSetItem(NOTE_WIDTH_CACHE_KEY, noteWidth);
          // Unchanged parts keep their identity, so nothing redraws for them.
          setLayout((prev) => {
            const sections = sameSections(prev.sidebarSections, sidebarSections)
              ? prev.sidebarSections
              : sidebarSections;
            return sections === prev.sidebarSections &&
              noteWidth === prev.noteWidth
              ? prev
              : { sidebarSections: sections, noteWidth };
          });
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

  return demoMode ? DEFAULT_LAYOUT : layout;
}

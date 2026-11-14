import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import { SETTINGS_SAVED_EVENT } from "./settingsApi";
import {
  ALL_SIDEBAR_SECTIONS,
  sidebarSectionsFrom,
  useSidebarSections,
} from "./sidebarSections";

vi.mock("../../api/api", () => ({ apiFetch: vi.fn() }));
const mockedApiFetch = vi.mocked(apiFetch);

function answer(changed: string, recent: string): Response {
  return new Response(
    JSON.stringify({
      settings: [
        { key: "HATCHDOOR_SIDEBAR_CHANGED_ON_DISK", value: changed },
        { key: "HATCHDOOR_SIDEBAR_RECENTLY_VIEWED", value: recent },
      ],
    }),
    { status: 200 },
  );
}

describe("sidebar switches (#684)", () => {
  beforeEach(() => {
    window.localStorage.clear();
    mockedApiFetch.mockReset();
  });
  afterEach(() => window.localStorage.clear());

  it("reads both as on unless the server says false", () => {
    expect(sidebarSectionsFrom({})).toEqual(ALL_SIDEBAR_SECTIONS);
    expect(
      sidebarSectionsFrom({
        settings: [
          { key: "HATCHDOOR_SIDEBAR_CHANGED_ON_DISK", value: "false" },
          { key: "HATCHDOOR_SIDEBAR_RECENTLY_VIEWED", value: "true" },
        ],
      }),
    ).toEqual({ changedOnDisk: false, recentlyViewed: true });
  });

  it("shows both before the answer, then follows it, and reads again after a save", async () => {
    mockedApiFetch.mockResolvedValueOnce(answer("false", "true"));
    const { result } = renderHook(() =>
      useSidebarSections({ settingsEnabled: true, demoMode: false }),
    );
    expect(result.current).toEqual(ALL_SIDEBAR_SECTIONS);
    await waitFor(() =>
      expect(result.current).toEqual({
        changedOnDisk: false,
        recentlyViewed: true,
      }),
    );

    mockedApiFetch.mockResolvedValueOnce(answer("true", "false"));
    act(() => {
      window.dispatchEvent(new Event(SETTINGS_SAVED_EVENT));
    });
    await waitFor(() =>
      expect(result.current).toEqual({
        changedOnDisk: true,
        recentlyViewed: false,
      }),
    );
  });

  it("starts from the last answer this browser had, so a hidden list never flashes", async () => {
    mockedApiFetch.mockResolvedValueOnce(answer("false", "false"));
    const first = renderHook(() =>
      useSidebarSections({ settingsEnabled: true, demoMode: false }),
    );
    await waitFor(() => expect(first.result.current.changedOnDisk).toBe(false));
    first.unmount();

    const second = renderHook(() =>
      useSidebarSections({ settingsEnabled: false, demoMode: false }),
    );
    expect(second.result.current).toEqual({
      changedOnDisk: false,
      recentlyViewed: false,
    });
  });

  it("keeps what it shows when the settings cannot be read", async () => {
    mockedApiFetch.mockResolvedValueOnce(new Response("", { status: 401 }));
    const { result } = renderHook(() =>
      useSidebarSections({ settingsEnabled: true, demoMode: false }),
    );
    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(1));
    expect(result.current).toEqual(ALL_SIDEBAR_SECTIONS);
  });

  it("shows both on a demo, which has no settings, and asks nothing", () => {
    window.localStorage.setItem(
      "hatchdoor.sidebarSections",
      JSON.stringify({ changedOnDisk: false, recentlyViewed: false }),
    );
    const { result } = renderHook(() =>
      useSidebarSections({ settingsEnabled: false, demoMode: true }),
    );
    expect(result.current).toEqual(ALL_SIDEBAR_SECTIONS);
    expect(mockedApiFetch).not.toHaveBeenCalled();
  });
});

import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { clearToken } from "../api/api";
import { useStartupStatus } from "./useStartupStatus";

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  clearToken();
  window.localStorage.clear();
});

function statusResponse(body: object) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

describe("useStartupStatus", () => {
  it("does not poll while the workspace is zero-Vault or in registry recovery", () => {
    const fetchMock = vi.spyOn(globalThis, "fetch");

    renderHook(() => useStartupStatus(false));

    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("keeps the stepped-past latch across a remount", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      statusResponse({ state: "scanning" }),
    );
    const { result, unmount } = renderHook(() => useStartupStatus());

    await waitFor(() => expect(result.current.hasSteppedPastGate).toBe(true));
    unmount();

    const { result: remounted } = renderHook(() => useStartupStatus(false));
    expect(remounted.current.hasSteppedPastGate).toBe(true);
  });

  it("starts with hasSteppedPastGate false and flips it once a non-gate state is seen", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      statusResponse({ state: "indexing", percent: 10 }),
    );

    const { result } = renderHook(() => useStartupStatus());
    expect(result.current.hasSteppedPastGate).toBe(false);

    await act(async () => {});

    expect(result.current.status).toEqual({ state: "indexing", percent: 10 });
    expect(result.current.hasSteppedPastGate).toBe(true);
  });

  it("never flips hasSteppedPastGate while state stays terms_required or downloading", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      statusResponse({ state: "downloading" }),
    );

    const { result } = renderHook(() => useStartupStatus());
    await act(async () => {});

    expect(result.current.status).toEqual({ state: "downloading" });
    expect(result.current.hasSteppedPastGate).toBe(false);
  });

  it("stops polling once ready, and a retry after a failure resumes it", async () => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(
        statusResponse({ state: "failed", message: "model download failed" }),
      );

    const { result } = renderHook(() => useStartupStatus());
    await act(async () => {});
    expect(result.current.status).toEqual({
      state: "failed",
      message: "model download failed",
    });
    expect(result.current.hasSteppedPastGate).toBe(true);

    // Polling stopped: advancing time fires no further fetch.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);

    // Real timers from here: the resumed poll below schedules its own
    // setTimeout via window.setTimeout, and this assertion only needs the
    // in-flight fetch/json microtasks to settle, not a real 1s wait.
    vi.useRealTimers();
    // Two queued responses: the retry POST itself, then the resumed poll's
    // GET (fetch doesn't distinguish the two calls by URL here).
    fetchMock
      .mockResolvedValueOnce(new Response(null, { status: 202 }))
      .mockResolvedValueOnce(statusResponse({ state: "ready" }));
    await act(async () => {
      await result.current.retryModelSetup();
    });

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/model/retry",
      expect.objectContaining({ method: "POST" }),
    );
    // retryModelSetup fires the resumed poll without awaiting it: the
    // optimistic `downloading` set lands first, then the resumed poll's
    // real answer.
    await waitFor(() =>
      expect(result.current.status).toEqual({ state: "ready" }),
    );
  });
});

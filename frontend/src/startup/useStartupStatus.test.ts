import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { clearToken } from "../api/api";
import { startupPollDelay, useStartupStatus } from "./useStartupStatus";

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

  it("backs off while the status route is unreachable and returns to 1s once it answers (#304)", async () => {
    vi.useFakeTimers();
    let online = false;
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(async () => {
        if (!online) {
          throw new TypeError("Failed to fetch");
        }
        return statusResponse({ state: "indexing", percent: 10 });
      });
    const calls = () => fetchMock.mock.calls.length;

    const { result } = renderHook(() => useStartupStatus());
    await act(async () => {});
    expect(calls()).toBe(1);
    expect(result.current.connectionIssue).toBe(true);

    // One failure: the next poll waits 2s, not 1s.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(calls()).toBe(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(calls()).toBe(2);

    // Two failures: 4s.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3_999);
    });
    expect(calls()).toBe(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(calls()).toBe(3);

    // A minute offline asks a handful of times, not sixty.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(calls()).toBeLessThanOrEqual(7);

    // Back online: the next scheduled poll succeeds, and from then on the
    // interval is the normal 1s.
    online = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(result.current.connectionIssue).toBe(false);
    const afterRecovery = calls();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(calls()).toBe(afterRecovery + 1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(calls()).toBe(afterRecovery + 2);
  });

  it("treats a non-2xx status answer as a failure to back off from", async () => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response("bad gateway", { status: 502 }));

    renderHook(() => useStartupStatus());
    await act(async () => {});
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("doubles the delay per failure up to a 30s ceiling", () => {
    expect(startupPollDelay(0)).toBe(1_000);
    expect(startupPollDelay(1)).toBe(2_000);
    expect(startupPollDelay(2)).toBe(4_000);
    expect(startupPollDelay(4)).toBe(16_000);
    expect(startupPollDelay(5)).toBe(30_000);
    expect(startupPollDelay(50)).toBe(30_000);
  });
});

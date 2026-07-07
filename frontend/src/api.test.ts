import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch, DEFAULT_FETCH_TIMEOUT_MS, setToken } from "./api";

afterEach(() => {
  window.localStorage.clear();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("apiFetch", () => {
  it("passes an abort signal to fetch so stalled requests can time out", async () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response("ok"));

    await apiFetch("/api/tree");

    const [, init] = fetchSpy.mock.calls[0] ?? [];
    expect(init?.signal).toBeInstanceOf(AbortSignal);
  });

  it("aborts requests that never settle", async () => {
    vi.useFakeTimers();
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (_input, init) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => {
            reject(
              init.signal?.reason ?? new DOMException("Aborted", "AbortError"),
            );
          });
        }),
    );

    const request = apiFetch("/api/tree");
    const rejection = expect(request).rejects.toMatchObject({
      name: "AbortError",
    });
    await vi.advanceTimersByTimeAsync(DEFAULT_FETCH_TIMEOUT_MS);

    await rejection;
  });

  it("honors a per-call timeoutMs override instead of the default timeout", async () => {
    vi.useFakeTimers();
    let aborted = false;
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (_input, init) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => {
            aborted = true;
            reject(
              init.signal?.reason ?? new DOMException("Aborted", "AbortError"),
            );
          });
        }),
    );

    const request = apiFetch("/api/attachment", {
      method: "POST",
      timeoutMs: 60_000,
    });
    const rejection = expect(request).rejects.toMatchObject({
      name: "AbortError",
    });

    await vi.advanceTimersByTimeAsync(DEFAULT_FETCH_TIMEOUT_MS);
    expect(aborted).toBe(false);

    await vi.advanceTimersByTimeAsync(60_000 - DEFAULT_FETCH_TIMEOUT_MS);
    expect(aborted).toBe(true);
    await rejection;
  });

  it("preserves Headers instance values when attaching the bearer token", async () => {
    setToken("secret-token");
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response("ok"));

    await apiFetch("/api/tree", {
      headers: new Headers({ "X-Trace-Id": "trace-1" }),
    });

    const [, init] = fetchSpy.mock.calls[0] ?? [];
    const headers = new Headers(init?.headers);
    expect(headers.get("X-Trace-Id")).toBe("trace-1");
    expect(headers.get("Authorization")).toBe("Bearer secret-token");
  });
});

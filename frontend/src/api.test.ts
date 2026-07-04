import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch, DEFAULT_FETCH_TIMEOUT_MS } from "./api";

afterEach(() => {
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
});

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  apiFetch,
  clearToken,
  DEFAULT_FETCH_TIMEOUT_MS,
  getToken,
  onUnauthorized,
  setToken,
  withAccessToken,
} from "./api";

afterEach(() => {
  clearToken();
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

describe("web token with site data blocked (#339)", () => {
  function blockStorage(): () => void {
    const descriptor = Object.getOwnPropertyDescriptor(window, "localStorage");
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      get() {
        throw new DOMException("The operation is insecure.", "SecurityError");
      },
    });
    return () => {
      if (descriptor) {
        Object.defineProperty(window, "localStorage", descriptor);
      }
    };
  }

  it("keeps an unlocked token for this page when storage throws, as WebKit does", async () => {
    const restore = blockStorage();
    try {
      setToken("secret-token");
      expect(getToken()).toBe("secret-token");
      expect(withAccessToken("/api/v1/vaults/events")).toBe(
        "/api/v1/vaults/events?access_token=secret-token",
      );

      const fetchSpy = vi
        .spyOn(globalThis, "fetch")
        .mockResolvedValue(new Response("ok"));
      await apiFetch("/api/tree");
      const [, init] = fetchSpy.mock.calls[0] ?? [];
      expect(new Headers(init?.headers).get("Authorization")).toBe(
        "Bearer secret-token",
      );

      clearToken();
      expect(getToken()).toBeNull();
    } finally {
      restore();
    }
  });

  it("keeps the token when storage reads but refuses the write", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("Quota exceeded", "QuotaExceededError");
    });

    setToken("secret-token");

    expect(getToken()).toBe("secret-token");
  });

  it("uses a new token the browser refused over an old one still stored", () => {
    window.localStorage.setItem("hatchdoor_web_token", "stale-token");
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("Quota exceeded", "QuotaExceededError");
    });

    setToken("secret-token");

    expect(getToken()).toBe("secret-token");
  });

  it("follows storage when it works, so a sign-out in another tab still applies", () => {
    setToken("secret-token");
    window.localStorage.removeItem("hatchdoor_web_token");

    expect(getToken()).toBeNull();
  });
});

describe("unauthorized notifications", () => {
  afterEach(() => onUnauthorized(null));

  it("ignores a 401 answering a request sent before the token changed", async () => {
    const handler = vi.fn();
    onUnauthorized(handler);
    let answer: (response: Response) => void = () => {};
    vi.spyOn(globalThis, "fetch").mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );

    const request = apiFetch("/api/v1/vaults");
    setToken("secret-token");
    answer(new Response("", { status: 401 }));
    await request;

    expect(handler).not.toHaveBeenCalled();
  });

  it("reports a 401 for the token currently in use", async () => {
    const handler = vi.fn();
    onUnauthorized(handler);
    setToken("wrong-token");
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response("", { status: 401 }),
    );

    await apiFetch("/api/v1/vaults");

    expect(handler).toHaveBeenCalledOnce();
  });
});

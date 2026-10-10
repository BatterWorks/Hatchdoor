import { afterEach, describe, expect, it, vi } from "vitest";

import {
  SEEN_VERSION_KEY,
  baseVersion,
  markSeen,
  readSeen,
  releasesToShow,
  type WhatsNewResponse,
} from "./whatsNew";

function release(version: string, actionNeeded = false) {
  return {
    version,
    date: "2026-10-20",
    highlights: [
      {
        text: `Something in ${version}.`,
        action_needed: actionNeeded,
        link: null,
      },
    ],
  };
}

function response(overrides: Partial<WhatsNewResponse> = {}): WhatsNewResponse {
  return {
    version: "2.8.0",
    previous_version: "2.7.0",
    fresh_install: false,
    releases: [release("2.8.0")],
    ...overrides,
  };
}

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("baseVersion", () => {
  it("drops a development build's suffix", () => {
    expect(baseVersion("2.8.0 (dev abc123)")).toBe("2.8.0");
    expect(baseVersion("2.8.0")).toBe("2.8.0");
  });
});

describe("releasesToShow", () => {
  it("shows what the server lists when the browser has no record", () => {
    const shown = releasesToShow(response(), { available: true, seen: null });
    expect(shown.map((r) => r.version)).toEqual(["2.8.0"]);
  });

  it("shows nothing once this browser has seen the running version", () => {
    expect(
      releasesToShow(response(), { available: true, seen: "2.8.0" }),
    ).toEqual([]);
  });

  it("compares a development build by its base version", () => {
    expect(
      releasesToShow(response({ version: "2.8.0 (dev abc123)" }), {
        available: true,
        seen: "2.8.0",
      }),
    ).toEqual([]);
  });

  it("keeps only the releases newer than what this browser saw", () => {
    const shown = releasesToShow(
      response({
        version: "2.9.0",
        previous_version: "2.7.0",
        releases: [release("2.9.0"), release("2.8.0")],
      }),
      { available: true, seen: "2.8.0" },
    );
    expect(shown.map((r) => r.version)).toEqual(["2.9.0"]);
  });

  it("stacks every skipped release, newest first", () => {
    const shown = releasesToShow(
      response({
        version: "2.9.0",
        previous_version: "2.6.0",
        releases: [release("2.7.0"), release("2.9.0"), release("2.8.0")],
      }),
      { available: true, seen: null },
    );
    expect(shown.map((r) => r.version)).toEqual(["2.9.0", "2.8.0", "2.7.0"]);
  });

  it("shows nothing on a fresh install", () => {
    expect(
      releasesToShow(
        response({ fresh_install: true, previous_version: null, releases: [] }),
        { available: true, seen: null },
      ),
    ).toEqual([]);
    // Even should the server list something, a fresh install stays quiet.
    expect(
      releasesToShow(response({ fresh_install: true }), {
        available: true,
        seen: null,
      }),
    ).toEqual([]);
  });

  it("shows nothing when the server has nothing new", () => {
    expect(
      releasesToShow(response({ releases: [] }), {
        available: true,
        seen: null,
      }),
    ).toEqual([]);
  });

  it("shows nothing when storage is blocked, so it cannot nag on every load", () => {
    expect(
      releasesToShow(response(), { available: false, seen: null }),
    ).toEqual([]);
  });

  it("ignores a stored value that is not a version", () => {
    const shown = releasesToShow(response(), {
      available: true,
      seen: "garbage",
    });
    expect(shown.map((r) => r.version)).toEqual(["2.8.0"]);
  });
});

describe("seen-version storage", () => {
  it("reads nothing before anything was stored", () => {
    expect(readSeen()).toEqual({ available: true, seen: null });
  });

  it("remembers the running version's base", () => {
    markSeen("2.8.0 (dev abc123)");
    expect(localStorage.getItem(SEEN_VERSION_KEY)).toBe("2.8.0");
    expect(readSeen()).toEqual({ available: true, seen: "2.8.0" });
  });

  it("reports blocked storage instead of throwing", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    expect(readSeen()).toEqual({ available: false, seen: null });
    expect(() => markSeen("2.8.0")).not.toThrow();
  });
});

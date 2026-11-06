import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { clearToken, onUnauthorized, setToken } from "../api/api";
import {
  collectionEnvelope,
  discoveryResponse,
  healthyVault,
  participantFor,
  staleVault,
} from "../test/fixtures/vaults";
import type { VaultStatistics, VaultSummary } from "../types";
import { useVaultCollection, useVaultProjection } from "./useVaultCollection";
import {
  fetchRegistryRevision,
  refreshVaultCollection,
  resetVaultCollection,
} from "./vaultCollectionStore";

afterEach(() => {
  cleanup();
  resetVaultCollection();
  vi.useRealTimers();
  vi.restoreAllMocks();
  clearToken();
  onUnauthorized(null);
});

function jsonResponse(body: object, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function statsFor(vaults: VaultSummary[], counts: number[]) {
  const stats: VaultStatistics[] = vaults.map((vault, index) => ({
    vault_id: vault.vault_id,
    vault_name: vault.name,
    note_count: counts[index],
    tag_count: 0,
    link_count: 0,
    vault_size_bytes: 0,
  }));
  return collectionEnvelope(
    "all",
    stats,
    vaults.map((vault) => participantFor(vault)),
  );
}

/** Answers the collection client's two reads and nothing else, so a stray
 * request from a surface that has not been migrated fails loudly. */
function mockCollection(
  vaults: VaultSummary[],
  options: {
    counts?: number[];
    demoMode?: boolean;
    extraDiscovery?: object;
  } = {},
) {
  const discovery = {
    ...discoveryResponse(vaults, options.demoMode ?? false),
    ...(options.extraDiscovery ?? {}),
  };
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/vaults")) {
        return Promise.resolve(jsonResponse(discovery));
      }
      if (url.endsWith("/api/v1/vaults/all/stats")) {
        return Promise.resolve(
          jsonResponse(statsFor(vaults, options.counts ?? vaults.map(() => 0))),
        );
      }
      return Promise.reject(new Error(`unexpected request: ${url}`));
    });
}

function emitRevision(revision: number) {
  for (const source of window.__hatchdoorEventSources) {
    if (source.url.includes("/api/v1/vaults/events")) {
      source.emit(
        "vault-collection-revision",
        JSON.stringify({ collection_revision: revision }),
      );
    }
  }
}

describe("the Vault collection client's list", () => {
  it("browses enabled Vaults only, and keeps the full registry for Vault management", async () => {
    const enabled = healthyVault("Enabled");
    const disabled = healthyVault("Disabled", { enabled: false });
    mockCollection([enabled, disabled]);

    const { result } = renderHook(() => useVaultCollection());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.vaults).toEqual([enabled]);
    expect(result.current.allVaults).toEqual([enabled, disabled]);
    expect(result.current.demoMode).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it("reads the collection once however many surfaces ask for it", async () => {
    const vault = healthyVault("Solo");
    const fetchMock = mockCollection([vault]);

    const { result } = renderHook(() => ({
      first: useVaultCollection(),
      second: useVaultCollection(),
      projection: useVaultProjection(),
    }));

    await waitFor(() => expect(result.current.first.loading).toBe(false));
    const discoveryCalls = fetchMock.mock.calls.filter(([input]) =>
      String(input).endsWith("/api/v1/vaults"),
    );
    expect(discoveryCalls).toHaveLength(1);
    expect(result.current.second.vaults).toBe(result.current.first.vaults);
  });

  it("surfaces a load failure as an error message", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      jsonResponse(
        { code: "internal_error", message: "boom", retryable: false },
        500,
      ),
    );

    const { result } = renderHook(() => useVaultCollection());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.vaults).toEqual([]);
    expect(result.current.error).toBe("boom");
    // Nothing is known about the collection: not the same as none (#333).
    expect(result.current.readState).toBe("error");
  });

  it("names an unreachable server rather than the browser's own wording (#333)", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(
      new TypeError("Failed to fetch"),
    );

    const { result } = renderHook(() => useVaultCollection());

    await waitFor(() => expect(result.current.readState).toBe("error"));
    expect(result.current.error).toBe("Could not reach the Hatchdoor server.");
  });

  it("recovers from a failed discovery on the next refresh (#333)", async () => {
    const alpha = healthyVault("Alpha");
    let online = false;
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        if (!online) {
          return Promise.reject(new TypeError("Failed to fetch"));
        }
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return Promise.resolve(
            jsonResponse(discoveryResponse([alpha], false)),
          );
        }
        return Promise.resolve(jsonResponse(statsFor([alpha], [4])));
      },
    );

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.readState).toBe("error"));

    online = true;
    await act(async () => {
      await refreshVaultCollection();
    });

    expect(result.current.readState).toBe("ready");
    expect(result.current.error).toBeNull();
    expect(result.current.vaults).toEqual([alpha]);
  });

  it("reads a discovery with no enabled Vaults as empty, not as an error", async () => {
    mockCollection([]);

    const { result } = renderHook(() => useVaultCollection());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.readState).toBe("empty");
  });

  it("keeps the last known list when a later refresh fails", async () => {
    const alpha = healthyVault("Alpha");
    let online = true;
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        if (!online) {
          return Promise.reject(new TypeError("Failed to fetch"));
        }
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return Promise.resolve(
            jsonResponse(discoveryResponse([alpha], false)),
          );
        }
        return Promise.resolve(jsonResponse(statsFor([alpha], [4])));
      },
    );

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.readState).toBe("ready"));

    online = false;
    await act(async () => {
      await refreshVaultCollection();
    });

    expect(result.current.vaults).toEqual([alpha]);
    expect(result.current.error).toBe("Could not reach the Hatchdoor server.");
    expect(result.current.readState).not.toBe("error");
  });

  it("keeps an unreadable registry distinct from a failed legacy upgrade", async () => {
    mockCollection([], {
      extraDiscovery: {
        recovery: {
          code: "vault_registry_recovery_required",
          kind: "corrupt",
          message: "the registry file is not valid JSON",
        },
      },
    });

    const { result } = renderHook(() => useVaultCollection());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.recovery?.kind).toBe("corrupt");
  });
});

describe("the Vault collection client's note counts", () => {
  it("reads counts at all scope and maps note_count by Vault", async () => {
    const alpha = healthyVault("Alpha");
    const beta = healthyVault("Beta");
    const fetchMock = mockCollection([alpha, beta], { counts: [126, 7] });

    const { result } = renderHook(() => useVaultCollection());

    await waitFor(() =>
      expect(result.current.noteCounts).toEqual({
        [alpha.vault_id]: 126,
        [beta.vault_id]: 7,
      }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/vaults/all/stats",
      expect.anything(),
    );
  });

  it("does not ask for counts when there is no enabled Vault to count", async () => {
    const fetchMock = mockCollection([]);

    const { result } = renderHook(() => useVaultCollection());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(
      fetchMock.mock.calls.filter(([input]) =>
        String(input).endsWith("/all/stats"),
      ),
    ).toHaveLength(0);
  });

  it("leaves prior counts in place when a counts read fails", async () => {
    const alpha = healthyVault("Alpha");
    let statsOk = true;
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return Promise.resolve(
            jsonResponse(discoveryResponse([alpha], false)),
          );
        }
        if (statsOk) {
          return Promise.resolve(jsonResponse(statsFor([alpha], [4])));
        }
        return Promise.resolve(jsonResponse({ message: "no" }, 500));
      },
    );

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() =>
      expect(result.current.noteCounts).toEqual({ [alpha.vault_id]: 4 }),
    );

    statsOk = false;
    await act(async () => {
      await refreshVaultCollection();
    });

    expect(result.current.noteCounts).toEqual({ [alpha.vault_id]: 4 });
    expect(result.current.readState).toBe("partial");
  });

  it("keeps a Vault's last known count when a partial read leaves it out (#333)", async () => {
    const alpha = healthyVault("Alpha");
    const beta = healthyVault("Beta");
    let partial = false;
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return Promise.resolve(
            jsonResponse(discoveryResponse([alpha, beta], false)),
          );
        }
        if (!partial) {
          return Promise.resolve(jsonResponse(statsFor([alpha, beta], [4, 7])));
        }
        // Beta's snapshot read failed: the server lists it as a participant
        // and leaves it out of `data`.
        const envelope = statsFor([alpha], [5]);
        return Promise.resolve(
          jsonResponse({
            ...envelope,
            partial: true,
            participants: [
              participantFor(alpha),
              participantFor(beta, "unavailable"),
            ],
          }),
        );
      },
    );

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.readState).toBe("ready"));
    expect(result.current.noteCounts).toEqual({
      [alpha.vault_id]: 4,
      [beta.vault_id]: 7,
    });

    partial = true;
    await act(async () => {
      await refreshVaultCollection();
    });

    expect(result.current.noteCounts).toEqual({
      [alpha.vault_id]: 5,
      [beta.vault_id]: 7,
    });
    expect(result.current.readState).toBe("partial");
  });

  it("leaves a Vault a partial read never counted unknown, not 0 (#333)", async () => {
    const alpha = healthyVault("Alpha");
    const beta = healthyVault("Beta");
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return Promise.resolve(
            jsonResponse(discoveryResponse([alpha, beta], false)),
          );
        }
        return Promise.resolve(
          jsonResponse({
            ...statsFor([alpha], [4]),
            partial: true,
            participants: [
              participantFor(alpha),
              participantFor(beta, "unavailable"),
            ],
          }),
        );
      },
    );

    const { result } = renderHook(() => ({
      collection: useVaultCollection(),
      projection: useVaultProjection(),
    }));
    await waitFor(() =>
      expect(result.current.collection.readState).toBe("partial"),
    );

    expect(result.current.collection.noteCounts).toEqual({
      [alpha.vault_id]: 4,
    });
    expect(result.current.projection.slotFor(beta)).toEqual({
      kind: "count",
      count: null,
    });
  });
});

describe("the Vault collection client's demo projection", () => {
  it("clamps a condition to amber with the instruction-free sentence in demo mode", async () => {
    const stale = staleVault("Stale");
    mockCollection([stale], { demoMode: true });

    const { result } = renderHook(() => ({
      collection: useVaultCollection(),
      projection: useVaultProjection(),
    }));
    await waitFor(() => expect(result.current.collection.loading).toBe(false));

    const slot = result.current.projection.slotFor(stale);
    expect(slot).toEqual({
      kind: "condition",
      word: "stale",
      tier: "warn",
      sentence: "The last index build for this Vault failed.",
    });
  });

  it("applies the collection's own counts unless the surface supplies its own", async () => {
    const vault = healthyVault("Alpha");
    mockCollection([vault], { counts: [12] });

    const { result } = renderHook(() => ({
      collection: useVaultCollection(),
      projection: useVaultProjection(),
    }));
    await waitFor(() =>
      expect(result.current.collection.noteCounts).toEqual({
        [vault.vault_id]: 12,
      }),
    );

    expect(result.current.projection.slotFor(vault)).toEqual({
      kind: "count",
      count: 12,
    });
    // The graph counts the nodes it actually drew, not the Vault's notes.
    expect(result.current.projection.slotFor(vault, 3)).toEqual({
      kind: "count",
      count: 3,
    });
  });

  it("describes the current scope in the slot's own words", async () => {
    const vault = healthyVault("Alpha");
    mockCollection([vault], { counts: [1] });

    const { result } = renderHook(() => ({
      collection: useVaultCollection(),
      projection: useVaultProjection(),
    }));
    await waitFor(() =>
      expect(result.current.collection.noteCounts).toEqual({
        [vault.vault_id]: 1,
      }),
    );

    expect(result.current.projection.describeScope("all")).toBe("1 Vault");
    expect(result.current.projection.describeScope(vault.vault_id)).toBe(
      "1 note",
    );
  });
});

describe("the Vault collection client's revision stream", () => {
  it("opens one subscription for the whole app", async () => {
    mockCollection([healthyVault("Solo")]);

    const { result } = renderHook(() => ({
      first: useVaultCollection(),
      second: useVaultCollection(),
    }));
    await waitFor(() => expect(result.current.first.loading).toBe(false));

    expect(
      window.__hatchdoorEventSources.filter((source) =>
        source.url.includes("/api/v1/vaults/events"),
      ),
    ).toHaveLength(1);
  });

  it("reloads the list and the counts when the collection revision moves", async () => {
    const vault = healthyVault("Alpha");
    let counts = [1];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return Promise.resolve(
            jsonResponse(discoveryResponse([vault], false)),
          );
        }
        return Promise.resolve(jsonResponse(statsFor([vault], counts)));
      },
    );

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() =>
      expect(result.current.noteCounts).toEqual({ [vault.vault_id]: 1 }),
    );

    counts = [9];
    await act(async () => {
      emitRevision(4);
    });

    await waitFor(() =>
      expect(result.current.noteCounts).toEqual({ [vault.vault_id]: 9 }),
    );
    expect(result.current.revision).toBe(4);
  });

  it("ignores a repeated revision, and a malformed payload", async () => {
    mockCollection([healthyVault("Alpha")]);
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      emitRevision(3);
    });
    await waitFor(() => expect(result.current.revision).toBe(3));

    await act(async () => {
      emitRevision(3);
      for (const source of window.__hatchdoorEventSources) {
        source.emit("vault-collection-revision", "not json");
      }
    });

    expect(result.current.revision).toBe(3);
  });

  it("follows a revision that counts from zero again, because the server restarted", async () => {
    const vault = healthyVault("Alpha");
    let vaults = [vault];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return Promise.resolve(
            jsonResponse(discoveryResponse(vaults, false)),
          );
        }
        return Promise.resolve(jsonResponse(statsFor(vaults, [1])));
      },
    );

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    await act(async () => {
      emitRevision(20);
    });
    await waitFor(() => expect(result.current.revision).toBe(20));

    // `collection_revision` lives in memory and counts from 0 again after a
    // restart. Holding out for 21 would strand every surface on the old
    // high-water mark, because this is the app's only invalidation path.
    vaults = [{ ...vault, search: "stale" }];
    await act(async () => {
      emitRevision(1);
    });

    await waitFor(() => expect(result.current.revision).toBe(1));
    await waitFor(() => expect(result.current.vaults[0].search).toBe("stale"));
  });
});

function streams() {
  return window.__hatchdoorEventSources.filter((source) =>
    source.url.includes("/api/v1/vaults/events"),
  );
}

function latestStream() {
  const all = streams();
  return all[all.length - 1];
}

/** What the browser does when a reconnect is answered with anything but 200:
 * the stream is closed for good and `error` fires once. */
function giveUp(source = latestStream()) {
  source.readyState = EventSource.CLOSED;
  source.emit("error", "");
}

/** A server whose Vault list the test can change, counting its list reads. */
function mutableServer(initial: VaultSummary[]) {
  const server = {
    vaults: initial,
    discoveryStatus: 200,
    discoveryReads: 0,
  };
  vi.spyOn(globalThis, "fetch").mockImplementation(
    (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/vaults")) {
        server.discoveryReads += 1;
        if (server.discoveryStatus !== 200) {
          return Promise.resolve(
            jsonResponse(
              { code: "unauthorized", message: "no", retryable: false },
              server.discoveryStatus,
            ),
          );
        }
        return Promise.resolve(
          jsonResponse(discoveryResponse(server.vaults, false)),
        );
      }
      return Promise.resolve(
        jsonResponse(
          statsFor(
            server.vaults,
            server.vaults.map(() => 0),
          ),
        ),
      );
    },
  );
  return server;
}

describe("the Vault collection client's stream recovery (#646)", () => {
  it("reopens a stream the browser gave up on, after a backoff, with the current token", async () => {
    setToken("before-restart");
    const vault = healthyVault("Alpha");
    const server = mutableServer([vault]);
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });

    // A reverse proxy answered 502 while the server restarted; meanwhile the
    // Vault was renamed and the token rotated.
    setToken("after-restart");
    server.vaults = [{ ...vault, name: "Beta" }];
    act(() => giveUp());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(999);
    });
    expect(streams()).toHaveLength(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(streams()).toHaveLength(2);
    expect(latestStream().url).toContain("access_token=after-restart");
    expect(result.current.vaults[0].name).toBe("Beta");
  });

  it("waits longer after each failed reopen, and starts over once one opens", async () => {
    mutableServer([healthyVault("Alpha")]);
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });

    act(() => giveUp());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    act(() => giveUp());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_999);
    });
    expect(streams()).toHaveLength(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(streams()).toHaveLength(3);

    act(() => latestStream().emit("open", ""));
    act(() => giveUp());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(streams()).toHaveLength(4);
  });

  it("leaves a stream the browser is still retrying to the browser", async () => {
    mutableServer([healthyVault("Alpha")]);
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });

    act(() => latestStream().emit("error", ""));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });

    expect(streams()).toHaveLength(1);
  });

  it("reads the collection again when the browser's own reconnect opens", async () => {
    const server = mutableServer([healthyVault("Alpha")]);
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    const source = latestStream();
    act(() => source.emit("open", ""));
    const before = server.discoveryReads;

    await act(async () => {
      source.emit("open", "");
    });

    expect(server.discoveryReads).toBe(before + 1);
  });

  it("raises the app's token prompt when the server refuses the token on reconnect", async () => {
    const unauthorized = vi.fn();
    onUnauthorized(unauthorized);
    setToken("rotated-away");
    const server = mutableServer([healthyVault("Alpha")]);
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });

    server.discoveryStatus = 401;
    act(() => giveUp());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });

    expect(unauthorized).toHaveBeenCalled();
    // And it keeps trying, so a token fixed in place is picked up.
    setToken("fixed-in-place");
    act(() => giveUp());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(streams()).toHaveLength(3);
    expect(latestStream().url).toContain("access_token=fixed-in-place");
  });

  it("reads again when a stream it reopened connects, in case the read sent with the reopen failed", async () => {
    const vault = healthyVault("Alpha");
    const server = mutableServer([vault]);
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });

    // The read sent with the reopen meets the proxy's 502...
    server.discoveryStatus = 502;
    act(() => giveUp());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(result.current.vaults[0].name).toBe("Alpha");

    // ...then the server is back with the renamed Vault, and the stream connects.
    server.discoveryStatus = 200;
    server.vaults = [{ ...vault, name: "Beta" }];
    await act(async () => {
      latestStream().emit("open", "");
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.vaults[0].name).toBe("Beta");
  });

  it("does not read twice when the first stream opens", async () => {
    const server = mutableServer([healthyVault("Alpha")]);
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    const before = server.discoveryReads;

    await act(async () => {
      latestStream().emit("open", "");
    });

    expect(server.discoveryReads).toBe(before);
  });

  it.each([
    [
      "the tab is shown again",
      () => document.dispatchEvent(new Event("visibilitychange")),
    ],
    [
      "the browser is back online",
      () => window.dispatchEvent(new Event("online")),
    ],
  ])(
    "reopens a closed stream at once and reads again when %s",
    async (_, resume) => {
      const vault = healthyVault("Alpha");
      const server = mutableServer([vault]);
      const { result } = renderHook(() => useVaultCollection());
      await waitFor(() => expect(result.current.loading).toBe(false));
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
      act(() => giveUp());
      server.vaults = [{ ...vault, name: "Beta" }];

      await act(async () => {
        resume();
        await vi.advanceTimersByTimeAsync(0);
      });

      expect(streams()).toHaveLength(2);
      expect(result.current.vaults[0].name).toBe("Beta");
      // The backoff timer it replaced does not open a third.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(60_000);
      });
      expect(streams()).toHaveLength(2);
    },
  );

  it("stops retrying once the last subscriber leaves", async () => {
    mutableServer([healthyVault("Alpha")]);
    const { result, unmount } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    act(() => giveUp());

    unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });

    expect(streams()).toHaveLength(1);
  });
});

describe("the Vault collection client's read sequencing (#646)", () => {
  it("drops a list answer that arrives after a newer read failed", async () => {
    const vault = healthyVault("Alpha");
    let reads = 0;
    let releaseHeld: () => void = () => {};
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          reads += 1;
          if (reads === 2) {
            return new Promise((resolve) => {
              releaseHeld = () =>
                resolve(jsonResponse(discoveryResponse([vault], false)));
            });
          }
          if (reads > 2) {
            return Promise.reject(new TypeError("Failed to fetch"));
          }
          return Promise.resolve(
            jsonResponse(discoveryResponse([vault], false)),
          );
        }
        return Promise.resolve(jsonResponse(statsFor([vault], [0])));
      },
    );
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      emitRevision(5);
    });
    await act(async () => {
      emitRevision(6);
    });
    await waitFor(() =>
      expect(result.current.error).toBe(
        "Could not reach the Hatchdoor server.",
      ),
    );

    await act(async () => {
      releaseHeld();
    });

    // The older answer does not clear what the newer read reported.
    expect(result.current.error).toBe("Could not reach the Hatchdoor server.");
  });

  it("drops a list answer that arrives after a newer one", async () => {
    const vault = healthyVault("Alpha");
    const renamed = { ...vault, name: "Beta" };
    let reads = 0;
    let releaseHeld: () => void = () => {};
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          reads += 1;
          if (reads === 2) {
            // Sent before the rename, delivered after it.
            return new Promise((resolve) => {
              releaseHeld = () =>
                resolve(jsonResponse(discoveryResponse([vault], false)));
            });
          }
          return Promise.resolve(
            jsonResponse(
              discoveryResponse(reads > 2 ? [renamed] : [vault], false),
            ),
          );
        }
        return Promise.resolve(jsonResponse(statsFor([vault], [0])));
      },
    );
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      emitRevision(5);
    });
    await act(async () => {
      emitRevision(6);
    });
    await waitFor(() => expect(result.current.vaults[0].name).toBe("Beta"));

    await act(async () => {
      releaseHeld();
    });

    expect(result.current.vaults[0].name).toBe("Beta");
  });

  it("drops a counts answer that arrives after a newer one", async () => {
    const vault = healthyVault("Alpha");
    let reads = 0;
    let releaseHeld: () => void = () => {};
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return Promise.resolve(
            jsonResponse(discoveryResponse([vault], false)),
          );
        }
        reads += 1;
        if (reads === 2) {
          return new Promise((resolve) => {
            releaseHeld = () => resolve(jsonResponse(statsFor([vault], [3])));
          });
        }
        return Promise.resolve(
          jsonResponse(statsFor([vault], [reads > 2 ? 7 : 1])),
        );
      },
    );
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() =>
      expect(result.current.noteCounts).toEqual({ [vault.vault_id]: 1 }),
    );

    await act(async () => {
      emitRevision(5);
    });
    await act(async () => {
      emitRevision(6);
    });
    await waitFor(() =>
      expect(result.current.noteCounts).toEqual({ [vault.vault_id]: 7 }),
    );

    await act(async () => {
      releaseHeld();
    });

    expect(result.current.noteCounts).toEqual({ [vault.vault_id]: 7 });
  });
});

describe("the Vault collection client's identity discipline", () => {
  it("hands back the same list and counts when a refresh finds nothing new", async () => {
    const vault = healthyVault("Alpha");
    mockCollection([vault], { counts: [3] });

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    const beforeVaults = result.current.vaults;
    const beforeCounts = result.current.noteCounts;

    // A note write bumps the collection revision without changing any Vault.
    // A fresh-but-equal array here would relayout the graph for nothing.
    await act(async () => {
      emitRevision(7);
    });
    await waitFor(() => expect(result.current.revision).toBe(7));

    expect(result.current.vaults).toBe(beforeVaults);
    expect(result.current.noteCounts).toBe(beforeCounts);
  });

  it("hands back a new list once a Vault actually changes", async () => {
    const vault = healthyVault("Alpha");
    let vaults = [vault];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return Promise.resolve(
            jsonResponse(discoveryResponse(vaults, false)),
          );
        }
        return Promise.resolve(jsonResponse(statsFor(vaults, [3])));
      },
    );

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));
    const before = result.current.vaults;

    vaults = [{ ...vault, search: "stale" }];
    await act(async () => {
      emitRevision(2);
    });

    await waitFor(() => expect(result.current.vaults).not.toBe(before));
    expect(result.current.vaults[0].search).toBe("stale");
  });

  it("tells live subscribers the collection is unknown again, and reloads it", async () => {
    const vault = healthyVault("Alpha");
    mockCollection([vault], { counts: [3] });

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      resetVaultCollection();
    });

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.vaults).toEqual([vault]);
    expect(
      window.__hatchdoorEventSources.filter((source) =>
        source.url.includes("/api/v1/vaults/events"),
      ).length,
    ).toBeGreaterThan(0);
  });
});

describe("the Vault collection client's registry revision", () => {
  it("reads the revision fresh for an optimistic-concurrency precondition", async () => {
    mockCollection([healthyVault("Alpha")], {
      extraDiscovery: { registry_revision: 12 },
    });

    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await expect(fetchRegistryRevision()).resolves.toBe(12);
    expect(result.current.registryRevision).toBe(12);
  });

  it("reports no revision when the read fails", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      jsonResponse({ message: "no" }, 500),
    );

    await expect(fetchRegistryRevision()).resolves.toBeNull();
  });
});

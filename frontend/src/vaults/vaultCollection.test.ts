import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { clearToken } from "../api/api";
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
  vi.restoreAllMocks();
  clearToken();
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
    expect(result.current.legacyMigrationRecovery).toBeNull();
  });

  it("keeps a failed legacy upgrade distinct from an unreadable registry", async () => {
    mockCollection([], {
      extraDiscovery: {
        legacy_migration_recovery: {
          code: "legacy_migration_required",
          message: "legacy Vault path is not a readable directory",
        },
      },
    });

    const { result } = renderHook(() => useVaultCollection());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.legacyMigrationRecovery?.code).toBe(
      "legacy_migration_required",
    );
    expect(result.current.recovery).toBeNull();
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

  it("ignores a revision that does not move forward, and a malformed payload", async () => {
    mockCollection([healthyVault("Alpha")]);
    const { result } = renderHook(() => useVaultCollection());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      emitRevision(3);
    });
    await waitFor(() => expect(result.current.revision).toBe(3));

    await act(async () => {
      emitRevision(2);
      for (const source of window.__hatchdoorEventSources) {
        source.emit("vault-collection-revision", "not json");
      }
    });

    expect(result.current.revision).toBe(3);
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

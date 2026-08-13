import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  collectionEnvelope,
  discoveryResponse,
  healthyVault,
  participantFor,
} from "../test/fixtures/vaults";
import type { VaultStatistics } from "../types";
import {
  resolvePrimaryVaultId,
  useVaultDiscovery,
  useVaultNoteCounts,
  useVaultScope,
} from "./useVaultScope";

describe("useVaultScope", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it("defaults to all and persists a selected scope across instances", () => {
    const { result, unmount } = renderHook(() => useVaultScope());
    expect(result.current[0]).toBe("all");

    act(() => {
      result.current[1]("vault-123");
    });
    expect(result.current[0]).toBe("vault-123");
    unmount();

    const { result: reloaded } = renderHook(() => useVaultScope());
    expect(reloaded.current[0]).toBe("vault-123");
  });
});

describe("useVaultDiscovery", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("loads enabled Vaults and demo_mode from GET /api/v1/vaults", async () => {
    const enabled = healthyVault("Enabled");
    const disabled = { ...healthyVault("Disabled"), enabled: false };
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify(discoveryResponse([enabled, disabled], true)),
        {
          status: 200,
          headers: { "content-type": "application/json" },
        },
      ),
    );

    const { result } = renderHook(() => useVaultDiscovery());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(fetchMock).toHaveBeenCalledWith("/api/v1/vaults", expect.anything());
    expect(result.current.vaults).toEqual([enabled]);
    expect(result.current.demoMode).toBe(true);
    expect(result.current.error).toBeNull();
  });

  it("surfaces a load failure as an error message", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          code: "internal_error",
          message: "boom",
          retryable: false,
        }),
        { status: 500 },
      ),
    );

    const { result } = renderHook(() => useVaultDiscovery());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.vaults).toEqual([]);
    expect(result.current.error).toBe("boom");
  });
});

describe("useVaultNoteCounts", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("fetches /api/v1/vaults/all/stats and maps note_count by vault_id when enabled", async () => {
    const alpha = healthyVault("Alpha");
    const beta = healthyVault("Beta");
    const stats: VaultStatistics[] = [
      {
        vault_id: alpha.vault_id,
        vault_name: alpha.name,
        note_count: 126,
        tag_count: 4,
        link_count: 9,
        vault_size_bytes: 1024,
      },
      {
        vault_id: beta.vault_id,
        vault_name: beta.name,
        note_count: 7,
        tag_count: 1,
        link_count: 0,
        vault_size_bytes: 256,
      },
    ];
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify(
          collectionEnvelope("all", stats, [
            participantFor(alpha),
            participantFor(beta),
          ]),
        ),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    );

    const { result } = renderHook(() => useVaultNoteCounts(true, 0));

    await waitFor(() =>
      expect(result.current).toEqual({
        [alpha.vault_id]: 126,
        [beta.vault_id]: 7,
      }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/vaults/all/stats",
      expect.anything(),
    );
  });

  it("does not fetch when disabled — a single-enabled-Vault instance makes no request", () => {
    const fetchMock = vi.spyOn(globalThis, "fetch");

    renderHook(() => useVaultNoteCounts(false, 0));

    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("refetches when vaultRevision changes", async () => {
    const alpha = healthyVault("Alpha");
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify(
          collectionEnvelope(
            "all",
            [
              {
                vault_id: alpha.vault_id,
                vault_name: alpha.name,
                note_count: 1,
                tag_count: 0,
                link_count: 0,
                vault_size_bytes: 10,
              },
            ],
            [participantFor(alpha)],
          ),
        ),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    );

    const { rerender } = renderHook(
      ({ revision }) => useVaultNoteCounts(true, revision),
      { initialProps: { revision: 0 } },
    );
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    rerender({ revision: 1 });
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
  });
});

describe("resolvePrimaryVaultId", () => {
  it("prefers the open note's Vault over the first enabled Vault", () => {
    const first = healthyVault("First");
    const second = healthyVault("Second");
    expect(resolvePrimaryVaultId(second.vault_id, [first, second])).toBe(
      second.vault_id,
    );
  });

  it("falls back to the first enabled Vault when no note is open", () => {
    const first = healthyVault("First");
    const second = healthyVault("Second");
    expect(resolvePrimaryVaultId(undefined, [first, second])).toBe(
      first.vault_id,
    );
  });

  it("is undefined at zero enabled Vaults", () => {
    expect(resolvePrimaryVaultId(undefined, [])).toBeUndefined();
  });
});

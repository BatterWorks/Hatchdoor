import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { discoveryResponse, healthyVault } from "../test/fixtures/vaults";
import {
  resolvePrimaryVaultId,
  useVaultDiscovery,
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

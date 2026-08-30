import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { clearToken } from "../api/api";
import { healthyVault } from "../test/fixtures/vaults";
import { resolvePrimaryVaultId, useVaultScope } from "./useVaultScope";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  clearToken();
  window.localStorage.clear();
});

describe("useVaultScope", () => {
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

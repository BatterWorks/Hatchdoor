import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { clearToken } from "../api/api";
import { VAULT_SCOPE_KEY } from "../app/constants";
import {
  collectionEnvelope,
  discoveryResponse,
  healthyVault,
  participantFor,
  pausedVault,
  unavailableVault,
} from "../test/fixtures/vaults";
import type { VaultSummary } from "../types";
import { useVaultCollection } from "../vaults";
import { resetVaultCollection } from "../vaults/vaultCollectionStore";
import { resolvePrimaryVaultId, useVaultScope } from "./useVaultScope";

afterEach(() => {
  cleanup();
  resetVaultCollection();
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

function jsonResponse(body: object, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** Answers discovery with whatever `current.vaults` holds at the moment of the
 * read, so a test can change the collection and then announce the change on
 * the revision stream the way the server does. */
function mockLiveCollection(initial: VaultSummary[]) {
  const current = { vaults: initial, fail: false };
  vi.spyOn(globalThis, "fetch").mockImplementation(
    (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/vaults")) {
        if (current.fail) {
          return Promise.resolve(jsonResponse({ error: "down" }, 500));
        }
        return Promise.resolve(jsonResponse(discoveryResponse(current.vaults)));
      }
      if (url.endsWith("/api/v1/vaults/all/stats")) {
        const enabled = current.vaults.filter((vault) => vault.enabled);
        return Promise.resolve(
          jsonResponse(
            collectionEnvelope(
              "all",
              enabled.map((vault) => ({
                vault_id: vault.vault_id,
                vault_name: vault.name,
                note_count: 1,
                tag_count: 0,
                link_count: 0,
                vault_size_bytes: 0,
              })),
              enabled.map((vault) => participantFor(vault)),
            ),
          ),
        );
      }
      return Promise.reject(new Error(`unexpected request: ${url}`));
    },
  );
  return current;
}

let revision = 100;

function emitRevision() {
  revision += 1;
  for (const source of window.__hatchdoorEventSources) {
    if (source.url.includes("/api/v1/vaults/events")) {
      source.emit(
        "vault-collection-revision",
        JSON.stringify({ collection_revision: revision }),
      );
    }
  }
}

/** The hook beside the collection it reads, so a test can wait for discovery
 * to have actually answered before asserting that nothing changed. */
function renderScope() {
  return renderHook(() => {
    const [scope, , notice] = useVaultScope();
    const { readState } = useVaultCollection();
    return { scope, notice, readState };
  });
}

describe("useVaultScope reconciles against the live collection (#335)", () => {
  it("falls back to all when the browsed Vault is paused on a later revision", async () => {
    const alpha = healthyVault("Alpha");
    const beta = healthyVault("Beta");
    const live = mockLiveCollection([alpha, beta]);
    window.localStorage.setItem(VAULT_SCOPE_KEY, beta.vault_id);

    const { result } = renderScope();
    // Beta is live: the stored scope stands and nothing is announced.
    await waitFor(() => expect(result.current.readState).toBe("ready"));
    expect(result.current.scope).toBe(beta.vault_id);
    expect(result.current.notice).toBeNull();

    // Paused from Settings, another tab, or an MCP agent: the server only
    // says so on the revision stream.
    live.vaults = [alpha, { ...pausedVault("Beta"), vault_id: beta.vault_id }];
    act(() => emitRevision());

    await waitFor(() => expect(result.current.scope).toBe("all"));
    expect(window.localStorage.getItem(VAULT_SCOPE_KEY)).toBe("all");
    expect(result.current.notice?.message).toBe(
      "Beta is paused, so the explorer now shows All Vaults.",
    );
  });

  it("falls back when the sole remaining Vault is not the browsed one", async () => {
    const alpha = healthyVault("Alpha");
    const beta = healthyVault("Beta");
    mockLiveCollection([alpha]);
    // Beta was disconnected while this browser was away: at one enabled Vault
    // there is no Scope zone or sheet left to change it by hand.
    window.localStorage.setItem(VAULT_SCOPE_KEY, beta.vault_id);

    const { result } = renderScope();

    await waitFor(() => expect(result.current.scope).toBe("all"));
    expect(window.localStorage.getItem(VAULT_SCOPE_KEY)).toBe("all");
    expect(result.current.notice?.message).toBe(
      "The Vault you were browsing has been disconnected, so the explorer now shows All Vaults.",
    );
  });

  it("falls back from a Vault that is enabled but unavailable", async () => {
    const alpha = healthyVault("Alpha");
    const gone = unavailableVault("Offsite");
    mockLiveCollection([alpha, gone]);
    window.localStorage.setItem(VAULT_SCOPE_KEY, gone.vault_id);

    const { result } = renderScope();

    await waitFor(() => expect(result.current.scope).toBe("all"));
    expect(result.current.notice?.message).toBe(
      "Offsite is unavailable, so the explorer now shows All Vaults.",
    );
  });

  it("judges nothing when discovery failed or no Vault is enabled", async () => {
    const beta = healthyVault("Beta");
    const live = mockLiveCollection([pausedVault("Alpha")]);
    window.localStorage.setItem(VAULT_SCOPE_KEY, beta.vault_id);

    const { result, unmount } = renderScope();
    await waitFor(() => expect(result.current.readState).toBe("empty"));
    expect(result.current.scope).toBe(beta.vault_id);
    unmount();

    live.fail = true;
    const { result: failed } = renderScope();
    await waitFor(() => expect(failed.current.readState).toBe("error"));
    expect(failed.current.scope).toBe(beta.vault_id);
    expect(window.localStorage.getItem(VAULT_SCOPE_KEY)).toBe(beta.vault_id);
    expect(failed.current.notice).toBeNull();
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

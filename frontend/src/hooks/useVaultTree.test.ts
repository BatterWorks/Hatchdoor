import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  collectionEnvelope,
  discoveryResponse,
  EIGHT_VAULTS,
  ONE_VAULT,
  participantFor,
  THREE_VAULTS,
} from "../test/fixtures/vaults";
import { resetVaultCollection } from "../vaults/vaultCollectionStore";
import { useVaultTree } from "./useVaultTree";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function mockFetch(recentEnvelope: unknown, treeData: unknown[] = []) {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/tree")) {
        return jsonResponse(collectionEnvelope("all", treeData, []));
      }
      if (url.includes("/recent")) {
        return jsonResponse(recentEnvelope);
      }
      return jsonResponse({ error: "not found" });
    });
}

afterEach(() => {
  cleanup();
  resetVaultCollection();
  vi.restoreAllMocks();
});

describe("useVaultTree — Changed on disk partiality (#141)", () => {
  it("captures partial: false and no missing Vaults for a fully fresh read at three Vaults", async () => {
    mockFetch(
      collectionEnvelope(
        "all",
        [],
        THREE_VAULTS.map((vault) => participantFor(vault, "fresh")),
      ),
    );

    const { result } = renderHook(() => useVaultTree("all"));

    await waitFor(() => expect(result.current.loadingTree).toBe(false));
    expect(result.current.modifiedNotesPartial).toBe(false);
    expect(result.current.modifiedNotesMissingVaults).toEqual([]);
  });

  it("names only the Vaults that did not answer, at three Vaults", async () => {
    const participants = [
      participantFor(THREE_VAULTS[0], "fresh"),
      participantFor(THREE_VAULTS[1], "fresh"),
      participantFor(THREE_VAULTS[2], "unavailable"),
    ];
    mockFetch(collectionEnvelope("all", [], participants));

    const { result } = renderHook(() => useVaultTree("all"));

    await waitFor(() => expect(result.current.loadingTree).toBe(false));
    expect(result.current.modifiedNotesPartial).toBe(true);
    expect(result.current.modifiedNotesMissingVaults).toEqual([
      THREE_VAULTS[2].name,
    ]);
  });

  it("names every Vault that did not answer, at eight Vaults", async () => {
    const participants = EIGHT_VAULTS.map((vault, index) =>
      participantFor(vault, index < 6 ? "fresh" : "unavailable"),
    );
    mockFetch(collectionEnvelope("all", [], participants));

    const { result } = renderHook(() => useVaultTree("all"));

    await waitFor(() => expect(result.current.loadingTree).toBe(false));
    expect(result.current.modifiedNotesPartial).toBe(true);
    expect(result.current.modifiedNotesMissingVaults).toEqual([
      EIGHT_VAULTS[6].name,
      EIGHT_VAULTS[7].name,
    ]);
  });
});

describe("useVaultTree — per-Vault trees (#142)", () => {
  it("exposes each participating Vault's own tree, ungrouped", async () => {
    const treeData = THREE_VAULTS.map((vault) => ({
      vault_id: vault.vault_id,
      vault_name: vault.name,
      tree: { name: vault.name, note_count: 0, folders: [], notes: [] },
    }));
    mockFetch(collectionEnvelope("all", [], []), treeData);

    const { result } = renderHook(() => useVaultTree("all"));

    await waitFor(() => expect(result.current.loadingTree).toBe(false));
    expect(result.current.vaultTrees).toEqual(
      THREE_VAULTS.map((vault) => ({
        vault_id: vault.vault_id,
        vault_name: vault.name,
        tree: { name: vault.name, folders: [], notes: [] },
      })),
    );
  });

  it("stamps each Vault's ID onto the notes its tree sends without one (#192)", async () => {
    const treeData = THREE_VAULTS.map((vault, index) => ({
      vault_id: vault.vault_id,
      vault_name: vault.name,
      tree: {
        name: vault.name,
        note_count: 1,
        folders: [
          {
            name: "Nested",
            note_count: 1,
            folders: [],
            notes: [{ title: `${vault.name} nested`, slug: `nested-${index}` }],
          },
        ],
        notes: [{ title: `${vault.name} home`, slug: `home-${index}` }],
      },
    }));
    mockFetch(collectionEnvelope("all", [], []), treeData);

    const { result } = renderHook(() => useVaultTree("all"));

    await waitFor(() => expect(result.current.loadingTree).toBe(false));
    for (const [index, vault] of THREE_VAULTS.entries()) {
      const tree = result.current.vaultTrees[index].tree;
      expect(tree.notes[0].vault_id).toBe(vault.vault_id);
      expect(tree.folders[0].notes[0].vault_id).toBe(vault.vault_id);
    }
    // The autocomplete pool is flattened across Vaults, losing the grouping,
    // so every candidate must already know which Vault it came from.
    expect(
      [
        ...new Set(result.current.noteCandidates.map((note) => note.vault_id)),
      ].sort(),
    ).toEqual(THREE_VAULTS.map((vault) => vault.vault_id).sort());
    expect(result.current.noteCandidates).toHaveLength(6);
  });
});

describe("useVaultTree — one load per collection revision", () => {
  /** Answers discovery, the tree and the recent list, counting the two reads
   * the explorer makes, with the collection revision under the test's control
   * on both the discovery response and the projection envelopes. */
  function mockAtRevision(revision: number) {
    const counts = { tree: 0, recent: 0 };
    const envelope = (data: unknown[]) => ({
      ...collectionEnvelope("all", data, []),
      collection_revision: revision,
    });
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse({
            ...discoveryResponse(ONE_VAULT),
            collection_revision: revision,
          });
        }
        if (url.endsWith("/api/v1/vaults/all/stats")) {
          return jsonResponse(collectionEnvelope("all", [], []));
        }
        if (url.includes("/tree")) {
          counts.tree += 1;
          return jsonResponse(envelope([]));
        }
        if (url.includes("/recent")) {
          counts.recent += 1;
          return jsonResponse(envelope([]));
        }
        return jsonResponse({ error: "not found" });
      },
    );
    return counts;
  }

  it("reads the tree and the recent list once for a plain load", async () => {
    const counts = mockAtRevision(7);

    const { result } = renderHook(() => useVaultTree("all"));
    await waitFor(() => expect(result.current.loadingTree).toBe(false));
    // Discovery publishing the revision it answered at is what used to look
    // like a change and fetch everything a second time.
    await waitFor(() => expect(result.current.vaultRevision).toBe(7));

    expect(counts.tree).toBe(1);
    expect(counts.recent).toBe(1);
  });

  it("reads again when the revision moves past the one it loaded at", async () => {
    const counts = mockAtRevision(7);

    const { result } = renderHook(() => useVaultTree("all"));
    await waitFor(() => expect(result.current.vaultRevision).toBe(7));
    expect(counts.tree).toBe(1);

    act(() => {
      for (const source of window.__hatchdoorEventSources) {
        if (source.url.includes("/api/v1/vaults/events")) {
          source.emit(
            "vault-collection-revision",
            JSON.stringify({ collection_revision: 8 }),
          );
        }
      }
    });

    await waitFor(() => expect(counts.tree).toBe(2));
    expect(counts.recent).toBe(2);
  });
});

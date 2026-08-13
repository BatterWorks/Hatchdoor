import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  collectionEnvelope,
  EIGHT_VAULTS,
  participantFor,
  THREE_VAULTS,
} from "../test/fixtures/vaults";
import { useVaultTree } from "./useVaultTree";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function mockFetch(recentEnvelope: unknown) {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/tree")) {
        return jsonResponse(collectionEnvelope("all", [], []));
      }
      if (url.includes("/recent")) {
        return jsonResponse(recentEnvelope);
      }
      return jsonResponse({ error: "not found" });
    });
}

afterEach(() => {
  cleanup();
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

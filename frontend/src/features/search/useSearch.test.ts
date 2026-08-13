import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useSearch } from ".";
import {
  EIGHT_VAULTS,
  participantFor,
  THREE_VAULTS,
} from "../../test/fixtures/vaults";

describe("useSearch", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("debounces a search request and exposes its results", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          scope: "all",
          collection_revision: 1,
          partial: false,
          participants: [],
          data: {
            mode: "keyword",
            results: [
              {
                vault_id: "vault-1",
                chunk_id: 7,
                note_slug: "plan",
                note_title: "Plan",
                note_path: "Projects/Plan",
                heading_path: "Next",
                content: "Plan the next milestone",
                score: 1,
                outbound_links: [],
              },
            ],
          },
        }),
        {
          status: 200,
          headers: { "content-type": "application/json" },
        },
      ),
    );
    const { result } = renderHook(() => useSearch("all"));

    act(() => {
      result.current.setSearchQuery("plan");
      result.current.setSearchIncludeContent(true);
      result.current.setSearchOpen(true);
    });

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/vaults/all/search?q=plan&mode=keyword&limit=30&per_note_cap=2",
        expect.objectContaining({ signal: expect.any(AbortSignal) }),
      );
    });
    await waitFor(() => {
      expect(result.current.searchResults).toHaveLength(1);
    });
    expect(result.current.searchResults[0]?.note_slug).toBe("plan");
    expect(result.current.searchError).toBeNull();
  });
});

describe("useSearch — partiality (#141)", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  async function search(participants: ReturnType<typeof participantFor>[]) {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          scope: "all",
          collection_revision: 1,
          partial: participants.some((p) => p.state !== "fresh"),
          participants,
          data: { mode: "keyword", results: [] },
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    );
    const { result } = renderHook(() => useSearch("all"));
    act(() => {
      result.current.setSearchQuery("plan");
      result.current.setSearchOpen(true);
    });
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    await waitFor(() => expect(result.current.searchLoading).toBe(false));
    return result;
  }

  it("names only the Vaults that did not answer, at three Vaults", async () => {
    const result = await search([
      participantFor(THREE_VAULTS[0], "fresh"),
      participantFor(THREE_VAULTS[1], "fresh"),
      participantFor(THREE_VAULTS[2], "unavailable"),
    ]);

    expect(result.current.searchPartial).toBe(true);
    expect(result.current.searchMissingVaultNames).toEqual([
      THREE_VAULTS[2].name,
    ]);
  });

  it("names every missing Vault, at eight Vaults", async () => {
    const result = await search(
      EIGHT_VAULTS.map((vault, index) =>
        participantFor(vault, index < 5 ? "fresh" : "stale"),
      ),
    );

    expect(result.current.searchPartial).toBe(true);
    expect(result.current.searchMissingVaultNames).toEqual([
      EIGHT_VAULTS[5].name,
      EIGHT_VAULTS[6].name,
      EIGHT_VAULTS[7].name,
    ]);
  });

  it("is not partial when every participant answered fresh", async () => {
    const result = await search(
      THREE_VAULTS.map((vault) => participantFor(vault, "fresh")),
    );

    expect(result.current.searchPartial).toBe(false);
    expect(result.current.searchMissingVaultNames).toEqual([]);
  });
});

import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useSearch } from ".";

describe("useSearch", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("debounces a search request and exposes its results", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          mode: "keyword",
          results: [
            {
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
        }),
        {
          status: 200,
          headers: { "content-type": "application/json" },
        },
      ),
    );
    const { result } = renderHook(() => useSearch());

    act(() => {
      result.current.setSearchQuery("plan");
      result.current.setSearchIncludeContent(true);
      result.current.setSearchOpen(true);
    });

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/search?q=plan&mode=keyword&limit=30&per_note_cap=2",
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

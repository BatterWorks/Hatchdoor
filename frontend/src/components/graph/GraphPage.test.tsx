import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GraphPage } from "./GraphPage";

class ResizeObserverStub {
  observe() {}
  disconnect() {}
}

describe("GraphPage", () => {
  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 1),
    );
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("requests the graph and renders the backend error", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({ error: "Graph index is unavailable" }), {
        status: 503,
        headers: { "content-type": "application/json" },
      }),
    );

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );

    expect(screen.getByText("Mapping your vault…")).toBeVisible();
    expect(
      await screen.findByRole("heading", { name: "Graph Unavailable" }),
    ).toBeVisible();
    expect(screen.getByText("Graph index is unavailable")).toBeVisible();
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/graph",
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });
});

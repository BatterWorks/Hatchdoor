import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "./App";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("App write mode", () => {
  it("opens inline edit mode and saves the note with the current content hash", async () => {
    let noteCalls = 0;
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);

        if (url.includes("/api/write-capabilities")) {
          return new Response(
            JSON.stringify({ enabled: true, warnings: [] }),
            { status: 200 },
          );
        }

        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [{ title: "Home", slug: "home" }],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/recently-modified")) {
          return new Response(JSON.stringify({ notes: [] }), { status: 200 });
        }

        if (url.includes("/api/note/home/links")) {
          return new Response(
            JSON.stringify({ links: { outgoing: [], backlinks: [] } }),
            { status: 200 },
          );
        }

        if (url.endsWith("/api/note/home") && init?.method === "PUT") {
          return new Response(
            JSON.stringify({
              ok: true,
              slug: "home",
              relative_path: "Home",
              content_hash: "hash-2",
              git_sync_warning: null,
              rewritten_notes: 0,
              moved_assets: 0,
              trashed_path: null,
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home")) {
          noteCalls += 1;
          return new Response(
            JSON.stringify({
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content:
                  noteCalls === 1 ? "# Home\nOriginal" : "# Home\nUpdated",
                content_hash: noteCalls === 1 ? "hash-1" : "hash-2",
              },
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }

        return new Response("not found", { status: 404 });
      });

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    expect(
      await screen.findByRole("heading", { level: 2, name: "Home" }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Edit note" }));

    const textarea = await screen.findByRole("textbox", {
      name: "Markdown content",
    });
    fireEvent.change(textarea, { target: { value: "# Home\nUpdated" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/note/home",
        expect.objectContaining({
          method: "PUT",
          body: JSON.stringify({
            content: "# Home\nUpdated",
            expected_content_hash: "hash-1",
          }),
        }),
      );
    });

    expect(await screen.findByText("Updated")).toBeInTheDocument();
    await waitFor(() => {
      expect(
        screen.queryByRole("textbox", { name: "Markdown content" }),
      ).toBeNull();
    });
  });
});

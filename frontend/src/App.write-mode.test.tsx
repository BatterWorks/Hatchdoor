import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultApp as App } from "./App";
import { noteDraftKey } from "./lib/writeDrafts";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

function mockReadAndWriteApi() {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";

        if (url.includes("/api/write-capabilities")) {
          return new Response(JSON.stringify({ enabled: true, warnings: [] }), {
            status: 200,
          });
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

        if (url.includes("/api/refresh")) {
          return new Response(JSON.stringify({ ok: true }), { status: 200 });
        }

        if (url.includes("/api/note/home/links")) {
          return new Response(
            JSON.stringify({ links: { outgoing: [], backlinks: [] } }),
            { status: 200 },
          );
        }

        if (url.endsWith("/api/note") && method === "POST") {
          return new Response(
            JSON.stringify({
              ok: true,
              slug: "projects-new-note",
              relative_path: "Projects/New Note",
              content_hash: "hash-new",
              git_sync_warning: null,
              rewritten_notes: 0,
              moved_assets: 0,
              trashed_path: null,
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home/rename") && method === "PATCH") {
          return new Response(
            JSON.stringify({
              ok: true,
              slug: "renamed-note",
              relative_path: "Renamed Note",
              content_hash: "hash-renamed",
              git_sync_warning: null,
              rewritten_notes: 1,
              moved_assets: 0,
              trashed_path: null,
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home/move") && method === "PATCH") {
          return new Response(
            JSON.stringify({
              ok: true,
              slug: "archive-home",
              relative_path: "Archive/Home",
              content_hash: "hash-moved",
              git_sync_warning: null,
              rewritten_notes: 0,
              moved_assets: 0,
              trashed_path: null,
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home/archive") && method === "PATCH") {
          return new Response(
            JSON.stringify({
              ok: true,
              slug: "archive-home",
              relative_path: "90-archive/Home",
              content_hash: "hash-archived",
              git_sync_warning: null,
              rewritten_notes: 1,
              moved_assets: 0,
              trashed_path: null,
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home") && method === "DELETE") {
          return new Response(
            JSON.stringify({
              ok: true,
              slug: "home",
              relative_path: "Home",
              content_hash: "hash-1",
              git_sync_warning: null,
              rewritten_notes: 0,
              moved_assets: 0,
              trashed_path: "90-archive/Home.md",
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home") && method === "GET") {
          return new Response(
            JSON.stringify({
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content: "# Home\nOriginal",
                content_hash: "hash-1",
              },
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }

        return new Response("not found", { status: 404 });
      },
    );
}

describe("App write mode", () => {
  it("opens inline edit mode and saves the note with the current content hash", async () => {
    let noteCalls = 0;
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(
        async (input: RequestInfo | URL, init?: RequestInit) => {
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
            return new Response(JSON.stringify({ results: [] }), {
              status: 200,
            });
          }

          return new Response("not found", { status: 404 });
        },
      );

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

  it("creates a new note from the actions menu", async () => {
    const fetchMock = mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "New note" }));
    // The picker lists folders that exist; this vault has none, so a note in
    // "Projects" is created through the New folder path.
    fireEvent.change(screen.getByLabelText("Folder"), {
      target: { value: "//new-folder" },
    });
    fireEvent.change(screen.getByLabelText("New folder name"), {
      target: { value: "Projects" },
    });
    fireEvent.change(screen.getByLabelText("Note name"), {
      target: { value: "New Note" },
    });
    fireEvent.change(screen.getByLabelText("Markdown content"), {
      target: { value: "# New Note\n" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/note",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({
            relative_path: "Projects/New Note",
            content: "# New Note\n",
          }),
        }),
      );
    });
  });

  it("opens the rename dialog from the actions menu", async () => {
    mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Rename note" }),
    );
    expect(screen.getByLabelText("New title")).toBeInTheDocument();
  });

  it("archives a note from the actions menu", async () => {
    const fetchMock = mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Archive note" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/note/home/archive",
        expect.objectContaining({
          method: "PATCH",
          body: JSON.stringify({ expected_content_hash: "hash-1" }),
        }),
      );
    });
  });

  it("saves against the hash captured at edit start, ignoring disk changes mid-edit", async () => {
    let noteCalls = 0;
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(
        async (input: RequestInfo | URL, init?: RequestInit) => {
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
                content_hash: "hash-3",
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
            // A later disk version exists, but the editor must not pick it up.
            return new Response(
              JSON.stringify({
                note: {
                  title: "Home",
                  slug: "home",
                  relative_path: "Home",
                  content: "# Home\nOriginal",
                  content_hash: noteCalls === 1 ? "hash-1" : "hash-2",
                },
              }),
              { status: 200 },
            );
          }
          if (url.includes("/api/resolve-batch")) {
            return new Response(JSON.stringify({ results: [] }), {
              status: 200,
            });
          }
          return new Response("not found", { status: 404 });
        },
      );

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Edit note" }));

    const textarea = await screen.findByRole("textbox", {
      name: "Markdown content",
    });
    fireEvent.change(textarea, { target: { value: "# Home\nMine" } });

    // A vault revision arrives while the editor is open.
    act(() => {
      window.__hatchdoorEventSources[0].emit(
        "vault-revision",
        JSON.stringify({ revision: 1 }),
      );
    });

    // The editor stays open and offers an explicit reload rather than silently
    // swapping the base version under the user.
    expect(
      await screen.findByRole("button", { name: "Reload latest" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "Markdown content" }),
    ).toHaveValue("# Home\nMine");

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/note/home",
        expect.objectContaining({
          method: "PUT",
          body: JSON.stringify({
            content: "# Home\nMine",
            expected_content_hash: "hash-1",
          }),
        }),
      );
    });
  });

  it("warns and offers reload for a stale recovered draft", async () => {
    window.localStorage.setItem(
      noteDraftKey("home"),
      JSON.stringify({
        slug: "home",
        content: "# Home\nStale draft",
        baseContentHash: "old-hash",
        savedAt: Date.now(),
      }),
    );
    mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Edit note" }));

    expect(
      await screen.findByText(/earlier draft based on a previous version/i),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Reload latest" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "Markdown content" }),
    ).toHaveValue("# Home\nStale draft");
  });

  it("shows a disk-versus-draft diff when saving hits a conflict", async () => {
    let noteCalls = 0;
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";

        if (url.includes("/api/write-capabilities")) {
          return new Response(JSON.stringify({ enabled: true, warnings: [] }), {
            status: 200,
          });
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
        if (url.endsWith("/api/note/home") && method === "PUT") {
          return new Response(JSON.stringify({ error: "conflict" }), {
            status: 409,
          });
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
                  noteCalls === 1 ? "# Home\nOriginal" : "# Home\nDisk edit",
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
      },
    );

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Edit note" }));

    fireEvent.change(
      screen.getByRole("textbox", { name: "Markdown content" }),
      {
        target: { value: "# Home\nMy draft" },
      },
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(
      await screen.findByRole("region", { name: "Conflict review" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Disk edit")).toBeInTheDocument();
    expect(screen.getByText("My draft")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Discard draft and use disk" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Keep draft on latest" }),
    ).toBeInTheDocument();
  });

  it("rejects path traversal before issuing a create request", async () => {
    const fetchMock = mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "New note" }));
    // The picker only offers folders that exist, so a traversal attempt has to
    // come through the free-text "New folder" path. Client validation must
    // still reject it, and the backend remains authoritative regardless.
    fireEvent.change(screen.getByLabelText("Folder"), {
      target: { value: "//new-folder" },
    });
    fireEvent.change(screen.getByLabelText("New folder name"), {
      target: { value: ".." },
    });
    fireEvent.change(screen.getByLabelText("Note name"), {
      target: { value: "escape" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    expect(
      await screen.findByText(/must not contain "\.\."/),
    ).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalledWith(
      "/api/note",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("inserts a wikilink from autocomplete suggestions", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/write-capabilities")) {
          return new Response(JSON.stringify({ enabled: true, warnings: [] }), {
            status: 200,
          });
        }
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [
                { title: "Home", slug: "home" },
                { title: "Project Plan", slug: "project-plan" },
              ],
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
        if (url.includes("/api/note/home")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content: "# Home\nOriginal",
                content_hash: "hash-1",
              },
            }),
            { status: 200 },
          );
        }
        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    const textarea = (await screen.findByRole("textbox", {
      name: "Markdown content",
    })) as HTMLTextAreaElement;
    fireEvent.change(textarea, {
      target: { value: "link to [[Pro", selectionStart: 13, selectionEnd: 13 },
    });

    const option = await screen.findByRole("option", { name: "Project Plan" });
    fireEvent.mouseDown(option);

    expect(textarea).toHaveValue("link to [[Project Plan]]");
  });

  it("toggles a live preview of the draft in the editor", async () => {
    mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    const textarea = await screen.findByRole("textbox", {
      name: "Markdown content",
    });
    fireEvent.change(textarea, { target: { value: "# Heading\n\nBody text" } });
    fireEvent.click(screen.getByRole("tab", { name: "Preview" }));

    expect(
      await screen.findByRole("heading", { level: 1, name: "Heading" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Body text")).toBeInTheDocument();
    expect(
      screen.queryByRole("textbox", { name: "Markdown content" }),
    ).toBeNull();
  });

  it("prefills the folder when creating from a folder row", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/write-capabilities")) {
          return new Response(JSON.stringify({ enabled: true, warnings: [] }), {
            status: 200,
          });
        }
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [{ name: "Projects", folders: [], notes: [] }],
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
        if (url.includes("/api/note/home")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content: "# Home\nOriginal",
                content_hash: "hash-1",
              },
            }),
            { status: 200 },
          );
        }
        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(
      await screen.findByRole("button", { name: "New note in Projects" }),
    );

    expect(screen.getByLabelText("Folder")).toHaveValue("Projects");
  });

  it("surfaces write side effects after a rename", async () => {
    mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Rename note" }),
    );
    fireEvent.change(screen.getByLabelText("New title"), {
      target: { value: "Renamed Note" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Rename" }));

    expect(
      await screen.findByText("Updated 1 linking note."),
    ).toBeInTheDocument();
  });
});

describe("touch editing hint", () => {
  // Entering a block on touch is a double tap, which is invisible: the gutter
  // rule says "something is here" without saying what gesture reaches it.
  function mockPointer(coarse: boolean) {
    vi.stubGlobal(
      "matchMedia",
      (query: string) =>
        ({
          matches: coarse && query.includes("coarse"),
          media: query,
          addEventListener: () => {},
          removeEventListener: () => {},
        }) as unknown as MediaQueryList,
    );
  }

  const HINT = "Double-tap a line to edit it.";

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("shows the hint on a coarse pointer", async () => {
    mockReadAndWriteApi();
    mockPointer(true);

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText(HINT)).toBeInTheDocument();
  });

  it("does not show it on a pointer that can hover", async () => {
    mockReadAndWriteApi();
    mockPointer(false);

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    expect(screen.queryByText(HINT)).toBeNull();
  });

  it("does not show it again once it has been dismissed", async () => {
    mockReadAndWriteApi();
    mockPointer(true);
    window.localStorage.setItem("hatchdoor.touchEditHintSeen", "1");

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    expect(screen.queryByText(HINT)).toBeNull();
  });

  // Retired on a landed edit rather than on entry, so an accidental double tap
  // does not count as having taught the gesture.
  it("retires the hint once an edit lands", async () => {
    mockReadAndWriteApi();
    mockPointer(true);

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByText(HINT);
    const block = screen.getByText("Original");
    for (let i = 0; i < 2; i += 1) {
      fireEvent.pointerDown(block, {
        pointerType: "touch",
        clientX: 10,
        clientY: 10,
        bubbles: true,
      });
      fireEvent.click(block, { clientX: 10, clientY: 10, bubbles: true });
    }
    const input = await screen.findByRole("textbox");
    fireEvent.change(input, { target: { value: "Edited" } });
    fireEvent.blur(input);

    await waitFor(() => {
      expect(screen.queryByText(HINT)).toBeNull();
    });
    expect(window.localStorage.getItem("hatchdoor.touchEditHintSeen")).toBe(
      "1",
    );
  });

  it("remembers the dismissal when tapped", async () => {
    mockReadAndWriteApi();
    mockPointer(true);

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    // By its label, not by the notice text: dismissal has to be a visible
    // control, since on touch there is no cursor to reveal that the line itself
    // is clickable.
    await screen.findByText(HINT);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss hint" }));

    expect(screen.queryByText(HINT)).toBeNull();
    expect(window.localStorage.getItem("hatchdoor.touchEditHintSeen")).toBe(
      "1",
    );
  });
});

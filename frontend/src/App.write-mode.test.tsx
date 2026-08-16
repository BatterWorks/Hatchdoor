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

import { EditorView } from "@codemirror/view";

import { VaultApp as App } from "./App";
import { noteDraftKey } from "./lib/writeDrafts";
import { discoveryResponse, healthyVault } from "./test/fixtures/vaults";

const VAULT = healthyVault("Vault");
const VAULT_ID = VAULT.vault_id;
const NOTE_URL = `/api/v1/vaults/${VAULT_ID}/notes/home`;
const NOTES_URL = `/api/v1/vaults/${VAULT_ID}/notes`;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function collectionEnvelope(data: unknown): Response {
  return jsonResponse({
    scope: "all",
    collection_revision: 1,
    partial: false,
    participants: [
      { vault_id: VAULT_ID, vault_name: VAULT.name, state: "fresh" },
    ],
    data,
  });
}

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

        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }

        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: true,
            warnings: [],
          });
        }

        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
              },
            },
          ]);
        }

        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }

        if (url.endsWith(NOTES_URL) && method === "POST") {
          return jsonResponse({
            vault_id: VAULT_ID,
            ok: true,
            slug: "projects-new-note",
            relative_path: "Projects/New Note",
            content_hash: "hash-new",
            quality_warnings: [],
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: null,
            layer: null,
          });
        }

        if (url.includes("/notes/home/rename") && method === "PATCH") {
          return jsonResponse({
            vault_id: VAULT_ID,
            ok: true,
            slug: "renamed-note",
            relative_path: "Renamed Note",
            content_hash: "hash-renamed",
            quality_warnings: [],
            rewritten_notes: 1,
            moved_assets: 0,
            trashed_path: null,
            layer: null,
          });
        }

        if (url.includes("/notes/home/move") && method === "PATCH") {
          return jsonResponse({
            vault_id: VAULT_ID,
            ok: true,
            slug: "archive-home",
            relative_path: "Archive/Home",
            content_hash: "hash-moved",
            quality_warnings: [],
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: null,
            layer: null,
          });
        }

        if (url.includes("/notes/home/archive") && method === "PATCH") {
          return jsonResponse({
            vault_id: VAULT_ID,
            ok: true,
            slug: "archive-home",
            relative_path: "90-archive/Home",
            content_hash: "hash-archived",
            quality_warnings: [],
            rewritten_notes: 1,
            moved_assets: 0,
            trashed_path: null,
            layer: null,
          });
        }

        if (url.endsWith("/notes/home") && method === "DELETE") {
          return jsonResponse({
            vault_id: VAULT_ID,
            ok: true,
            slug: "home",
            relative_path: "Home",
            content_hash: "hash-1",
            quality_warnings: [],
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: "90-archive/Home.md",
            layer: null,
          });
        }

        if (url.includes("/notes/home") && method === "GET") {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "# Home\nOriginal",
              content_hash: "hash-1",
              layer: null,
            },
          });
        }

        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
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

          if (url.endsWith("/api/v1/vaults")) {
            return jsonResponse(discoveryResponse([VAULT]));
          }

          if (url.includes("/write-capabilities")) {
            return jsonResponse({
              vault_id: VAULT_ID,
              enabled: true,
              warnings: [],
            });
          }

          if (url.includes("/tree")) {
            return collectionEnvelope([
              {
                vault_id: VAULT_ID,
                vault_name: VAULT.name,
                tree: {
                  name: "Vault",
                  folders: [],
                  notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
                },
              },
            ]);
          }

          if (url.includes("/recent")) {
            return collectionEnvelope([]);
          }

          if (url.includes("/notes/home/links")) {
            return jsonResponse({
              vault_id: VAULT_ID,
              outgoing: [],
              backlinks: [],
            });
          }

          if (url.endsWith("/notes/home") && init?.method === "PUT") {
            return jsonResponse({
              vault_id: VAULT_ID,
              ok: true,
              slug: "home",
              relative_path: "Home",
              content_hash: "hash-2",
              quality_warnings: [],
              rewritten_notes: 0,
              moved_assets: 0,
              trashed_path: null,
              layer: null,
            });
          }

          if (url.includes("/notes/home")) {
            noteCalls += 1;
            return jsonResponse({
              vault_id: VAULT_ID,
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content:
                  noteCalls === 1 ? "# Home\nOriginal" : "# Home\nUpdated",
                content_hash: noteCalls === 1 ? "hash-1" : "hash-2",
                layer: null,
              },
            });
          }

          if (url.includes("/resolve-batch")) {
            return jsonResponse({ vault_id: VAULT_ID, results: [] });
          }

          return new Response("not found", { status: 404 });
        },
      );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
        NOTE_URL,
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
    fireEvent.click(screen.getByRole("button", { name: "Create and open" }));

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        NOTES_URL,
        expect.objectContaining({
          method: "POST",
          // Created empty: the note is written in place once it opens, so the
          // dialog collects a destination and nothing else.
          body: JSON.stringify({
            relative_path: "Projects/New Note",
            content: "",
          }),
        }),
      );
    });
  });

  it("opens the rename dialog from the actions menu", async () => {
    mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
        `${NOTE_URL}/archive`,
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

          if (url.endsWith("/api/v1/vaults")) {
            return jsonResponse(discoveryResponse([VAULT]));
          }

          if (url.includes("/write-capabilities")) {
            return jsonResponse({
              vault_id: VAULT_ID,
              enabled: true,
              warnings: [],
            });
          }
          if (url.includes("/tree")) {
            return collectionEnvelope([
              {
                vault_id: VAULT_ID,
                vault_name: VAULT.name,
                tree: {
                  name: "Vault",
                  folders: [],
                  notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
                },
              },
            ]);
          }
          if (url.includes("/recent")) {
            return collectionEnvelope([]);
          }
          if (url.includes("/notes/home/links")) {
            return jsonResponse({
              vault_id: VAULT_ID,
              outgoing: [],
              backlinks: [],
            });
          }
          if (url.endsWith("/notes/home") && init?.method === "PUT") {
            return jsonResponse({
              vault_id: VAULT_ID,
              ok: true,
              slug: "home",
              relative_path: "Home",
              content_hash: "hash-3",
              quality_warnings: [],
              rewritten_notes: 0,
              moved_assets: 0,
              trashed_path: null,
              layer: null,
            });
          }
          if (url.includes("/notes/home")) {
            noteCalls += 1;
            // A later disk version exists, but the editor must not pick it up.
            return jsonResponse({
              vault_id: VAULT_ID,
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content: "# Home\nOriginal",
                content_hash: noteCalls === 1 ? "hash-1" : "hash-2",
                layer: null,
              },
            });
          }
          if (url.includes("/resolve-batch")) {
            return jsonResponse({ vault_id: VAULT_ID, results: [] });
          }
          return new Response("not found", { status: 404 });
        },
      );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
        "vault-collection-revision",
        JSON.stringify({ collection_revision: 1, vault_ids: [VAULT_ID] }),
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
        NOTE_URL,
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
      noteDraftKey(VAULT_ID, "home"),
      JSON.stringify({
        vaultId: VAULT_ID,
        slug: "home",
        content: "# Home\nStale draft",
        baseContentHash: "old-hash",
        savedAt: Date.now(),
      }),
    );
    mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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

        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: true,
            warnings: [],
          });
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }
        if (url.endsWith("/notes/home") && method === "PUT") {
          return jsonResponse(
            { code: "write_conflict", message: "Conflict", retryable: true },
            409,
          );
        }
        if (url.includes("/notes/home")) {
          noteCalls += 1;
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content:
                noteCalls === 1 ? "# Home\nOriginal" : "# Home\nDisk edit",
              content_hash: noteCalls === 1 ? "hash-1" : "hash-2",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
    fireEvent.click(screen.getByRole("button", { name: "Create and open" }));

    expect(
      await screen.findByText(/must not contain "\.\."/),
    ).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalledWith(
      NOTES_URL,
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("inserts a wikilink from autocomplete suggestions", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: true,
            warnings: [],
          });
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes: [
                  { vault_id: VAULT_ID, title: "Home", slug: "home" },
                  {
                    vault_id: VAULT_ID,
                    title: "Project Plan",
                    slug: "project-plan",
                  },
                ],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "# Home\nOriginal",
              content_hash: "hash-1",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));

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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));

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
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: true,
            warnings: [],
          });
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [{ name: "Projects", folders: [], notes: [] }],
                notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "# Home\nOriginal",
              content_hash: "hash-1",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    expect(await screen.findByText(HINT)).toBeInTheDocument();
  });

  it("does not show it on a pointer that can hover", async () => {
    mockReadAndWriteApi();
    mockPointer(false);

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
    // The open block is a CodeMirror editor, so its text is editor state
    // rather than a DOM value.
    const view = EditorView.findFromDOM(input as HTMLElement)!;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: "Edited" },
    });
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
  // Dropping a file while a block is open uploaded the attachment and then
  // silently lost its embed: the drop wrote the document it had computed from
  // the pre-edit content, and the open block's own commit, seeded before the
  // drop, landed second and overwrote it. Both writes returned 200, so nothing
  // surfaced the loss and the attachment was left orphaned in the vault.
  it("keeps the embed when a file is dropped while a block is open", async () => {
    const writes: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";

        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: true,
            warnings: [],
          });
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }
        if (url.includes("/attachments") && method === "POST") {
          return jsonResponse({
            vault_id: VAULT_ID,
            ok: true,
            attachment: {
              relative_path: "Attachments/report.pdf",
              size_bytes: 4,
              content_hash: "hash-att",
              layer: null,
            },
            rewritten_notes: 0,
            trashed_path: null,
            cleanup_warning: null,
          });
        }
        if (url.endsWith("/notes/home") && method === "PUT") {
          writes.push(JSON.parse(String(init?.body)).content as string);
          return jsonResponse({
            vault_id: VAULT_ID,
            ok: true,
            slug: "home",
            relative_path: "Home",
            content_hash: `hash-${writes.length + 1}`,
            quality_warnings: [],
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: null,
            layer: null,
          });
        }
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "# Home\nOriginal",
              content_hash: "hash-1",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    // Open the paragraph and type into it without leaving the block, so the
    // edit is still uncommitted when the file lands. Waiting for the "Edit"
    // button first ensures write mode (which now depends on a Vault
    // discovery round trip before write-capabilities can even be requested)
    // has actually turned on before the block is clicked.
    await screen.findByRole("button", { name: "Edit" });
    const block = screen.getByText("Original");
    fireEvent.click(block);
    const input = await screen.findByRole("textbox");
    const view = EditorView.findFromDOM(input as HTMLElement)!;
    act(() => {
      view.dispatch({
        changes: {
          from: 0,
          to: view.state.doc.length,
          insert: "Original edited",
        },
      });
    });

    const file = new File(["%PDF"], "report.pdf", { type: "application/pdf" });
    const dropTarget = document.querySelector(".note-body-drop")!;
    await act(async () => {
      fireEvent.drop(dropTarget, {
        dataTransfer: { files: [file] },
        clientY: 10,
      });
    });

    await waitFor(() => {
      expect(writes.length).toBeGreaterThan(0);
    });
    // Whatever order the writes land in, the last one is what the vault keeps,
    // and it has to carry both the embed and the edit.
    await waitFor(() => {
      const latest = writes[writes.length - 1];
      expect(latest).toContain("![[Attachments/report.pdf]]");
      expect(latest).toContain("Original edited");
    });
  });

  it("shows the app's own notice, not the generic autosave-error banner, when a block-editor autosave hits demo_read_only (#152)", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";

        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: true,
            warnings: [],
          });
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }
        if (url.endsWith("/notes/home") && method === "PUT") {
          return jsonResponse(
            {
              code: "demo_read_only",
              message:
                "This is a public read-only demo instance; mutations and Vault-control operations are disabled.",
              retryable: false,
            },
            403,
          );
        }
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "# Home\nOriginal",
              content_hash: "hash-1",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await screen.findByRole("button", { name: "Edit" });
    const block = screen.getByText("Original");
    fireEvent.click(block);
    const input = await screen.findByRole("textbox");
    const view = EditorView.findFromDOM(input as HTMLElement)!;
    act(() => {
      view.dispatch({
        changes: {
          from: 0,
          to: view.state.doc.length,
          insert: "Original edited",
        },
      });
    });
    await act(async () => {
      fireEvent.blur(input);
    });

    await screen.findByText(
      "This is a public read-only demo, so that change was not saved.",
    );
    expect(
      screen.queryByText(
        "Edits aren't saving. Hatchdoor could not reach the vault.",
      ),
    ).not.toBeInTheDocument();
  });
});

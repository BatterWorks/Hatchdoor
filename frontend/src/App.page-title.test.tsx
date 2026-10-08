import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultApp as App } from "./App";
import { discoveryResponse, healthyVault } from "./test/fixtures/vaults";

const VAULT = healthyVault("Vault");
const VAULT_ID = VAULT.vault_id;
const NOTES = [
  { title: "Home", slug: "home" },
  { title: "Harbour", slug: "harbour" },
];

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

function mockInstance(demoMode: boolean) {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input: RequestInfo | URL) => {
      const url = String(input);

      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse(discoveryResponse([VAULT], demoMode));
      }
      if (url.includes("/write-capabilities")) {
        return jsonResponse({ enabled: false });
      }
      if (url.includes("/tree")) {
        return collectionEnvelope([
          {
            vault_id: VAULT_ID,
            vault_name: VAULT.name,
            tree: {
              name: "Vault",
              folders: [],
              notes: NOTES.map((note) => ({ vault_id: VAULT_ID, ...note })),
            },
          },
        ]);
      }
      if (url.includes("/recent")) {
        return collectionEnvelope([]);
      }
      if (url.includes("/resolve-batch")) {
        return jsonResponse({ vault_id: VAULT_ID, results: [] });
      }
      if (url.endsWith("/links")) {
        return jsonResponse({
          vault_id: VAULT_ID,
          outgoing: [],
          backlinks: [],
        });
      }
      const note = NOTES.find((entry) => url.endsWith(`/notes/${entry.slug}`));
      if (note) {
        return jsonResponse({
          vault_id: VAULT_ID,
          note: {
            ...note,
            relative_path: note.title,
            content: `# ${note.title}`,
            content_hash: `hash-${note.slug}`,
            layer: null,
          },
        });
      }
      return new Response("not found", { status: 404 });
    },
  );
}

function renderAt(path: string) {
  render(
    <MemoryRouter initialEntries={[path]}>
      <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
    </MemoryRouter>,
  );
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
  document.title = "";
});

describe.each([
  ["an ordinary instance", false],
  ["a demo instance", true],
])("Browser tab title on %s (#514)", (_name, demoMode) => {
  it("names the open note and follows a move to another note", async () => {
    mockInstance(demoMode);
    renderAt(`/v/${VAULT_ID}/n/home`);
    expect(document.title).toBe("Hatchdoor");

    await screen.findByRole("heading", { level: 2, name: "Home" });
    await waitFor(() => expect(document.title).toBe("Home · Hatchdoor"));

    const [harbour] = await screen.findAllByRole("link", {
      name: "Harbour",
    });
    fireEvent.click(harbour);
    await screen.findByRole("heading", { level: 2, name: "Harbour" });
    await waitFor(() => expect(document.title).toBe("Harbour · Hatchdoor"));
  });

  it("stays on the bare name for a note that fails to load", async () => {
    mockInstance(demoMode);
    renderAt(`/v/${VAULT_ID}/n/missing`);

    await screen.findByText("Note Unavailable");
    expect(document.title).toBe("Hatchdoor");
  });

  it("names Settings, or the home page a demo instance sends it to", async () => {
    mockInstance(demoMode);
    renderAt("/settings");

    await waitFor(() =>
      expect(document.title).toBe(
        demoMode ? "Hatchdoor" : "Settings · Hatchdoor",
      ),
    );
    if (demoMode) {
      expect(document.querySelector(".settings-page")).not.toBeInTheDocument();
    }
  });

  it("names a page that is not a note", async () => {
    mockInstance(demoMode);
    renderAt("/no-such-page");

    await screen.findByText("Page Not Found");
    expect(document.title).toBe("Page not found · Hatchdoor");
  });
});

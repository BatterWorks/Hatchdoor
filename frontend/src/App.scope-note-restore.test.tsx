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
import { LAST_NOTE_BY_VAULT_KEY, LAST_NOTE_KEY } from "./app/constants";
import { discoveryResponse, THREE_VAULTS } from "./test/fixtures/vaults";

const [ALPHA, BETA] = THREE_VAULTS;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function collectionEnvelope(data: unknown): Response {
  return jsonResponse({
    scope: "all",
    collection_revision: 1,
    partial: false,
    participants: THREE_VAULTS.map((vault) => ({
      vault_id: vault.vault_id,
      vault_name: vault.name,
      state: "fresh",
    })),
    data,
  });
}

/** One note per Vault, named after it: `alpha-home` in Alpha, `beta-home` in
 * Beta, nothing at all in Gamma. */
function mockThreeVaultFetch() {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse(discoveryResponse(THREE_VAULTS));
      }
      if (url.includes("/tree")) {
        return collectionEnvelope(
          THREE_VAULTS.map((vault) => ({
            vault_id: vault.vault_id,
            vault_name: vault.name,
            tree: { name: vault.name, folders: [], notes: [] },
          })),
        );
      }
      if (url.includes("/recent")) {
        return collectionEnvelope([]);
      }
      if (url.includes("/links")) {
        return jsonResponse({ outgoing: [], backlinks: [] });
      }
      if (url.includes("/resolve-batch")) {
        return jsonResponse({ results: [] });
      }
      if (url.includes("/write-capabilities")) {
        return jsonResponse({ enabled: false, warnings: [] });
      }

      const note = /\/vaults\/([^/]+)\/notes\/([^/?]+)/.exec(url);
      if (note) {
        const [, vaultId, slug] = note;
        const title = slug === "alpha-home" ? "Alpha Home" : "Beta Home";
        return jsonResponse({
          vault_id: vaultId,
          note: {
            title,
            slug,
            relative_path: title,
            content: `# ${title}`,
            content_hash: `hash-${slug}`,
            layer: null,
          },
        });
      }

      return jsonResponse({ error: "not found" }, 404);
    },
  );
}

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
    </MemoryRouter>,
  );
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("switching the browsing scope restores that Vault's own note", () => {
  it("opens the note the picked Vault was last left on", async () => {
    window.localStorage.setItem(
      LAST_NOTE_BY_VAULT_KEY,
      JSON.stringify({
        [ALPHA.vault_id]: "alpha-home",
        [BETA.vault_id]: "beta-home",
      }),
    );
    mockThreeVaultFetch();

    renderAt(`/v/${ALPHA.vault_id}/n/alpha-home`);
    await screen.findByRole("heading", { level: 2, name: "Alpha Home" });

    fireEvent.click(await screen.findByRole("radio", { name: /^Beta/ }));

    expect(
      await screen.findByRole("heading", { level: 2, name: "Beta Home" }),
    ).toBeInTheDocument();
  });

  it("lands on the empty state when the picked Vault has no remembered note", async () => {
    window.localStorage.setItem(
      LAST_NOTE_BY_VAULT_KEY,
      JSON.stringify({ [ALPHA.vault_id]: "alpha-home" }),
    );
    // The landing redirect would otherwise restore Alpha's note the instant
    // the switch puts the reader on "/", undoing the switch.
    window.localStorage.setItem(
      LAST_NOTE_KEY,
      JSON.stringify({ vaultId: ALPHA.vault_id, slug: "alpha-home" }),
    );
    mockThreeVaultFetch();

    renderAt(`/v/${ALPHA.vault_id}/n/alpha-home`);
    await screen.findByRole("heading", { level: 2, name: "Alpha Home" });

    fireEvent.click(await screen.findByRole("radio", { name: /^Gamma/ }));

    await waitFor(() =>
      expect(
        screen.queryByRole("heading", { level: 2, name: "Alpha Home" }),
      ).not.toBeInTheDocument(),
    );
    // Give the landing redirect a turn to fire before trusting the result.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      screen.queryByRole("heading", { level: 2, name: "Alpha Home" }),
    ).not.toBeInTheDocument();
  });

  it("leaves a reader who is not on a note page where they are", async () => {
    window.localStorage.setItem(
      LAST_NOTE_BY_VAULT_KEY,
      JSON.stringify({ [BETA.vault_id]: "beta-home" }),
    );
    mockThreeVaultFetch();

    renderAt("/stats");
    await screen.findByRole("radio", { name: /^Beta/ });

    fireEvent.click(screen.getByRole("radio", { name: /^Beta/ }));

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      screen.queryByRole("heading", { level: 2, name: "Beta Home" }),
    ).not.toBeInTheDocument();
    expect(screen.getAllByText("Stats Unavailable").length).toBeGreaterThan(0);
  });

  it("records the open note against its own Vault as it is read", async () => {
    mockThreeVaultFetch();

    renderAt(`/v/${BETA.vault_id}/n/beta-home`);
    await screen.findByRole("heading", { level: 2, name: "Beta Home" });

    await waitFor(() =>
      expect(
        JSON.parse(window.localStorage.getItem(LAST_NOTE_BY_VAULT_KEY) ?? "{}"),
      ).toEqual({ [BETA.vault_id]: "beta-home" }),
    );
  });

  it("forgets the remembered note of a Vault that has left the collection", async () => {
    window.localStorage.setItem(
      LAST_NOTE_BY_VAULT_KEY,
      JSON.stringify({
        [ALPHA.vault_id]: "alpha-home",
        "departed-vault": "gone",
      }),
    );
    mockThreeVaultFetch();

    renderAt(`/v/${ALPHA.vault_id}/n/alpha-home`);
    await screen.findByRole("heading", { level: 2, name: "Alpha Home" });

    await waitFor(() =>
      expect(
        JSON.parse(window.localStorage.getItem(LAST_NOTE_BY_VAULT_KEY) ?? "{}"),
      ).toEqual({ [ALPHA.vault_id]: "alpha-home" }),
    );
  });
});

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultApp as App } from "./App";
import { SCOPE_ZONE_COLLAPSED_KEY } from "./app/constants";
import { discoveryResponse, THREE_VAULTS } from "./test/fixtures/vaults";

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
      return jsonResponse({ error: "not found" }, 404);
    },
  );
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("App scope shortcut (#146)", () => {
  it("`v` unfolds a collapsed Scope zone and focuses the current row", async () => {
    window.localStorage.setItem(SCOPE_ZONE_COLLAPSED_KEY, "1");
    mockThreeVaultFetch();

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    const head = await screen.findByRole("button", { name: /Scope/ });
    expect(head).toHaveAttribute("aria-expanded", "false");

    fireEvent.keyDown(window, { key: "v" });

    await waitFor(() => {
      expect(head).toHaveAttribute("aria-expanded", "true");
    });
    const selectedRow = await screen.findByRole("radio", {
      name: /^All Vaults/,
    });
    expect(selectedRow).toHaveFocus();
  });

  it("pressing `v` again after arrowing away re-homes focus on the current row (harmless repeat)", async () => {
    mockThreeVaultFetch();

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("radio", { name: /^All Vaults/ });
    fireEvent.keyDown(window, { key: "v" });
    const allVaultsRow = await screen.findByRole("radio", {
      name: /^All Vaults/,
    });
    expect(allVaultsRow).toHaveFocus();

    fireEvent.keyDown(allVaultsRow, { key: "ArrowDown" });
    const alphaRow = screen.getByRole("radio", { name: /^Alpha/ });
    expect(alphaRow).toHaveFocus();

    fireEvent.keyDown(window, { key: "v" });
    expect(allVaultsRow).toHaveFocus();
  });

  it("a repeat `v` press does not clobber the origin — Escape still returns to the first press's location", async () => {
    mockThreeVaultFetch();

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("radio", { name: /^All Vaults/ });
    const searchTrigger = screen.getByRole("button", { name: "Search" });
    searchTrigger.focus();

    fireEvent.keyDown(window, { key: "v" });
    const allVaultsRow = await screen.findByRole("radio", {
      name: /^All Vaults/,
    });
    expect(allVaultsRow).toHaveFocus();

    // A second `v` press while already inside the zone is "harmless" — it
    // must not overwrite the origin with the row itself.
    fireEvent.keyDown(window, { key: "v" });
    expect(allVaultsRow).toHaveFocus();

    fireEvent.keyDown(allVaultsRow, { key: "Escape" });
    expect(searchTrigger).toHaveFocus();
  });

  it("Escape after `v` with no pick returns focus to where `v` was pressed", async () => {
    mockThreeVaultFetch();

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    // Wait for Vault discovery to land before touching focus, so `v` is not
    // read while `vaults.length <= 1` still holds and silently no-ops.
    await screen.findByRole("radio", { name: /^All Vaults/ });

    const searchTrigger = screen.getByRole("button", { name: "Search" });
    searchTrigger.focus();
    expect(searchTrigger).toHaveFocus();

    fireEvent.keyDown(window, { key: "v" });
    const selectedRow = await screen.findByRole("radio", {
      name: /^All Vaults/,
    });
    expect(selectedRow).toHaveFocus();

    fireEvent.keyDown(selectedRow, { key: "Escape" });
    expect(searchTrigger).toHaveFocus();
  });

  it("does nothing while the search dialog is open — types into the field instead", async () => {
    window.localStorage.setItem(SCOPE_ZONE_COLLAPSED_KEY, "1");
    mockThreeVaultFetch();

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    const head = await screen.findByRole("button", { name: /Scope/ });
    expect(head).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    const input = await screen.findByPlaceholderText("Search notes…");
    input.focus();

    fireEvent.keyDown(input, { key: "v" });

    expect(input).toHaveFocus();
    expect(head).toHaveAttribute("aria-expanded", "false");
  });

  it("never announces the pre-discovery placeholder count, even while Vault discovery is still in flight", async () => {
    let resolveDiscovery: (response: Response) => void = () => {};
    const discoveryPromise = new Promise<Response>((resolve) => {
      resolveDiscovery = resolve;
    });
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return discoveryPromise;
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    // Discovery hasn't resolved yet — `vaults` is a temporary `[]`. The live
    // region must stay silent rather than announce "0 Vaults" off it.
    const liveRegion = document.querySelector(".visually-hidden");
    expect(liveRegion).toHaveTextContent("");

    resolveDiscovery(jsonResponse(discoveryResponse(THREE_VAULTS)));

    await screen.findByRole("radio", { name: /^All Vaults/ });
    // Discovery landing is a baseline, not a pick — still silent.
    expect(liveRegion).toHaveTextContent("");
  });

  it("announces the scope name and count when a row is picked", async () => {
    mockThreeVaultFetch();

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    const alphaRow = await screen.findByRole("radio", { name: /^Alpha/ });
    fireEvent.click(alphaRow);

    await waitFor(() => {
      expect(screen.getByText(/^Alpha\./)).toBeInTheDocument();
    });
  });
});

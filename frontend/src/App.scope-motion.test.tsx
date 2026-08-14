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

describe("App scope-change motion (#147)", () => {
  it("scrolls the explorer back to the top when the browsing scope changes", async () => {
    mockThreeVaultFetch();

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    const alphaRow = await screen.findByRole("radio", { name: /^Alpha/ });
    const nav = document.querySelector(".explorer-nav") as HTMLElement;
    expect(nav).not.toBeNull();

    Object.defineProperty(nav, "scrollTop", {
      configurable: true,
      value: 400,
      writable: true,
    });
    expect(nav.scrollTop).toBe(400);

    fireEvent.click(alphaRow);

    await waitFor(() => {
      expect(nav.scrollTop).toBe(0);
    });
  });

  it("does not touch the explorer's scroll position on mount, before any scope change", async () => {
    mockThreeVaultFetch();

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await screen.findByRole("radio", { name: /^Alpha/ });
    const nav = document.querySelector(".explorer-nav") as HTMLElement;

    Object.defineProperty(nav, "scrollTop", {
      configurable: true,
      value: 250,
      writable: true,
    });

    // Give any stray effect a turn to run, then confirm nothing reset it.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(nav.scrollTop).toBe(250);
  });
});

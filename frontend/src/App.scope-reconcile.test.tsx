import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultApp as App } from "./App";
import { VAULT_SCOPE_KEY } from "./app/constants";
import {
  discoveryResponse,
  healthyVault,
  pausedVault,
} from "./test/fixtures/vaults";
import type { VaultSummary } from "./types";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

/** A server whose Vault list can change under the app. A tree or recent read
 * scoped to a Vault that is not enabled is refused the way the backend
 * refuses one, so a stale scope shows up as the explorer's error block. */
function mockServer(initial: VaultSummary[]) {
  const live = { vaults: initial };
  const enabled = () => live.vaults.filter((vault) => vault.enabled);
  const envelope = (scope: string, data: unknown) =>
    jsonResponse({
      scope,
      collection_revision: 1,
      partial: false,
      participants: enabled().map((vault) => ({
        vault_id: vault.vault_id,
        vault_name: vault.name,
        state: "fresh",
      })),
      data,
    });
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse(discoveryResponse(live.vaults));
      }
      if (url.includes("/write-capabilities")) {
        return jsonResponse({ enabled: false, warnings: [] });
      }
      const scoped = /\/api\/v1\/vaults\/([^/]+)\/(tree|recent|stats)/.exec(
        url,
      );
      if (scoped) {
        const [, scope, kind] = scoped;
        if (
          scope !== "all" &&
          !enabled().some((vault) => vault.vault_id === scope)
        ) {
          return jsonResponse(
            { error: "Vault is disabled", code: "vault_disabled" },
            409,
          );
        }
        if (kind === "tree") {
          return envelope(
            scope,
            enabled()
              .filter((vault) => scope === "all" || vault.vault_id === scope)
              .map((vault) => ({
                vault_id: vault.vault_id,
                vault_name: vault.name,
                tree: {
                  name: vault.name,
                  folders: [],
                  notes: [
                    {
                      slug: `${vault.name.toLowerCase()}-home`,
                      title: `${vault.name} Home`,
                      relative_path: `${vault.name} Home.md`,
                    },
                  ],
                },
              })),
          );
        }
        return envelope(scope, []);
      }
      return jsonResponse({ error: "not found" }, 404);
    });
  return { live, fetchMock };
}

function emitRevision(revision: number) {
  for (const source of window.__hatchdoorEventSources) {
    if (source.url.includes("/api/v1/vaults/events")) {
      source.emit(
        "vault-collection-revision",
        JSON.stringify({ collection_revision: revision }),
      );
    }
  }
}

function renderApp() {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
    </MemoryRouter>,
  );
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("the browsing scope is reconciled against the live collection (#335)", () => {
  it("leaves a scope whose Vault was paused while away, even at one enabled Vault", async () => {
    const alpha = healthyVault("Alpha");
    const beta = pausedVault("Beta");
    const { fetchMock } = mockServer([alpha, beta]);
    window.localStorage.setItem(VAULT_SCOPE_KEY, beta.vault_id);

    renderApp();

    // One enabled Vault: no Scope zone to escape with, so the app has to.
    expect(
      await screen.findByText(
        "Beta is paused, so the explorer now shows All Vaults.",
      ),
    ).toBeInTheDocument();
    expect(await screen.findByText("Alpha Home")).toBeInTheDocument();
    expect(screen.queryByText("Explorer Unavailable")).not.toBeInTheDocument();
    expect(window.localStorage.getItem(VAULT_SCOPE_KEY)).toBe("all");
    expect(
      fetchMock.mock.calls.some(([input]) =>
        String(input).includes("/api/v1/vaults/all/tree"),
      ),
    ).toBe(true);
  });

  it("lands in a working scope when the browsed Vault is paused from elsewhere", async () => {
    const alpha = healthyVault("Alpha");
    const beta = healthyVault("Beta");
    const { live } = mockServer([alpha, beta]);
    window.localStorage.setItem(VAULT_SCOPE_KEY, beta.vault_id);

    renderApp();
    expect(await screen.findByText("Beta Home")).toBeInTheDocument();
    expect(screen.queryByText("Alpha Home")).not.toBeInTheDocument();

    // Another tab, or an MCP agent, pauses Beta; this tab hears only the
    // revision.
    live.vaults = [alpha, { ...pausedVault("Beta"), vault_id: beta.vault_id }];
    act(() => emitRevision(50));

    expect(
      await screen.findByText(
        "Beta is paused, so the explorer now shows All Vaults.",
      ),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByText("Alpha Home")).toBeInTheDocument(),
    );
    expect(screen.queryByText("Explorer Unavailable")).not.toBeInTheDocument();
    expect(window.localStorage.getItem(VAULT_SCOPE_KEY)).toBe("all");
  });
});

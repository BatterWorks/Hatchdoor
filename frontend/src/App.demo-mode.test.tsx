import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultApp as App } from "./App";
import { discoveryResponse, healthyVault } from "./test/fixtures/vaults";

const VAULT = healthyVault("Vault");
const VAULT_ID = VAULT.vault_id;

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

/** A demo instance, matching the real server: `GET .../write-capabilities`
 * carries the same `demo_guard` layer every mutation route does, so it
 * 403s with `demo_read_only` rather than answering `enabled: true`. */
function mockDemoInstance() {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input);

      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse(discoveryResponse([VAULT], true));
      }
      if (url.includes("/write-capabilities")) {
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
        return jsonResponse({ vault_id: VAULT_ID, outgoing: [], backlinks: [] });
      }
      if (url.includes("/notes/home") && !url.includes("?")) {
        return jsonResponse({
          vault_id: VAULT_ID,
          note: {
            title: "Home",
            slug: "home",
            relative_path: "Home",
            content: "# Home",
            content_hash: "hash-1",
            layer: null,
          },
        });
      }
      if (url.includes("/resolve-batch")) {
        return jsonResponse({ vault_id: VAULT_ID, results: [] });
      }
      if (url.includes("/all/stats")) {
        return jsonResponse({ data: [] });
      }
      return new Response("not found", { status: 404 });
    });
}

describe("Demo mode (#152)", () => {
  it("shows no write affordance in demo mode, end to end", async () => {
    mockDemoInstance();

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    expect(
      screen.queryByRole("button", { name: "Edit" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "New note" }),
    ).not.toBeInTheDocument();
  });

  it("redirects /settings to the ordinary workspace, silently, rather than rendering Vault management", async () => {
    mockDemoInstance();

    render(
      <MemoryRouter initialEntries={["/settings"]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.queryByText("Vaults")).not.toBeInTheDocument();
    });
    expect(
      screen.queryByRole("heading", { name: /settings/i }),
    ).not.toBeInTheDocument();
  });

  it("never mounts SettingsPage while discovery is still resolving demoMode, even for one frame", async () => {
    let resolveDiscovery!: (response: Response) => void;
    const discoveryPromise = new Promise<Response>((resolve) => {
      resolveDiscovery = resolve;
    });
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return discoveryPromise;
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/settings"]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    // `demoMode` defaults to `false` while this fetch is in flight — the bug
    // this guards against rendered SettingsPage during exactly this window.
    expect(screen.queryByText("Loading settings…")).not.toBeInTheDocument();
    expect(document.querySelector(".settings-page")).not.toBeInTheDocument();

    resolveDiscovery(jsonResponse(discoveryResponse([VAULT], true)));

    await waitFor(() => {
      expect(document.querySelector(".settings-page")).not.toBeInTheDocument();
    });
  });

  it("never renders a clickable sidebar Settings link while discovery is still resolving demoMode (#152)", async () => {
    let resolveDiscovery!: (response: Response) => void;
    const discoveryPromise = new Promise<Response>((resolve) => {
      resolveDiscovery = resolve;
    });
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return discoveryPromise;
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    // `demoMode` defaults to `false` while this fetch is in flight — the
    // sidebar link must fail closed through that gap, not just once
    // `demoMode` itself resolves to `true`.
    expect(
      screen.queryByRole("link", { name: "Settings" }),
    ).not.toBeInTheDocument();

    resolveDiscovery(jsonResponse(discoveryResponse([VAULT], true)));

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "Notes Explorer" })).toBeVisible();
    });
    expect(
      screen.queryByRole("link", { name: "Settings" }),
    ).not.toBeInTheDocument();
  });
});

import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { App as RootApp, VaultApp as App } from "./App";
import type { VaultDiscoveryResponse } from "./types";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function emptyEnvelope(): Response {
  return jsonResponse({
    scope: "all",
    collection_revision: 0,
    partial: false,
    participants: [],
    data: [],
  });
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

function mockDiscovery(discovery: VaultDiscoveryResponse) {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse(discovery);
      }
      if (url.includes("/tree") || url.includes("/recent")) {
        return emptyEnvelope();
      }
      return jsonResponse({}, 404);
    },
  );
}

function renderApp() {
  render(
    <MemoryRouter initialEntries={["/"]}>
      <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
    </MemoryRouter>,
  );
}

describe("VaultApp's zero-Vault and broken-registry note-pane states (#150)", () => {
  it("lets an unreadable registry override a terms-required gate", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse({
            collection_revision: 0,
            vaults: [],
            recovery: {
              code: "vault_registry_recovery_required",
              kind: "corrupt",
              message: "the registry file is not valid JSON",
            },
            demo_mode: false,
          });
        }
        if (url.includes("/tree") || url.includes("/recent")) {
          return emptyEnvelope();
        }
        return jsonResponse({}, 404);
      },
    );
    render(
      <MemoryRouter initialEntries={["/"]}>
        <RootApp />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Vault Registry Unavailable")).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "Set up multilingual search" }),
    ).not.toBeInTheDocument();
    expect(globalThis.fetch).not.toHaveBeenCalledWith(
      "/api/startup-status",
      expect.anything(),
    );
  });

  it("keeps a zero-Vault workspace open without polling or showing a model gate", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse({
            registry_revision: 0,
            collection_revision: 0,
            vaults: [],
            demo_mode: false,
          });
        }
        if (url.includes("/tree") || url.includes("/recent")) {
          return emptyEnvelope();
        }
        return jsonResponse({}, 404);
      },
    );
    render(
      <MemoryRouter initialEntries={["/"]}>
        <RootApp />
      </MemoryRouter>,
    );

    expect(await screen.findByText("No Vaults Yet")).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "Set up multilingual search" }),
    ).not.toBeInTheDocument();
    expect(globalThis.fetch).not.toHaveBeenCalledWith(
      "/api/startup-status",
      expect.anything(),
    );
  });

  it("renders a neutral Add a Vault empty state at genuine zero Vaults", async () => {
    mockDiscovery({
      registry_revision: 0,
      collection_revision: 0,
      vaults: [],
      demo_mode: false,
    });
    renderApp();

    expect(await screen.findByText("No Vaults Yet")).toBeVisible();
    expect(screen.getByRole("button", { name: "Add a Vault" })).toBeVisible();
    expect(
      screen.queryByText("Vault Registry Unavailable"),
    ).not.toBeInTheDocument();
  });

  it("opens the same creation flow as Settings from the zero-Vault Add a Vault button (#153)", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse({
            registry_revision: 0,
            collection_revision: 0,
            vaults: [],
            demo_mode: false,
          });
        }
        if (url.endsWith("/api/v1/vaults/all/stats")) {
          return jsonResponse({ data: [] });
        }
        if (url.includes("/tree") || url.includes("/recent")) {
          return emptyEnvelope();
        }
        return jsonResponse({}, 404);
      },
    );
    render(
      <MemoryRouter initialEntries={["/"]}>
        <RootApp />
      </MemoryRouter>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Add a Vault" }));

    expect(
      await screen.findByRole("dialog", { name: "Add a Vault" }),
    ).toBeVisible();
  });

  it("renders no Add a Vault action in the zero-Vault demo state", async () => {
    mockDiscovery({
      registry_revision: 0,
      collection_revision: 0,
      vaults: [],
      demo_mode: true,
    });
    renderApp();

    expect(await screen.findByText("No Vaults Yet")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Add a Vault" }),
    ).not.toBeInTheDocument();
  });

  it("renders the same empty state for a just-emptied instance as for a brand-new install", async () => {
    mockDiscovery({
      registry_revision: 3,
      collection_revision: 3,
      vaults: [],
      demo_mode: false,
    });
    renderApp();

    expect(await screen.findByText("No Vaults Yet")).toBeVisible();
  });

  it("offers Try again alone for an unreadable registry, with no Start with no Vaults action", async () => {
    mockDiscovery({
      collection_revision: 0,
      vaults: [],
      recovery: {
        code: "vault_registry_recovery_required",
        kind: "corrupt",
        message: "the registry file is not valid JSON",
      },
      demo_mode: false,
    });
    renderApp();

    expect(await screen.findByText("Vault Registry Unavailable")).toBeVisible();
    expect(
      screen.getByText(
        "the registry file is not valid JSON Nothing was changed, and your Markdown is untouched.",
      ),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Try again" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Start with no Vaults" }),
    ).not.toBeInTheDocument();
  });

  it("offers Try again and a confirmed Start with no Vaults for a failed legacy upgrade", async () => {
    mockDiscovery({
      registry_revision: 0,
      collection_revision: 0,
      vaults: [],
      legacy_migration_recovery: {
        code: "legacy_migration_required",
        message: "legacy Vault path is not a readable directory",
      },
      demo_mode: false,
    });
    renderApp();

    expect(await screen.findByText("Vault Registry Unavailable")).toBeVisible();
    expect(screen.getByRole("button", { name: "Try again" })).toBeVisible();
    fireEvent.click(
      screen.getByRole("button", { name: "Start with no Vaults" }),
    );

    expect(
      await screen.findByRole("dialog", { name: "Start with no Vaults" }),
    ).toBeVisible();
    expect(
      screen.getByText(
        "Notes and history are untouched, old settings will be ignored from now on, and the folder must be added by hand.",
      ),
    ).toBeVisible();
  });

  it("confirming Start with no Vaults calls the endpoint and returns to the ordinary zero-Vault state", async () => {
    let confirmed = false;
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults/start-with-no-vaults")) {
          confirmed = true;
          expect(init?.method).toBe("POST");
          return jsonResponse({ vaults: [] });
        }
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse({
            registry_revision: confirmed ? 1 : 0,
            collection_revision: 0,
            vaults: [],
            legacy_migration_recovery: confirmed
              ? undefined
              : {
                  code: "legacy_migration_required",
                  message: "legacy Vault path is not a readable directory",
                },
            demo_mode: false,
          });
        }
        if (url.includes("/tree") || url.includes("/recent")) {
          return emptyEnvelope();
        }
        return jsonResponse({}, 404);
      },
    );
    renderApp();

    fireEvent.click(
      await screen.findByRole("button", { name: "Start with no Vaults" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Start with no Vaults",
    });
    fireEvent.click(
      within(dialog).getByRole("button", { name: "Start with no Vaults" }),
    );

    await waitFor(() => {
      expect(screen.getByText("No Vaults Yet")).toBeVisible();
    });
    expect(
      screen.queryByText("Vault Registry Unavailable"),
    ).not.toBeInTheDocument();
  });
});

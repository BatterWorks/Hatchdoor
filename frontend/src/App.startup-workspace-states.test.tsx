import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { App as RootApp, VaultApp as App } from "./App";
import { LAST_NOTE_KEY } from "./app/constants";
import {
  CONTEXTUAL_HELP,
  type ContextualHelp,
  helpLinkName,
} from "./features/help";
import { HelpContext } from "./features/help/useHelp";
import { discoveryResponse, THREE_VAULTS } from "./test/fixtures/vaults";
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

  it("renders no Add a Vault action in the zero-Vault demo state, with its own sentence in its place (#152)", async () => {
    mockDiscovery({
      registry_revision: 0,
      collection_revision: 0,
      vaults: [],
      demo_mode: true,
    });
    renderApp();

    expect(await screen.findByText("No Vaults Yet")).toBeVisible();
    expect(screen.getByText("This demo has no Vaults loaded.")).toBeVisible();
    expect(
      screen.queryByText(
        "Add a Vault to start browsing and searching your notes.",
      ),
    ).not.toBeInTheDocument();
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

  it("offers Try again for an unreadable registry", async () => {
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
  });
});

describe("VaultApp when Vault discovery fails (#333)", () => {
  const [ALPHA] = THREE_VAULTS;

  /** The three-Vault instance, reachable only while `network.online` holds;
   * offline, every request fails the way a browser's fetch does. */
  function mockThreeVaultServer(network: { online: boolean }) {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        if (!network.online) {
          throw new TypeError("Failed to fetch");
        }
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse(THREE_VAULTS));
        }
        if (url.endsWith("/api/startup-status")) {
          return jsonResponse({ state: "ready" });
        }
        if (url.includes("/tree")) {
          return jsonResponse({
            scope: "all",
            collection_revision: 1,
            partial: false,
            participants: [],
            data: THREE_VAULTS.map((vault) => ({
              vault_id: vault.vault_id,
              vault_name: vault.name,
              tree: { name: vault.name, folders: [], notes: [] },
            })),
          });
        }
        if (url.includes("/recent") || url.includes("/stats")) {
          return emptyEnvelope();
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
          return jsonResponse({
            vault_id: vaultId,
            note: {
              title: "Alpha Home",
              slug,
              relative_path: "Alpha Home",
              content: "# Alpha Home",
              content_hash: `hash-${slug}`,
              layer: null,
            },
          });
        }
        return jsonResponse({}, 404);
      },
    );
  }

  it("renders a server error as an error with Try again, never as No Vaults Yet", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(
            { code: "internal_error", message: "Bad gateway" },
            502,
          );
        }
        return jsonResponse({}, 404);
      },
    );
    renderApp();

    expect(await screen.findByText("Vaults Unavailable")).toBeVisible();
    expect(screen.getByText(/Bad gateway/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Try again" })).toBeVisible();
    expect(screen.queryByText("No Vaults Yet")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Add a Vault" }),
    ).not.toBeInTheDocument();
  });

  it("shows the connection error, not No Vaults Yet, when the app is reloaded offline after loading", async () => {
    const network = { online: true };
    mockThreeVaultServer(network);
    render(
      <MemoryRouter initialEntries={["/"]}>
        <RootApp />
      </MemoryRouter>,
    );
    expect(
      await screen.findByRole("heading", { level: 1, name: "Notes" }),
    ).toBeVisible();

    // Offline, then reload: the whole app mounts afresh with no server.
    cleanup();
    network.online = false;
    render(
      <MemoryRouter initialEntries={["/"]}>
        <RootApp />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Vaults Unavailable")).toBeVisible();
    expect(
      screen.getByText(/Could not reach the Hatchdoor server\./),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Try again" })).toBeVisible();
    expect(screen.queryByText("No Vaults Yet")).not.toBeInTheDocument();

    // Back online, Try again brings the workspace back.
    network.online = true;
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Notes" }),
    ).toBeVisible();
    expect(screen.queryByText("Vaults Unavailable")).not.toBeInTheDocument();
  });

  it("keeps the stored last note through a failed discovery and restores it on recovery", async () => {
    const stored = JSON.stringify({
      vaultId: ALPHA.vault_id,
      slug: "alpha-home",
    });
    window.localStorage.setItem(LAST_NOTE_KEY, stored);
    const network = { online: false };
    mockThreeVaultServer(network);
    renderApp();

    expect(await screen.findByText("Vaults Unavailable")).toBeVisible();
    expect(window.localStorage.getItem(LAST_NOTE_KEY)).toBe(stored);

    network.online = true;
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));

    expect(
      await screen.findByRole("heading", { level: 2, name: "Alpha Home" }),
    ).toBeInTheDocument();
    expect(window.localStorage.getItem(LAST_NOTE_KEY)).toBe(stored);
  });
});

describe("How does this work? links on the start states (#423)", () => {
  const openHelp = vi.fn();

  function renderWithHelp() {
    render(
      <HelpContext.Provider
        value={{ openHelp, closeHelp: () => {}, isOpen: false }}
      >
        <MemoryRouter initialEntries={["/"]}>
          <App
            startupStatus={{ state: "ready" }}
            onRetryModelSetup={() => {}}
          />
        </MemoryRouter>
      </HelpContext.Provider>,
    );
  }

  async function clickLinkUnder(title: string, to: ContextualHelp) {
    const block = (await screen.findByText(title)).closest(".state-block");
    fireEvent.click(
      within(block as HTMLElement).getByRole("button", {
        name: helpLinkName(to),
      }),
    );
  }

  function expectOpened(target: { page: string; heading?: string }) {
    expect(openHelp).toHaveBeenLastCalledWith(
      target.page,
      "heading" in target ? target.heading : undefined,
    );
  }

  it("links No Vaults Yet to connecting a first Vault", async () => {
    mockDiscovery({
      registry_revision: 0,
      collection_revision: 0,
      vaults: [],
      demo_mode: false,
    });
    renderWithHelp();
    await clickLinkUnder("No Vaults Yet", CONTEXTUAL_HELP.noVaults);
    expectOpened(CONTEXTUAL_HELP.noVaults);
  });

  it("leaves the link off a demo with no Vaults, which nobody there can add", async () => {
    mockDiscovery({
      registry_revision: 0,
      collection_revision: 0,
      vaults: [],
      demo_mode: true,
    });
    renderWithHelp();
    await screen.findByText("No Vaults Yet");
    expect(
      screen.queryByRole("button", { name: /^How does this work\?/ }),
    ).not.toBeInTheDocument();
  });

  it("links an unreadable registry to the recovery section", async () => {
    mockDiscovery({
      collection_revision: 0,
      vaults: [],
      recovery: {
        code: "vault_registry_recovery_required",
        kind: "corrupt",
        message: "the registry file is not valid JSON",
      },
      demo_mode: false,
    } as VaultDiscoveryResponse);
    renderWithHelp();
    await clickLinkUnder(
      "Vault Registry Unavailable",
      CONTEXTUAL_HELP.registryRecovery,
    );
    expectOpened(CONTEXTUAL_HELP.registryRecovery);
  });

  it("links Vaults Unavailable to its troubleshooting entry", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
      jsonResponse({ code: "internal_error", message: "Bad gateway" }, 502),
    );
    renderWithHelp();
    await clickLinkUnder(
      "Vaults Unavailable",
      CONTEXTUAL_HELP.vaultsUnavailable,
    );
    expectOpened(CONTEXTUAL_HELP.vaultsUnavailable);
  });
});

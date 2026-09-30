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

import { App as RootApp, VaultApp as App } from "./App";
import { clearToken } from "./api/api";
import type { StartupStatus } from "./startup/useStartupStatus";
import { discoveryResponse, healthyVault } from "./test/fixtures/vaults";

// #339: the demo and startup seams. Each test here drives the real shell
// through the same fetch/SSE seams the browser uses.

const VAULT = healthyVault("Vault");
const VAULT_ID = VAULT.vault_id;
const NOTE_ROUTE = `/v/${VAULT_ID}/n/home`;

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

const DEMO_REFUSAL = {
  code: "demo_read_only",
  message:
    "This is a public read-only demo instance; mutations and Vault-control operations are disabled.",
  retryable: false,
};

type Server = {
  demo: boolean;
  startup: StartupStatus;
  /** When set, `/api/startup-status` waits on it before answering. */
  startupHold?: Promise<void>;
  treeFetches: number;
  retries: number;
  /** `write-capabilities` requests seen so far. */
  capabilityReads: number;
  /** When set, `write-capabilities` fails as a dropped connection would. */
  capabilitiesUnreachable: boolean;
  /** When set, writes and `write-capabilities` are refused `demo_read_only`
   * even though discovery has not reported demo mode yet. */
  refuseWrites: boolean;
  /** When set, every request without this bearer token is refused 401, as a
   * deployment with `HATCHDOOR_WEB_BEARER_TOKEN` does. Startup status stays
   * public there too. */
  requiredToken?: string;
};

/** One Vault holding one note, `home`. `server` is live: flip its fields to
 * change what the next request sees, the way a restart would. */
function mockServer(overrides: Partial<Server> = {}): Server {
  const server: Server = {
    demo: false,
    startup: { state: "ready" },
    treeFetches: 0,
    retries: 0,
    capabilityReads: 0,
    capabilitiesUnreachable: false,
    refuseWrites: false,
    ...overrides,
  };
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      if (
        server.requiredToken &&
        !url.endsWith("/api/startup-status") &&
        new Headers(init?.headers).get("Authorization") !==
          `Bearer ${server.requiredToken}`
      ) {
        return jsonResponse({ error: "unauthorized" }, 401);
      }
      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse(discoveryResponse([VAULT], server.demo));
      }
      if (url.endsWith("/api/startup-status")) {
        if (server.startupHold) {
          await server.startupHold;
        }
        return jsonResponse(server.startup);
      }
      if (url.endsWith("/api/model/retry") && method === "POST") {
        if (server.demo) {
          return new Response("not found", { status: 404 });
        }
        server.retries += 1;
        server.startup = {
          state: "downloading",
          downloaded_bytes: 30,
          total_bytes: 100,
          percent: 30,
        };
        return jsonResponse({});
      }
      if (url.includes("/write-capabilities")) {
        server.capabilityReads += 1;
        if (server.capabilitiesUnreachable) {
          throw new TypeError("Failed to fetch");
        }
        return server.demo || server.refuseWrites
          ? jsonResponse(DEMO_REFUSAL, 403)
          : jsonResponse({ vault_id: VAULT_ID, enabled: true, warnings: [] });
      }
      if (
        server.refuseWrites &&
        method === "POST" &&
        url.endsWith(`/api/v1/vaults/${VAULT_ID}/notes`)
      ) {
        return jsonResponse(DEMO_REFUSAL, 403);
      }
      if (url.includes("/tree")) {
        server.treeFetches += 1;
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
      if (url.includes("/stats")) {
        return jsonResponse({ data: [] });
      }
      if (url.includes("/search")) {
        return collectionEnvelope([]);
      }
      return new Response("not found", { status: 404 });
    },
  );
  return server;
}

afterEach(() => {
  cleanup();
  clearToken();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("write mode re-derives when the backend flips into demo mode (#339)", () => {
  it("drops Edit and New note from an open tab once the collection reports demo mode", async () => {
    const server = mockServer();
    render(
      <MemoryRouter initialEntries={[NOTE_ROUTE]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    expect(await screen.findByRole("button", { name: "Edit" })).toBeVisible();
    expect(screen.getByRole("button", { name: "New note" })).toBeVisible();

    // The server restarts into demo mode; the revision stream reconnects and
    // reports the new server's revision, which re-reads the collection.
    server.demo = true;
    act(() => {
      window.__hatchdoorEventSources[0].emit(
        "vault-collection-revision",
        JSON.stringify({ collection_revision: 7, vault_ids: [] }),
      );
    });

    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "Edit" }),
      ).not.toBeInTheDocument();
    });
    expect(
      screen.queryByRole("button", { name: "New note" }),
    ).not.toBeInTheDocument();
  });
});

describe("write mode re-reads keep or drop Edit for the right reasons (#339)", () => {
  it("keeps Edit when a revision-triggered re-read fails on a dropped connection", async () => {
    const server = mockServer();
    render(
      <MemoryRouter initialEntries={[NOTE_ROUTE]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    expect(await screen.findByRole("button", { name: "Edit" })).toBeVisible();
    const readsBefore = server.capabilityReads;

    server.capabilitiesUnreachable = true;
    act(() => {
      window.__hatchdoorEventSources[0].emit(
        "vault-collection-revision",
        JSON.stringify({ collection_revision: 9, vault_ids: [VAULT_ID] }),
      );
    });

    await waitFor(() => {
      expect(server.capabilityReads).toBeGreaterThan(readsBefore);
    });
    // Let the rejected re-read settle before asserting nothing changed.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(screen.getByRole("button", { name: "Edit" })).toBeVisible();
    expect(screen.getByRole("button", { name: "New note" })).toBeVisible();
  });

  it("re-asks write-capabilities after a demo_read_only refusal and drops Edit once it is refused too", async () => {
    const server = mockServer();
    render(
      <MemoryRouter initialEntries={[NOTE_ROUTE]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    expect(await screen.findByRole("button", { name: "Edit" })).toBeVisible();
    const readsBefore = server.capabilityReads;

    // The posture moved under the tab, but discovery still says
    // `demo_mode: false` and no revision arrives: only the refusal itself
    // can prompt the re-read.
    server.refuseWrites = true;
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "New note" }));
    fireEvent.change(screen.getByLabelText("Note name"), {
      target: { value: "Refused" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create and open" }));

    expect(
      await screen.findByText(
        "This is a public read-only demo, so that change was not saved.",
      ),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(server.capabilityReads).toBeGreaterThan(readsBefore);
    });
    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "Edit" }),
      ).not.toBeInTheDocument();
    });
  });
});

describe("boot with site data blocked (#339)", () => {
  it("renders a readable note when every localStorage access throws, as WebKit does", async () => {
    mockServer();
    const descriptor = Object.getOwnPropertyDescriptor(window, "localStorage");
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      get() {
        throw new DOMException("The operation is insecure.", "SecurityError");
      },
    });
    try {
      render(
        <MemoryRouter initialEntries={[NOTE_ROUTE]}>
          <RootApp />
        </MemoryRouter>,
      );

      expect(
        await screen.findByRole("heading", { level: 2, name: "Home" }),
      ).toBeVisible();
    } finally {
      cleanup();
      if (descriptor) {
        Object.defineProperty(window, "localStorage", descriptor);
      }
    }
  });
});

describe("unlock with site data blocked (#339)", () => {
  it("unlocks in place when the browser refuses to store the token", async () => {
    mockServer({ requiredToken: "secret-token" });
    const reload = vi.fn();
    const location = window.location;
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { ...location, reload },
    });
    const descriptor = Object.getOwnPropertyDescriptor(window, "localStorage");
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      get() {
        throw new DOMException("The operation is insecure.", "SecurityError");
      },
    });
    try {
      render(
        <MemoryRouter initialEntries={[NOTE_ROUTE]}>
          <RootApp />
        </MemoryRouter>,
      );
      fireEvent.change(await screen.findByPlaceholderText("Bearer token"), {
        target: { value: "secret-token" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Unlock" }));

      // A page reload would forget a token only this page holds.
      expect(reload).not.toHaveBeenCalled();
      expect(
        await screen.findByRole("heading", { level: 2, name: "Home" }),
      ).toBeVisible();
      expect(
        screen.queryByRole("dialog", { name: "Access token required" }),
      ).not.toBeInTheDocument();
    } finally {
      cleanup();
      Object.defineProperty(window, "location", {
        configurable: true,
        value: location,
      });
      if (descriptor) {
        Object.defineProperty(window, "localStorage", descriptor);
      }
    }
  });
});

describe("startup states after the gate has stepped aside (#339)", () => {
  it("shows a model re-download in the Scope zone slot", async () => {
    mockServer();
    render(
      <MemoryRouter initialEntries={["/"]}>
        <App
          startupStatus={{ state: "downloading", percent: 40 }}
          onRetryModelSetup={() => {}}
        />
      </MemoryRouter>,
    );

    expect(
      await screen.findByRole("status", {
        name: "Downloading search model 40%",
      }),
    ).toBeInTheDocument();
  });

  it("says search is downloading, not 'No matching notes', after Retry setup", async () => {
    const server = mockServer({
      startup: { state: "failed", message: "model download failed" },
    });
    // Latched: this browser has already reached the workspace once.
    window.localStorage.setItem("hatchdoor:startup-gate-stepped-past", "1");
    render(
      <MemoryRouter initialEntries={["/"]}>
        <RootApp />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { name: "Notes Explorer" });
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    fireEvent.click(await screen.findByRole("button", { name: "Retry setup" }));

    expect(
      await screen.findByText(/Downloading the search model/),
    ).toBeVisible();
    expect(server.retries).toBe(1);
    expect(screen.queryByText("No matching notes.")).not.toBeInTheDocument();
  });

  it("never mounts the workspace before the first startup answer, so a model gate cannot tear it down", async () => {
    let release!: () => void;
    const server = mockServer({
      startup: { state: "terms_required" },
      startupHold: new Promise<void>((resolve) => {
        release = resolve;
      }),
    });
    render(
      <MemoryRouter initialEntries={["/"]}>
        <RootApp />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(globalThis.fetch).toHaveBeenCalledWith(
        "/api/startup-status",
        expect.anything(),
      );
    });
    expect(screen.queryByText("Notes Explorer")).not.toBeInTheDocument();

    release();
    expect(
      await screen.findByRole("heading", {
        name: "Set up multilingual search",
      }),
    ).toBeVisible();
    expect(server.treeFetches).toBe(0);
  });
});

describe("unknown routes (#339)", () => {
  it("renders a 404 with a way back instead of an empty pane", async () => {
    mockServer();
    render(
      <MemoryRouter initialEntries={["/n/some-old-link"]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    expect(
      await screen.findByRole("heading", { name: "Page Not Found" }),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Go to notes" }));
    expect(
      await screen.findByRole("heading", { name: "Notes Explorer" }),
    ).toBeVisible();
  });
});

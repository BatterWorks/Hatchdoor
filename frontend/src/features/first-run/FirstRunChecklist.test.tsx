import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import { healthyVault } from "../../test/fixtures/vaults";
import type { VaultSummary } from "../../types";
import { FirstRunChecklist } from "./FirstRunChecklist";
import {
  DISMISSED_KEY,
  recordSearchResults,
  resetFirstRunForTests,
} from "./firstRun";

vi.mock("../../api/api", () => ({ apiFetch: vi.fn() }));
const mockedApiFetch = vi.mocked(apiFetch);

type Call = { path: string; method: string; body: unknown };
let calls: Call[] = [];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function settings({
  mcp = false,
  writes = false,
  token = false,
  update = false,
  lastAgent = null as null | { name: string; connected_at: string },
} = {}) {
  return {
    settings: [
      { key: "HATCHDOOR_MCP_ENABLED", value: String(mcp), locked: null },
      {
        key: "HATCHDOOR_MCP_WRITE_ENABLED",
        value: String(writes),
        locked: null,
      },
      {
        key: "HATCHDOOR_MCP_BEARER_TOKEN",
        value: null,
        configured: token,
        locked: null,
      },
      { key: "HATCHDOOR_PUBLIC_URL", value: "", locked: null },
      {
        key: "HATCHDOOR_UPDATE_CHECK_ENABLED",
        value: String(update),
        locked: null,
      },
    ],
    last_agent: lastAgent,
  };
}

/** Routes `METHOD path` to a body; a function sees the request body. */
function serve(routes: Record<string, unknown | ((body: unknown) => unknown)>) {
  mockedApiFetch.mockImplementation(async (input, init) => {
    const path = String(input);
    const method = init?.method ?? "GET";
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ path, method, body });
    const route = routes[`${method} ${path}`];
    if (route === undefined) return json({}, 404);
    return json(typeof route === "function" ? route(body) : route);
  });
}

function renderChecklist(vaults: VaultSummary[] = []) {
  const props = {
    onVaultCreated: vi.fn(),
    onAddGitVault: vi.fn(),
    onOpenSearch: vi.fn(),
  };
  render(<FirstRunChecklist vaults={vaults} {...props} />);
  return props;
}

function step(name: string): HTMLElement {
  return screen.getByRole("heading", { name }).closest("li")!;
}

/** The checklist's own help link, in the paragraph under its intro text. The
 * steps and the update-check block carry more links with the same name. */
function introHelpLink(): HTMLElement {
  const intro = screen.getByText(/^Four steps from nothing/);
  return within(intro.nextElementSibling as HTMLElement).getByRole("button", {
    name: "How does this work? Adding your notes",
  });
}

beforeEach(() => {
  calls = [];
  window.localStorage.clear();
  resetFirstRunForTests();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  mockedApiFetch.mockReset();
});

describe("FirstRunChecklist", () => {
  it("opens on adding notes, with the Git option and the public demo", async () => {
    serve({ "GET /api/settings": settings() });
    const props = renderChecklist();

    expect(
      screen.getByRole("heading", { name: "Set up Hatchdoor" }),
    ).toBeVisible();
    expect(screen.getByText("1 of 4 done")).toBeVisible();
    expect(step("Add your notes")).toHaveAttribute("data-state", "open");
    expect(step("Choose a search model")).toHaveAttribute("data-state", "done");
    expect(
      screen.getByRole("link", { name: "Open the public demo" }),
    ).toHaveAttribute("href", "https://hatchdoor.battercloud.cc");
    fireEvent.click(
      screen.getByRole("button", { name: "Use a Git repository instead" }),
    );
    expect(props.onAddGitVault).toHaveBeenCalled();
    // The update-check block arrives with the settings response and brings a
    // second help link, so wait for it before picking out the intro's.
    await screen.findByText("Optional: tell me when there is a new version");
    expect(introHelpLink()).toBeVisible();
  });

  it("adds a picked folder as a Vault, named after the folder", async () => {
    serve({
      "GET /api/settings": settings(),
      "GET /api/v1/folders": {
        root: "/data/vault",
        root_found: true,
        path: "",
        markdown: { count: 412, at_least: false },
        vault: null,
        folders: [
          {
            name: "Notes",
            path: "Notes",
            markdown: { count: 412, at_least: false },
            vault: null,
            has_subfolders: false,
          },
        ],
        skipped_invalid_names: 0,
      },
      "GET /api/v1/vaults": {
        registry_revision: 7,
        collection_revision: 0,
        vaults: [],
      },
      "POST /api/v1/vaults": (body: unknown) => ({
        vault: { ...healthyVault("Notes"), ...(body as object) },
      }),
    });
    const props = renderChecklist();

    fireEvent.click(screen.getByRole("button", { name: "Pick a folder" }));
    fireEvent.click(await screen.findByRole("button", { name: /^Notes/ }));
    expect(screen.getByRole("textbox", { name: "Name" })).toHaveValue("Notes");
    fireEvent.click(screen.getByRole("button", { name: "Add these notes" }));

    await waitFor(() => expect(props.onVaultCreated).toHaveBeenCalled());
    const create = calls.find(
      (call) => call.method === "POST" && call.path === "/api/v1/vaults",
    );
    expect(create?.body).toEqual({
      expected_registry_revision: 7,
      name: "Notes",
      source: { type: "local", path: "/data/vault/Notes" },
    });
  });

  it("ticks adding notes once a Vault exists", () => {
    serve({ "GET /api/settings": settings() });
    renderChecklist([healthyVault("Recipes")]);
    expect(step("Add your notes")).toHaveAttribute("data-state", "done");
    expect(within(step("Add your notes")).getByText("Recipes")).toBeVisible();
    expect(step("Connect your agent")).toHaveAttribute("data-state", "open");
  });

  it("connects an agent read-only and shows a filled-in config per client", async () => {
    serve({
      "GET /api/settings": settings(),
      "POST /api/settings/mcp-token/generate": { value: "new-secret" },
      "PATCH /api/settings": settings({ mcp: true, token: true }),
    });
    renderChecklist([healthyVault("Recipes")]);

    fireEvent.click(
      await screen.findByRole("button", { name: "Connect an agent" }),
    );
    await screen.findByText("Copy this now.");

    const patch = calls.find((call) => call.method === "PATCH");
    expect(patch?.body).toEqual({
      updates: {
        HATCHDOOR_MCP_ENABLED: "true",
        HATCHDOOR_MCP_WRITE_ENABLED: "false",
        HATCHDOOR_MCP_BEARER_TOKEN: "new-secret",
      },
      confirm: [],
    });
    const code = () => document.querySelector(".first-run-code")!.textContent!;
    expect(code()).toContain("claude mcp add");
    expect(code()).toContain(`${window.location.origin}/mcp`);
    expect(code()).toContain("Bearer new-secret");
    for (const [label, marker] of [
      ["Codex", "[mcp_servers.hatchdoor]"],
      ["OpenClaw", '"streamable-http"'],
      ["Hermes", "mcp_servers:"],
      ["Other", "Address"],
    ]) {
      fireEvent.click(screen.getByRole("button", { name: label }));
      expect(code()).toContain(marker);
      expect(code()).toContain("Bearer new-secret");
    }
    expect(screen.getByText(/Waiting for your agent to connect/)).toBeVisible();
    expect(screen.getByText(/Your agent starts read-only/)).toBeVisible();
  });

  it("ticks itself when the agent connects, naming it and when", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let connected = false;
    serve({
      "GET /api/settings": () =>
        settings({
          mcp: true,
          token: true,
          lastAgent: connected
            ? {
                name: "Claude Code",
                connected_at: new Date(Date.now() - 120_000).toISOString(),
              }
            : null,
        }),
    });
    renderChecklist([healthyVault("Recipes")]);
    await screen.findByText(/Waiting for your agent to connect/);

    connected = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    await waitFor(() =>
      expect(step("Connect your agent")).toHaveAttribute("data-state", "done"),
    );
    expect(
      within(step("Connect your agent")).getByText("Claude Code"),
    ).toBeVisible();
    expect(
      within(step("Connect your agent")).getByText(/connected 2 minutes ago/),
    ).toBeVisible();
  });

  it("offers a new password when the old one can no longer be shown", async () => {
    serve({
      "GET /api/settings": settings({ mcp: true, token: true }),
      "POST /api/settings/mcp-token/generate": { value: "second" },
      "PATCH /api/settings": settings({ mcp: true, token: true }),
    });
    renderChecklist([healthyVault("Recipes")]);
    expect(
      await screen.findByText(/The password was shown once/),
    ).toBeVisible();
    expect(document.querySelector(".first-run-code")!.textContent).toContain(
      "<your MCP password>",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Make a new password" }),
    );
    await screen.findByText("Copy this now.");
    const patch = calls.find((call) => call.method === "PATCH");
    expect(patch?.body).toEqual({
      updates: { HATCHDOOR_MCP_BEARER_TOKEN: "second" },
      confirm: [],
    });
  });

  it("ticks the search step from a search that found something in this browser", async () => {
    serve({ "GET /api/settings": settings() });
    const props = renderChecklist([healthyVault("Recipes")]);
    fireEvent.click(screen.getByRole("button", { name: "Try a search" }));
    fireEvent.click(screen.getByRole("button", { name: "Open search" }));
    expect(props.onOpenSearch).toHaveBeenCalled();

    act(() => recordSearchResults("recipes", 12));
    expect(step("Try a search")).toHaveAttribute("data-state", "done");
    expect(
      within(step("Try a search")).getByText(/found 12 notes/),
    ).toBeVisible();
  });

  it("says so when everything is done, and closes for good", async () => {
    recordSearchResults("recipes", 3);
    serve({
      "GET /api/settings": settings({
        mcp: true,
        token: true,
        lastAgent: { name: "Codex", connected_at: new Date().toISOString() },
      }),
    });
    renderChecklist([healthyVault("Recipes")]);
    expect(
      await screen.findByRole("heading", { name: "You're set up" }),
    ).toBeVisible();
    expect(screen.getByText("4 of 4 done")).toBeVisible();
    fireEvent.click(
      screen.getByRole("button", { name: "Close the checklist" }),
    );
    expect(window.localStorage.getItem(DISMISSED_KEY)).toBe("1");
  });

  it("names each of its four help links after what it explains (#460)", async () => {
    serve({ "GET /api/settings": settings() });
    renderChecklist([healthyVault("Recipes")]);
    await screen.findByRole("button", { name: "Connect an agent" });

    const links = screen.getAllByRole("button", {
      name: /^How does this work\?/,
    });
    expect(links.map((link) => link.getAttribute("aria-label"))).toEqual([
      "How does this work? Adding your notes",
      "How does this work? Connecting your agent",
      "How does this work? Letting your agent change notes",
      "How does this work? Hearing about new releases",
    ]);
    for (const link of links) {
      expect(link).toHaveTextContent(/^How does this work\?$/);
    }
  });

  it("offers the update check off, and turns it on when asked", async () => {
    serve({
      "GET /api/settings": settings(),
      "PATCH /api/settings": settings({ update: true }),
    });
    renderChecklist();
    const toggle = await screen.findByRole("button", {
      name: "Tell me when there is a new version",
    });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "true"));
    expect(calls.find((call) => call.method === "PATCH")?.body).toEqual({
      updates: { HATCHDOOR_UPDATE_CHECK_ENABLED: "true" },
      confirm: [],
    });
  });
});

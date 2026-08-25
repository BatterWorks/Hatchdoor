import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import { SettingsPage } from "./SettingsPage";

function renderSettingsPage() {
  return render(
    <MemoryRouter initialEntries={["/settings"]}>
      <SettingsPage />
    </MemoryRouter>,
  );
}

vi.mock("../../api/api", () => ({ apiFetch: vi.fn() }));
const mockedApiFetch = vi.mocked(apiFetch);

const settings = [
  {
    key: "HATCHDOOR_EMBED_LAYERS",
    value: "true",
    source: "default",
    locked: null,
    class: "reindex",
    kind: "switch",
  },
  {
    key: "HATCHDOOR_MCP_ENABLED",
    value: "false",
    source: "default",
    locked: null,
    class: "instant",
    kind: "switch",
  },
  {
    key: "HATCHDOOR_MCP_WRITE_ENABLED",
    value: "false",
    source: "default",
    locked: null,
    class: "instant",
    kind: "switch",
  },
  {
    key: "HATCHDOOR_MCP_RATE_LIMITS_ENABLED",
    value: "true",
    source: "default",
    locked: null,
    class: "instant",
    kind: "switch",
  },
  {
    key: "HATCHDOOR_MCP_BEARER_TOKEN",
    value: null,
    configured: false,
    source: "default",
    locked: null,
    class: "instant",
    kind: "secret",
  },
  {
    key: "HATCHDOOR_MCP_ALLOWED_ORIGINS",
    value: "http://localhost",
    source: "default",
    locked: null,
    class: "instant",
    kind: "text",
  },
  {
    key: "HATCHDOOR_MAX_ATTACHMENT_BYTES",
    value: "10485760",
    source: "default",
    locked: null,
    class: "instant",
    kind: "number",
  },
  {
    key: "HATCHDOOR_MCP_MAX_BASE64_BYTES",
    value: "10485760",
    source: "default",
    locked: null,
    class: "instant",
    kind: "number",
  },
  {
    key: "HATCHDOOR_GIT_AUTHOR_NAME",
    value: "Server author",
    source: "default",
    locked: null,
    class: "instant",
    kind: "text",
  },
  {
    key: "HATCHDOOR_GIT_AUTHOR_EMAIL",
    value: "author@example.test",
    source: "default",
    locked: null,
    class: "instant",
    kind: "text",
  },
] as const;

const json = (body: unknown) =>
  new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
  });

function vault(name: string, enabled = true) {
  return {
    vault_id: enabled
      ? "00000000-0000-4000-8000-000000000001"
      : "00000000-0000-4000-8000-000000000002",
    name,
    enabled,
    source: { type: "local", path: "/notes" },
    exclude_patterns: ["drafts/"],
    credential_configured: false,
    archive_folder: "Archive/",
    activation: enabled ? "active" : "disabled",
    local_content: "read_write",
    search: "ready",
    git: "disabled",
    watcher: enabled ? "running" : "disabled",
    capabilities: {
      browse: true,
      search: true,
      mutate: true,
      pull: false,
      push: false,
      retry: false,
    },
  };
}

function mockPage(vaults = [vault("Field notes")]) {
  mockedApiFetch.mockImplementation(async (input) => {
    const url = String(input);
    if (url === "/api/settings") return json({ settings });
    if (url === "/api/git-status")
      return json({ state: "disabled", mode: "off" });
    if (url === "/api/index-status")
      return json({ state: "up_to_date", stale: false });
    if (url === "/api/v1/vaults")
      return json({
        registry_revision: 3,
        collection_revision: 3,
        vaults,
        demo_mode: false,
      });
    if (url === "/api/v1/vaults/all/stats")
      return json({
        data: vaults
          .filter((item) => item.enabled)
          .map((item) => ({ vault_id: item.vault_id, note_count: 12 })),
      });
    if (url.includes("/recent?limit=1"))
      return json({ data: [{ mtime_ns: 0 }] });
    throw new Error(`Unexpected API request: ${url}`);
  });
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.clearAllMocks();
  window.localStorage.clear();
});

describe("SettingsPage", () => {
  it("opens a Vault's own settings page from the management index", async () => {
    mockPage();
    renderSettingsPage();
    fireEvent.click(await screen.findByRole("button", { name: /Field notes/ }));

    expect(
      await screen.findByRole("heading", { name: "Field notes" }),
    ).toBeVisible();
    expect(screen.getByText("This Vault is ready to use.")).toBeVisible();
    expect(
      screen.getByText(
        "A folder on this server · 12 notes · no indexed changes yet",
      ),
    ).toBeVisible();
    expect(screen.getByDisplayValue("Archive/")).toBeVisible();
    expect(screen.getByPlaceholderText("Server author")).toBeVisible();
    expect(screen.getByText("Where this Vault came from")).toBeVisible();
    expect(screen.getByRole("button", { name: "Pause Vault" })).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Rebuild search index" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Disconnect Vault" }),
    ).toBeVisible();
  });

  it("returns to a This server section from an open Vault page", async () => {
    mockPage();
    renderSettingsPage();
    fireEvent.click(await screen.findByRole("button", { name: /Field notes/ }));
    expect(
      await screen.findByRole("heading", { name: "Field notes" }),
    ).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: /Notes handling/ }));

    expect(
      screen.queryByRole("heading", { name: "Field notes" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: /Notes handling/ }),
    ).toBeVisible();
  });

  it("keeps a paused Vault in Settings and nowhere in the server section", async () => {
    mockPage([vault("Field notes"), vault("Archive", false)]);
    renderSettingsPage();

    const paused = await screen.findByRole("button", { name: /Archive/ });
    expect(paused).toHaveAttribute("data-paused", "true");
    expect(screen.getByText("paused")).toBeVisible();
    expect(screen.getByText("This server")).toBeVisible();
    expect(
      screen.getByRole("button", { name: /Notes handling/ }),
    ).toHaveTextContent("01");
    expect(
      screen.queryByRole("button", { name: "Versioning" }),
    ).not.toBeInTheDocument();
  });

  it("surfaces a held draft under This server and withdraws once it is discarded", async () => {
    window.localStorage.setItem(
      "hatchdoor:heldDraft:note:orphaned",
      JSON.stringify({
        id: "note:orphaned",
        kind: "note",
        slug: "orphaned",
        content: "unsaved",
        baseContentHash: "abc",
        savedAt: Date.now(),
      }),
    );
    mockPage();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderSettingsPage();

    const draftsNav = await screen.findByRole("button", {
      name: /Unsaved drafts/,
    });
    expect(draftsNav).toHaveTextContent("1");

    fireEvent.click(draftsNav);
    expect(
      await screen.findByRole("heading", { name: "Unsaved drafts" }),
    ).toBeVisible();
    expect(screen.getByText("orphaned")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Discard" }));

    // The section is a migration artefact: gone for good once dealt with.
    expect(
      screen.queryByRole("button", { name: /Unsaved drafts/ }),
    ).not.toBeInTheDocument();
    expect(
      window.localStorage.getItem("hatchdoor:heldDraft:note:orphaned"),
    ).toBeNull();
  });
});

import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
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

vi.mock("../../api/api", () => ({
  apiFetch: vi.fn(),
  // The Vault collection client opens the shared revision stream through this.
  withAccessToken: (url: string) => url,
}));
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
    key: "HATCHDOOR_PUBLIC_URL",
    value: "",
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
  {
    // The retired instance-wide Git switch, at its shipped default. The page
    // hides it and must not let it hide anything else (#340).
    key: "HATCHDOOR_GIT_SYNC_ENABLED",
    value: "false",
    source: "default",
    locked: null,
    class: "instant",
    kind: "mode",
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

function mockPage(
  vaults = [vault("Field notes")],
  onPatch?: (updates: Record<string, string>) => void,
) {
  mockedApiFetch.mockImplementation(async (input, init) => {
    const url = String(input);
    if (url === "/api/settings" && init?.method === "PATCH") {
      const { updates } = JSON.parse(String(init.body)) as {
        updates: Record<string, string>;
      };
      onPatch?.(updates);
      return json({
        settings: settings.map((item) =>
          item.key in updates ? { ...item, value: updates[item.key] } : item,
        ),
      });
    }
    if (url === "/api/settings") return json({ settings });
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
    // `/api/index-status` and `/api/git-status` were retired with their
    // consoles (#183): a request to either is now an unexpected request and
    // fails this stub, which is how the page is held to never polling again.
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

  it("renders no instance-wide Search index or Versioning console", async () => {
    mockPage();
    renderSettingsPage();

    // Every question the two consoles answered is answered per Vault now
    // (#183), so the page no longer shows them and no longer polls the three
    // routes that fed them — the stub throws on an unexpected request.
    expect(
      await screen.findByRole("heading", { name: /Notes handling/ }),
    ).toBeVisible();
    expect(screen.queryByText("Search index")).not.toBeInTheDocument();
    expect(screen.queryByText("Up to date")).not.toBeInTheDocument();
    expect(screen.queryByText("Behind your settings")).not.toBeInTheDocument();
    const requested = mockedApiFetch.mock.calls.map((call) => String(call[0]));
    expect(requested).not.toContain("/api/index-status");
    expect(requested).not.toContain("/api/git-status");
  });

  it("offers the server commit identity with Git sync at its default (#340)", async () => {
    mockPage();
    renderSettingsPage();

    // Every Vault without its own commit identity falls back to these two, so
    // they stay editable however the retired instance-wide Git switch reads.
    expect(await screen.findByLabelText("Recorded as (name)")).toHaveValue(
      "Server author",
    );
    expect(screen.getByLabelText("Recorded as (email)")).toHaveValue(
      "author@example.test",
    );
    // The footer counts the rows the page renders: all eleven, none hidden.
    expect(screen.getByText(/11 editable here, 0 set in/)).toBeVisible();
  });

  it("offers the public address under Agent access", async () => {
    mockPage();
    renderSettingsPage();
    fireEvent.click(
      await screen.findByRole("button", { name: /Agent access/ }),
    );

    expect(await screen.findByText("Public address")).toBeVisible();
    expect(
      screen.getByPlaceholderText("https://notes.example.com"),
    ).toBeVisible();
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

  describe("keeps each section's unsaved edits to itself (#338)", () => {
    async function editAcrossTwoSections() {
      fireEvent.click(
        await screen.findByRole("button", { name: /Agent access/ }),
      );
      fireEvent.change(await screen.findByLabelText("Public address"), {
        target: { value: "https://notes.example.test" },
      });
      fireEvent.click(screen.getByRole("button", { name: /Uploads/ }));
      fireEvent.change(
        await screen.findByLabelText("Largest file from this app"),
        { target: { value: "20" } },
      );
    }

    it("saving one section leaves another section's edit in place", async () => {
      const sent: Record<string, string>[] = [];
      mockPage(undefined, (updates) => sent.push(updates));
      renderSettingsPage();
      await editAcrossTwoSections();

      fireEvent.click(screen.getByRole("button", { name: "Save uploads" }));
      await screen.findByText("Saved");
      expect(sent).toHaveLength(1);
      expect(Object.keys(sent[0])).toEqual(["HATCHDOOR_MAX_ATTACHMENT_BYTES"]);

      fireEvent.click(screen.getByRole("button", { name: /Agent access/ }));
      expect(await screen.findByLabelText("Public address")).toHaveValue(
        "https://notes.example.test",
      );
    });

    it("discarding one section leaves another section's edit in place", async () => {
      mockPage();
      renderSettingsPage();
      await editAcrossTwoSections();

      fireEvent.click(screen.getByRole("button", { name: "Discard" }));
      expect(screen.getByLabelText("Largest file from this app")).toHaveValue(
        10,
      );

      fireEvent.click(screen.getByRole("button", { name: /Agent access/ }));
      expect(await screen.findByLabelText("Public address")).toHaveValue(
        "https://notes.example.test",
      );
    });
  });

  it("focuses the reindex confirmation and closes it on Escape", async () => {
    mockPage();
    renderSettingsPage();
    fireEvent.click(
      await screen.findByLabelText("Meaning search in demoted layers"),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Save notes handling" }),
    );

    const dialog = await screen.findByRole("dialog", {
      name: "Before this is saved",
    });
    expect(
      within(dialog).getByRole("button", { name: "Cancel" }),
    ).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(
      screen.queryByRole("dialog", { name: "Before this is saved" }),
    ).not.toBeInTheDocument();
  });
});

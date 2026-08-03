import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import { SettingsPage } from "./SettingsPage";

vi.mock("../../api/api", () => ({ apiFetch: vi.fn() }));
const mockedApiFetch = vi.mocked(apiFetch);

const settings = [
  {
    key: "HATCHDOOR_ARCHIVE_PREFIX",
    value: "90-archive/",
    source: "default",
    locked: null,
    class: "instant",
    kind: "text",
  },
  {
    key: "HATCHDOOR_EXCLUDE",
    value: ".git/**",
    source: "default",
    locked: null,
    class: "reindex",
    kind: "text",
  },
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
    value: "http://127.0.0.1,http://localhost",
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
    key: "HATCHDOOR_GIT_BRANCH",
    value: "main",
    source: "environment",
    locked: "never",
    class: "instant",
    kind: "text",
  },
] as const;

const json = (body: unknown) =>
  new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
  });

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

describe("SettingsPage", () => {
  it("renders server metadata and saves only changes from the active section", async () => {
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));
    mockedApiFetch.mockResolvedValueOnce(
      json({ state: "up_to_date", stale: false, drift: false }),
    );
    mockedApiFetch.mockResolvedValueOnce(
      json({
        settings: [{ ...settings[0], value: "archive/" }, ...settings.slice(1)],
      }),
    );
    render(<SettingsPage />);
    const input = await screen.findByDisplayValue("90-archive/");
    fireEvent.change(input, { target: { value: "archive/" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));
    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(3));
    expect(mockedApiFetch.mock.calls[2]?.[0]).toBe("/api/settings");
    expect(JSON.parse(String(mockedApiFetch.mock.calls[2]?.[1]?.body))).toEqual(
      { updates: { HATCHDOOR_ARCHIVE_PREFIX: "archive/" } },
    );
    fireEvent.click(screen.getByRole("button", { name: /Versioning/ }));
    expect(await screen.findByText("Managed outside this page")).toBeVisible();
    expect(screen.getByText("HATCHDOOR_GIT_BRANCH")).toBeVisible();
  });

  it("confirms an index-affecting save and polls the dedicated stale-index status", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));
    mockedApiFetch.mockResolvedValueOnce(
      json({
        state: "rebuilding",
        stale: true,
        drift: true,
        notes_completed: 12,
        notes_total: 40,
        chunks_completed: 18,
        chunks_total: 70,
        tokens_completed: 4_000,
        tokens_total: 20_000,
        percent: 20,
        eta_seconds: 80,
        last_failure: null,
      }),
    );
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));

    render(<SettingsPage />);
    const exclude = await screen.findByDisplayValue(".git/**");
    fireEvent.change(exclude, { target: { value: ".git/**, build/**" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));

    await waitFor(() => expect(window.confirm).toHaveBeenCalledTimes(1));
    expect(window.confirm).toHaveBeenCalledWith(
      expect.stringContaining("rebuild the search index"),
    );
    expect(JSON.parse(String(mockedApiFetch.mock.calls[2]?.[1]?.body))).toEqual(
      {
        updates: { HATCHDOOR_EXCLUDE: ".git/**, build/**" },
        confirm_reindex: true,
      },
    );
    expect(
      await screen.findByText("Rebuilding in the background"),
    ).toBeVisible();
    expect(
      screen.getByText(
        "Search remains available from the previous coherent index while this rebuild runs.",
      ),
    ).toBeVisible();
    expect(
      screen.getByText("12 / 40 notes · 20% · about 1m 20s left"),
    ).toBeVisible();
  });

  it("generates an MCP token candidate without saving it, then includes it in one enable transaction", async () => {
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));
    mockedApiFetch.mockResolvedValueOnce(
      json({ state: "up_to_date", stale: false, drift: false }),
    );
    mockedApiFetch.mockResolvedValueOnce(json({ value: "candidate-token" }));
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));

    render(<SettingsPage />);
    await screen.findByDisplayValue("90-archive/");
    fireEvent.click(screen.getByRole("button", { name: /Agent access/ }));
    fireEvent.click(
      screen.getByRole("button", { name: "Generate MCP password" }),
    );

    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(3));
    expect(mockedApiFetch.mock.calls[2]?.[0]).toBe(
      "/api/settings/mcp-token/generate",
    );
    expect(mockedApiFetch.mock.calls[2]?.[1]?.method).toBe("POST");
    expect(await screen.findByDisplayValue("candidate-token")).toBeVisible();

    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Save Agent access (MCP)" }));
    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(4));
    expect(JSON.parse(String(mockedApiFetch.mock.calls[3]?.[1]?.body))).toEqual({
      updates: {
        HATCHDOOR_MCP_ENABLED: "true",
        HATCHDOOR_MCP_BEARER_TOKEN: "candidate-token",
      },
    });
  });
});

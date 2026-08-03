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
    key: "HATCHDOOR_GIT_SYNC_ENABLED",
    value: "off",
    source: "default",
    locked: null,
    class: "instant",
    kind: "mode",
  },
  {
    key: "HATCHDOOR_GIT_HTTPS_USERNAME",
    value: "hatchdoor",
    source: "default",
    locked: null,
    class: "instant",
    kind: "text",
  },
  {
    key: "HATCHDOOR_GIT_HTTPS_TOKEN",
    value: null,
    configured: false,
    source: "default",
    locked: null,
    class: "instant",
    kind: "secret",
  },
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

const idleStatuses = () => {
  mockedApiFetch.mockResolvedValueOnce(
    json({ state: "disabled", mode: "off" }),
  );
  mockedApiFetch.mockResolvedValueOnce(
    json({ state: "up_to_date", stale: false, drift: false }),
  );
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

describe("SettingsPage", () => {
  it("saves only the active section, and records locked settings on a plaque", async () => {
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));
    idleStatuses();
    mockedApiFetch.mockResolvedValueOnce(
      json({
        settings: [{ ...settings[0], value: "archive/" }, ...settings.slice(1)],
      }),
    );
    render(<SettingsPage />);
    const input = await screen.findByDisplayValue("90-archive/");
    fireEvent.change(input, { target: { value: "archive/" } });
    fireEvent.click(screen.getByRole("button", { name: "Save vault" }));
    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(4));
    expect(mockedApiFetch.mock.calls[3]?.[0]).toBe("/api/settings");
    expect(JSON.parse(String(mockedApiFetch.mock.calls[3]?.[1]?.body))).toEqual(
      {
        updates: { HATCHDOOR_ARCHIVE_PREFIX: "archive/" },
      },
    );

    fireEvent.click(screen.getByRole("button", { name: /Versioning/ }));
    fireEvent.click(screen.getByRole("button", { name: "This machine" }));
    expect(await screen.findByText("Managed outside this page")).toBeVisible();
    expect(screen.getByText("HATCHDOOR_GIT_BRANCH")).toBeVisible();
    expect(
      screen.getByText(
        "Hatchdoor always follows whichever branch your vault folder is on, so there is nothing to choose.",
      ),
    ).toBeVisible();
  });

  it("confirms an index-affecting save in a modal and shows the rebuild strip", async () => {
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));
    mockedApiFetch.mockResolvedValueOnce(
      json({ state: "disabled", mode: "off" }),
    );
    mockedApiFetch.mockResolvedValueOnce(
      json({
        state: "rebuilding",
        stale: true,
        drift: true,
        notes_completed: 12,
        notes_total: 40,
        percent: 20,
        eta_seconds: 80,
        last_failure: null,
      }),
    );
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));

    render(<SettingsPage />);
    const exclude = await screen.findByDisplayValue(".git/**");
    fireEvent.change(exclude, { target: { value: ".git/**, build/**" } });
    fireEvent.click(screen.getByRole("button", { name: "Save vault" }));

    expect(await screen.findByText("Before this is saved")).toBeVisible();
    expect(
      screen.getByText(/keeps answering from the old setting/),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Go ahead" }));

    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(4));
    expect(JSON.parse(String(mockedApiFetch.mock.calls[3]?.[1]?.body))).toEqual(
      {
        updates: { HATCHDOOR_EXCLUDE: ".git/**, build/**" },
        confirm_reindex: true,
      },
    );
    expect(await screen.findByText("Rebuilding 20%")).toBeVisible();
    expect(
      screen.getByText(/Still answering from the old setting/),
    ).toBeVisible();
    expect(screen.getByText(/about 80 seconds left/)).toBeVisible();
  });

  it("generates an MCP token candidate without saving it, then includes it in one enable transaction", async () => {
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));
    idleStatuses();
    mockedApiFetch.mockResolvedValueOnce(json({ value: "candidate-token" }));
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));

    render(<SettingsPage />);
    await screen.findByDisplayValue("90-archive/");
    fireEvent.click(screen.getByRole("button", { name: /Agent access/ }));
    fireEvent.click(screen.getByRole("button", { name: "Generate" }));

    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(4));
    expect(mockedApiFetch.mock.calls[3]?.[0]).toBe(
      "/api/settings/mcp-token/generate",
    );
    expect(mockedApiFetch.mock.calls[3]?.[1]?.method).toBe("POST");
    expect(await screen.findByDisplayValue("candidate-token")).toBeVisible();

    fireEvent.click(
      screen.getByRole("button", { name: "Let assistants connect (MCP)" }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Save agent access (mcp)" }),
    );
    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(5));
    expect(JSON.parse(String(mockedApiFetch.mock.calls[4]?.[1]?.body))).toEqual(
      {
        updates: {
          HATCHDOOR_MCP_ENABLED: "true",
          HATCHDOOR_MCP_BEARER_TOKEN: "candidate-token",
        },
      },
    );
  });

  it("hides remote credentials when local history is selected", async () => {
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));
    idleStatuses();
    render(<SettingsPage />);
    await screen.findByDisplayValue("90-archive/");
    fireEvent.click(screen.getByRole("button", { name: /Versioning/ }));
    fireEvent.click(screen.getByRole("button", { name: "This machine" }));
    expect(screen.queryByText("Username")).not.toBeInTheDocument();
    expect(screen.getByText("Branch")).toBeVisible();
  });

  it("confirms a remote downgrade before saving it", async () => {
    const remoteSettings = settings.map((setting) =>
      setting.key === "HATCHDOOR_GIT_SYNC_ENABLED"
        ? { ...setting, value: "remote" }
        : setting,
    );
    mockedApiFetch.mockResolvedValueOnce(json({ settings: remoteSettings }));
    mockedApiFetch.mockResolvedValueOnce(
      json({ state: "running", mode: "remote" }),
    );
    mockedApiFetch.mockResolvedValueOnce(
      json({ state: "up_to_date", stale: false, drift: false }),
    );
    mockedApiFetch.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          error:
            "Switching away from remote versioning stops sending future commits to the remote.",
          confirmation_required: "git_downgrade",
        }),
        { status: 409, headers: { "content-type": "application/json" } },
      ),
    );
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));
    render(<SettingsPage />);
    await screen.findByDisplayValue("90-archive/");
    fireEvent.click(screen.getByRole("button", { name: /Versioning/ }));
    fireEvent.click(screen.getByRole("button", { name: "Off" }));
    fireEvent.click(screen.getByRole("button", { name: "Save versioning" }));
    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(4));
    expect(await screen.findByText("Before this is saved")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Go ahead" }));
    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(5));
    expect(JSON.parse(String(mockedApiFetch.mock.calls[4]?.[1]?.body))).toEqual(
      {
        updates: { HATCHDOOR_GIT_SYNC_ENABLED: "off" },
        confirm_git_downgrade: true,
      },
    );
  });

  it("shows a busy refusal as a warning rather than a failure", async () => {
    const remoteSettings = settings.map((setting) =>
      setting.key === "HATCHDOOR_GIT_SYNC_ENABLED"
        ? { ...setting, value: "remote" }
        : setting,
    );
    mockedApiFetch.mockResolvedValueOnce(json({ settings: remoteSettings }));
    mockedApiFetch.mockResolvedValueOnce(
      json({ state: "stopping", mode: "remote" }),
    );
    mockedApiFetch.mockResolvedValueOnce(
      json({ state: "up_to_date", stale: false, drift: false }),
    );
    mockedApiFetch.mockResolvedValueOnce(
      new Response(
        JSON.stringify({ error: "sync task did not stop", state: "stopping" }),
        { status: 409, headers: { "content-type": "application/json" } },
      ),
    );
    render(<SettingsPage />);
    await screen.findByDisplayValue("90-archive/");
    fireEvent.click(screen.getByRole("button", { name: /Versioning/ }));
    fireEvent.click(screen.getByRole("button", { name: "Off" }));
    fireEvent.click(screen.getByRole("button", { name: "Save versioning" }));
    expect(
      await screen.findByText(
        "Still finishing the last batch of changes. Try again in a few seconds — nothing was lost.",
      ),
    ).toBeVisible();
  });
});

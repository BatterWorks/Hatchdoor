import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

afterEach(() => vi.clearAllMocks());

describe("SettingsPage", () => {
  it("renders server metadata and saves only changes from the active section", async () => {
    mockedApiFetch.mockResolvedValueOnce(json({ settings }));
    mockedApiFetch.mockResolvedValueOnce(
      json({
        settings: [{ ...settings[0], value: "archive/" }, ...settings.slice(1)],
      }),
    );
    render(<SettingsPage />);
    const input = await screen.findByDisplayValue("90-archive/");
    fireEvent.change(input, { target: { value: "archive/" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));
    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(2));
    expect(mockedApiFetch.mock.calls[1]?.[0]).toBe("/api/settings");
    expect(JSON.parse(String(mockedApiFetch.mock.calls[1]?.[1]?.body))).toEqual(
      { updates: { HATCHDOOR_ARCHIVE_PREFIX: "archive/" } },
    );
    fireEvent.click(screen.getByRole("button", { name: /Versioning/ }));
    expect(await screen.findByText("Managed outside this page")).toBeVisible();
    expect(screen.getByText("HATCHDOOR_GIT_BRANCH")).toBeVisible();
  });
});

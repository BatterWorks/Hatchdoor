import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  acceptGemma: vi.fn(),
  useVaultDiscovery: vi.fn(),
}));

vi.mock("./hooks/useVaultScope", () => ({
  useVaultDiscovery: mocks.useVaultDiscovery,
}));

vi.mock("./startup/useStartupStatus", () => ({
  useStartupStatus: () => ({
    status: { state: "terms_required" },
    connectionIssue: false,
    hasSteppedPastGate: false,
    acceptGemma: mocks.acceptGemma,
    declineGemma: vi.fn(),
    retryModelSetup: vi.fn(),
  }),
}));

import { App } from "./App";
import { clearToken, notifyUnauthorized } from "./api/api";

afterEach(() => {
  cleanup();
  clearToken();
  vi.restoreAllMocks();
  mocks.acceptGemma.mockReset();
  mocks.useVaultDiscovery.mockReset();
});

it("prompts for the web token when first-run model setup is unauthorized", async () => {
  mocks.useVaultDiscovery.mockReturnValue({
    vaults: [{ enabled: true }],
    demoMode: false,
    loading: false,
    error: null,
    recovery: null,
    legacyMigrationRecovery: null,
    loadVaults: vi.fn(),
  });
  mocks.acceptGemma.mockImplementation(() => notifyUnauthorized());

  render(
    <MemoryRouter>
      <App />
    </MemoryRouter>,
  );

  fireEvent.click(
    await screen.findByRole("button", {
      name: "Accept terms and set up Gemma",
    }),
  );

  expect(
    await screen.findByRole("dialog", { name: "Access token required" }),
  ).toBeVisible();
});

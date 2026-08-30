import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  acceptGemma: vi.fn(),
  useVaultCollection: vi.fn(),
}));

vi.mock("./vaults", () => ({
  useVaultCollection: mocks.useVaultCollection,
  useVaultProjection: () => ({
    slotFor: () => ({ kind: "count", count: 0 }),
    describeScope: () => null,
  }),
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
  mocks.useVaultCollection.mockReset();
});

it("prompts for the web token when first-run model setup is unauthorized", async () => {
  mocks.useVaultCollection.mockReturnValue({
    vaults: [{ enabled: true }],
    demoMode: false,
    loading: false,
    error: null,
    recovery: null,
    legacyMigrationRecovery: null,
    allVaults: [{ enabled: true }],
    registryRevision: 0,
    revision: 0,
    noteCounts: {},
    refresh: vi.fn(),
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

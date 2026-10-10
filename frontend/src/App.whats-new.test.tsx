import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
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
    status: { state: "ready" },
    connectionIssue: false,
    hasSteppedPastGate: true,
    acceptGemma: vi.fn(),
    declineGemma: vi.fn(),
    retryModelSetup: vi.fn(),
  }),
}));

import { App } from "./App";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  mocks.useVaultCollection.mockReset();
  localStorage.clear();
});

function collection(demoMode: boolean) {
  return {
    vaults: [],
    demoMode,
    loading: false,
    readState: "empty",
    error: null,
    recovery: null,
    allVaults: [],
    registryRevision: 0,
    revision: null,
    noteCounts: {},
    refresh: vi.fn(),
  };
}

function serveUpgrade() {
  return vi.spyOn(globalThis, "fetch").mockImplementation(async (input) =>
    String(input) === "/api/v1/whats-new"
      ? new Response(
          JSON.stringify({
            version: "2.8.0",
            previous_version: "2.7.0",
            fresh_install: false,
            releases: [
              {
                version: "2.8.0",
                date: "2026-10-20",
                highlights: [
                  { text: "One.", action_needed: false, link: null },
                  { text: "Two.", action_needed: false, link: null },
                  { text: "Three.", action_needed: false, link: null },
                ],
              },
            ],
          }),
        )
      : new Response("not found", { status: 404 }),
  );
}

function whatsNewRequests(fetchMock: ReturnType<typeof serveUpgrade>) {
  return fetchMock.mock.calls.filter(
    ([input]) => String(input) === "/api/v1/whats-new",
  );
}

it("shows What's new over the signed-in workspace after an upgrade (#418)", async () => {
  mocks.useVaultCollection.mockReturnValue(collection(false));
  serveUpgrade();

  render(
    <MemoryRouter>
      <App />
    </MemoryRouter>,
  );

  expect(
    await screen.findByRole("dialog", { name: "Hatchdoor 2.8.0" }),
  ).toBeInTheDocument();
});

it("never asks for What's new in demo mode (#418)", async () => {
  mocks.useVaultCollection.mockReturnValue(collection(true));
  const fetchMock = serveUpgrade();

  render(
    <MemoryRouter>
      <App />
    </MemoryRouter>,
  );

  await screen.findByText("No Vaults Yet");
  await waitFor(() => expect(fetchMock).toHaveBeenCalled());
  expect(whatsNewRequests(fetchMock)).toHaveLength(0);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

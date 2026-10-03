import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
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

const originalScrollIntoView = Element.prototype.scrollIntoView;

afterEach(() => {
  cleanup();
  Element.prototype.scrollIntoView = originalScrollIntoView;
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
    revision: null,
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

it("links the token prompt to the Help page that answers it, signed out (#417)", async () => {
  mocks.useVaultCollection.mockReturnValue({
    vaults: [{ enabled: true }],
    demoMode: false,
    loading: false,
    error: null,
    recovery: null,
    legacyMigrationRecovery: null,
    allVaults: [{ enabled: true }],
    registryRevision: 0,
    revision: null,
    noteCounts: {},
    refresh: vi.fn(),
  });
  mocks.acceptGemma.mockImplementation(() => notifyUnauthorized());
  const scrolled: string[] = [];
  Element.prototype.scrollIntoView = function (this: Element) {
    scrolled.push(this.id);
  };
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(
      async (input) =>
        new Response(
          String(input) ===
            "/docs/get-started/install-hatchdoor-with-docker-compose.md"
            ? "# Install Hatchdoor\n\n## Where do I find my token?\n\nIn `.env`."
            : "# Hatchdoor documentation\n\nThe manual.",
        ),
    );

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
  const prompt = await screen.findByRole("dialog", {
    name: "Access token required",
  });

  fireEvent.click(
    within(prompt).getByRole("button", { name: "Where do I find my token?" }),
  );

  const help = screen.getByRole("complementary", { name: "Help" });
  expect(help).toHaveClass("is-above-dialogs");
  const heading = await within(help).findByRole("heading", {
    name: "Where do I find my token?",
  });
  await waitFor(() => expect(scrolled).toContain(heading.id));
  const [url, init] = fetchMock.mock.calls[0];
  expect(url).toBe(
    "/docs/get-started/install-hatchdoor-with-docker-compose.md",
  );
  expect(new Headers(init?.headers).has("Authorization")).toBe(false);

  fireEvent.click(within(prompt).getByRole("button", { name: "Help" }));
  expect(
    await within(help).findByRole("heading", {
      name: "Hatchdoor documentation",
    }),
  ).toBeVisible();
  expect(fetchMock.mock.calls.at(-1)?.[0]).toBe("/docs/index.md");
});

import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App as RootApp } from "./App";
import { resetFirstRunForTests } from "./features/first-run/firstRun";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function emptyEnvelope(): Response {
  return jsonResponse({
    scope: "all",
    collection_revision: 0,
    partial: false,
    participants: [],
    data: [],
  });
}

function mockServer({
  freshInstall,
  demoMode = false,
}: {
  freshInstall: boolean;
  demoMode?: boolean;
}) {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse({
          registry_revision: 0,
          collection_revision: 0,
          vaults: [],
          demo_mode: demoMode,
        });
      }
      if (url.endsWith("/api/v1/whats-new")) {
        return jsonResponse({
          version: "2.8.0",
          previous_version: freshInstall ? null : "2.7.0",
          fresh_install: freshInstall,
          releases: [],
        });
      }
      if (url.endsWith("/api/settings")) {
        return jsonResponse({ settings: [], last_agent: null });
      }
      if (url.includes("/tree") || url.includes("/recent")) {
        return emptyEnvelope();
      }
      if (url.includes("/docs/")) {
        return new Response("# Hatchdoor documentation", { status: 200 });
      }
      return jsonResponse({}, 404);
    });
}

function renderApp() {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <RootApp />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  window.localStorage.clear();
  resetFirstRunForTests();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("the first-run checklist in the app (#419)", () => {
  it("opens a fresh install on the checklist instead of No Vaults Yet, and keeps its Help link", async () => {
    mockServer({ freshInstall: true });
    renderApp();

    expect(
      await screen.findByRole("heading", { name: "Set up Hatchdoor" }),
    ).toBeVisible();
    expect(screen.queryByText("No Vaults Yet")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "How does this work?" }),
    ).toBeVisible();
  });

  it("still shows after a restart, until it is closed", async () => {
    mockServer({ freshInstall: true });
    const first = renderApp();
    await screen.findByRole("heading", { name: "Set up Hatchdoor" });
    first.unmount();
    resetFirstRunForTests();

    renderApp();
    await screen.findByRole("heading", { name: "Set up Hatchdoor" });
    fireEvent.click(
      screen.getByRole("button", { name: "Close the checklist" }),
    );
    expect(await screen.findByText("No Vaults Yet")).toBeVisible();
    cleanup();
    resetFirstRunForTests();

    renderApp();
    expect(await screen.findByText("No Vaults Yet")).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "Set up Hatchdoor" }),
    ).not.toBeInTheDocument();
  });

  it("never shows on its own to an upgraded install", async () => {
    const fetchMock = mockServer({ freshInstall: false });
    renderApp();
    expect(await screen.findByText("No Vaults Yet")).toBeVisible();
    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/whats-new",
        expect.anything(),
      ),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      screen.queryByRole("heading", { name: "Set up Hatchdoor" }),
    ).not.toBeInTheDocument();
  });

  it("never shows in demo mode", async () => {
    mockServer({ freshInstall: true, demoMode: true });
    renderApp();
    expect(
      await screen.findByText("This demo has no Vaults loaded."),
    ).toBeVisible();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      screen.queryByRole("heading", { name: "Set up Hatchdoor" }),
    ).not.toBeInTheDocument();
  });

  it("reopens from Help after it was closed", async () => {
    window.localStorage.setItem("hatchdoor_first_run_dismissed", "1");
    resetFirstRunForTests();
    mockServer({ freshInstall: true });
    renderApp();
    expect(await screen.findByText("No Vaults Yet")).toBeVisible();

    fireEvent.click(screen.getAllByRole("button", { name: "Help" })[0]);
    fireEvent.click(
      await screen.findByRole("button", { name: /Setup checklist/ }),
    );

    expect(
      await screen.findByRole("heading", { name: "Set up Hatchdoor" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("complementary", { name: "Help" }),
    ).not.toBeInTheDocument();
  });

  it("offers no Setup checklist entry in demo mode Help", async () => {
    mockServer({ freshInstall: false, demoMode: true });
    renderApp();
    await screen.findByText("This demo has no Vaults loaded.");
    fireEvent.click(screen.getAllByRole("button", { name: "Help" })[0]);
    await screen.findByRole("complementary", { name: "Help" });
    expect(
      screen.queryByRole("button", { name: /Setup checklist/ }),
    ).not.toBeInTheDocument();
  });
});

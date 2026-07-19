import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultApp as App } from "./App";

const originalOnLine = navigator.onLine;
const originalClipboard = navigator.clipboard;

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  Object.defineProperty(navigator, "onLine", {
    configurable: true,
    value: originalOnLine,
  });
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: originalClipboard,
  });
  vi.restoreAllMocks();
});

describe("App mobile and topbar actions", () => {
  it("opens the mobile drawer from the measured topbar edge", async () => {
    vi.spyOn(window, "matchMedia").mockImplementation(
      ((query: string) =>
        ({
          matches: query.includes("max-width"),
          media: query,
          onchange: null,
          addListener: () => {},
          removeListener: () => {},
          addEventListener: () => {},
          removeEventListener: () => {},
          dispatchEvent: () => false,
        }) as MediaQueryList) as typeof window.matchMedia,
    );
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (this.classList.contains("app-topbar")) {
          return {
            x: 0,
            y: 0,
            width: 390,
            height: 104,
            top: 0,
            right: 390,
            bottom: 104,
            left: 0,
            toJSON: () => ({}),
          } as DOMRect;
        }

        return {
          x: 0,
          y: 0,
          width: 0,
          height: 0,
          top: 0,
          right: 0,
          bottom: 0,
          left: 0,
          toJSON: () => ({}),
        } as DOMRect;
      },
    );
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          name: "Vault",
          folders: [],
          notes: [],
        }),
        { status: 200 },
      ),
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    fireEvent.click(
      await screen.findByRole("button", { name: "Toggle explorer" }),
    );
    expect(document.querySelector<HTMLElement>(".app-shell")).toHaveStyle({
      "--mobile-drawer-top": "104px",
    });
    expect(document.querySelector<HTMLElement>(".hotbar")).toHaveStyle({
      flexShrink: "0",
    });
    expect(
      await screen.findByRole("button", { name: "Close explorer" }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Close explorer" }));
    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "Close explorer" }),
      ).toBeNull();
    });
  });

  it("executes remaining note actions from the mobile overflow menu", async () => {
    Object.defineProperty(navigator, "onLine", {
      configurable: true,
      value: false,
    });

    const clipboardWrite = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: clipboardWrite },
    });
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [{ title: "Home", slug: "home" }],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home/links")) {
          return new Response(
            JSON.stringify({ links: { outgoing: [], backlinks: [] } }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content: "# Home",
              },
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), {
            status: 200,
          });
        }

        return new Response("not found", { status: 404 });
      });

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    expect(
      await screen.findByRole("heading", { level: 2, name: "Home" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Offline")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    expect(
      screen.getByRole("button", { name: "More actions" }),
    ).toHaveAttribute("aria-expanded", "true");
    expect(screen.queryByRole("menuitem", { name: "Search" })).toBeNull();
    expect(
      screen.queryByRole("menuitem", { name: "Refresh vault" }),
    ).toBeNull();
    expect(
      screen.queryByRole("menuitem", { name: "Toggle properties" }),
    ).toBeNull();

    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Copy note link" }),
    );
    await waitFor(() => {
      expect(clipboardWrite).toHaveBeenCalledTimes(1);
    });

    expect(
      fetchSpy.mock.calls.some((call) =>
        String(call[0]).includes("/api/refresh"),
      ),
    ).toBe(false);
  });

  it("closes the note overflow menu when clicking outside", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [{ title: "Home", slug: "home" }],
            }),
            { status: 200 },
          );
        }
        if (url.includes("/api/note/home/links")) {
          return new Response(
            JSON.stringify({ links: { outgoing: [], backlinks: [] } }),
            { status: 200 },
          );
        }
        if (url.includes("/api/note/home")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content: "# Home",
              },
            }),
            { status: 200 },
          );
        }
        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), {
            status: 200,
          });
        }
        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    const actions = screen.getByRole("button", { name: "More actions" });
    fireEvent.click(actions);
    expect(actions).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getByRole("menuitem", { name: "Copy note link" }),
    ).toBeInTheDocument();

    fireEvent.pointerDown(document.body);

    await waitFor(() => {
      expect(actions).toHaveAttribute("aria-expanded", "false");
    });
    expect(
      screen.queryByRole("menuitem", { name: "Copy note link" }),
    ).toBeNull();
  });
});

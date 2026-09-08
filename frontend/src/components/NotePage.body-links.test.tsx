import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NOTE_PROPERTIES_COLLAPSED_KEY } from "../app/constants";
import { NotePage } from "./NotePage";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="pathname">{location.pathname}</div>;
}

const VAULT = "vault-1";

function mockVault(
  content: string,
  links: Record<string, string>,
  archived: string[] = [],
) {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/resolve-batch")) {
        // The real thing is a network round-trip, and the delay is the point:
        // the linked note renders with the previous note's resolved body until
        // it lands.
        await new Promise((resolve) => setTimeout(resolve, 5));
        const body = JSON.parse(String(init?.body ?? "{}")) as {
          targets: string[];
        };
        return jsonResponse({
          vault_id: VAULT,
          results: body.targets.map((target) => ({
            target,
            slug: links[target] ?? null,
            archived: archived.includes(target),
          })),
          asset_results: [],
        });
      }
      if (url.includes("/notes/")) {
        const slug = url.split("/notes/")[1].split(/[?#]/)[0];
        return jsonResponse({
          vault_id: VAULT,
          note: {
            title: slug,
            slug,
            relative_path: `${slug}.md`,
            content:
              slug === "home"
                ? content
                : "# Other\n\n## Section two\n\nBody.\n",
            content_hash: `hash-${slug}`,
            layer: null,
          },
        });
      }
      return jsonResponse({ error: "not found" }, 404);
    },
  );
}

function renderApp() {
  const props = {
    onActiveNoteChange: vi.fn(),
    onTagSelect: vi.fn(),
    propertiesCollapsedStorageKey: NOTE_PROPERTIES_COLLAPSED_KEY,
    vaultRevision: 0,
    writeEnabled: false,
    editRequestId: 0,
    vaults: [],
  };
  return render(
    <MemoryRouter initialEntries={[`/v/${VAULT}/n/home`]}>
      <LocationProbe />
      <Routes>
        <Route path="/v/:vaultId/n/:slug" element={<NotePage {...props} />} />
      </Routes>
    </MemoryRouter>,
  );
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.localStorage.clear();
});

describe("in-body note links", () => {
  it("routes client-side when a wikilink in the body is clicked", async () => {
    mockVault("Go to [[Other]] now.\n", { Other: "other" });
    renderApp();

    const link = await screen.findByRole("link", { name: "Other" });
    fireEvent.click(link, { button: 0 });

    await waitFor(() => {
      expect(screen.getByTestId("pathname").textContent).toBe(
        `/v/${VAULT}/n/other`,
      );
    });
  });

  // Routing the link takes the fragment away from the browser, so the jump to
  // the heading has to be made by the page itself. The linked note loads
  // before its own wikilinks resolve, so for a moment the body still holds the
  // note that was linked from: jumping then lands on nothing.
  it("scrolls to the heading a wikilink into another note names", async () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      cb(0);
      return 0;
    });

    mockVault("Jump to [[Other#Section two]].\n", {
      "Other#Section two": "other",
    });
    renderApp();

    const link = await screen.findByRole("link", { name: "Other" });
    fireEvent.click(link, { button: 0 });

    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalled();
    });
    expect(
      scrollIntoView.mock.instances[0] ?? scrollIntoView.mock.contexts[0],
    ).toBe(document.getElementById("section-two"));
  });

  it("scrolls to the heading a same-note wikilink names", async () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      cb(0);
      return 0;
    });

    mockVault("Jump to [[Home#Second part]].\n\n## Second part\n\nBody.\n", {
      "Home#Second part": "home",
    });
    renderApp();

    const link = await screen.findByRole("link", {
      name: "Home",
    });
    fireEvent.click(link, { button: 0 });

    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalled();
    });
    expect(screen.getByTestId("pathname").textContent).toBe(
      `/v/${VAULT}/n/home`,
    );
  });

  // An asset URL is a file the browser fetches, not a route. Handing it to the
  // router would swallow the click and land on a note that does not exist.
  it("leaves an asset link to the browser", async () => {
    mockVault(`[Download](/api/v1/vaults/${VAULT}/assets/report.csv)\n`, {});
    renderApp();

    const link = await screen.findByRole("link", { name: "Download" });
    fireEvent.click(link, { button: 0 });

    await waitFor(() => {
      expect(screen.getByTestId("pathname").textContent).toBe(
        `/v/${VAULT}/n/home`,
      );
    });
  });

  // An archived note is reached by the same route, through a branch of its own
  // that had the same bare anchor.
  it("routes client-side when an archived wikilink is clicked", async () => {
    mockVault("See [[Old Plan]].\n", { "Old Plan": "old-plan" }, ["Old Plan"]);
    renderApp();

    const link = await screen.findByRole("link", { name: "Old Plan" });
    fireEvent.click(link, { button: 0 });

    await waitFor(() => {
      expect(screen.getByTestId("pathname").textContent).toBe(
        `/v/${VAULT}/n/old-plan`,
      );
    });
  });

  // The browser re-jumped every time the fragment was followed. Latching the
  // jump per note rather than per navigation would leave the second click of a
  // link doing nothing at all for a reader who had scrolled away.
  it("jumps again when the same heading link is followed twice", async () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      cb(0);
      return 0;
    });

    mockVault("Jump to [[Home#Second part]].\n\n## Second part\n\nBody.\n", {
      "Home#Second part": "home",
    });
    renderApp();

    const link = await screen.findByRole("link", { name: "Home" });
    fireEvent.click(link, { button: 0 });
    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalledTimes(1);
    });

    fireEvent.click(link, { button: 0 });
    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalledTimes(2);
    });
  });
});

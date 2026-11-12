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
  return (
    <>
      <div data-testid="pathname">{location.pathname}</div>
      <div data-testid="hash">{location.hash}</div>
    </>
  );
}

const VAULT = "vault-1";

function mockVault(
  content: string,
  links: Record<string, string>,
  archived: string[] = [],
  otherContent = "# Other\n\n## Section two\n\nBody.\n",
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
            content: slug === "home" ? content : otherContent,
            content_hash: `hash-${slug}`,
            layer: null,
          },
        });
      }
      return jsonResponse({ error: "not found" }, 404);
    },
  );
}

function renderApp(
  overrides: Partial<Parameters<typeof NotePage>[0]> = {},
  entry = `/v/${VAULT}/n/home`,
) {
  const props = {
    onActiveNoteChange: vi.fn(),
    onTagSelect: vi.fn(),
    propertiesCollapsedStorageKey: NOTE_PROPERTIES_COLLAPSED_KEY,
    vaultRevision: null,
    writeEnabled: false,
    editRequestId: 0,
    vaults: [],
    ...overrides,
  };
  return render(
    <MemoryRouter initialEntries={[entry]}>
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

  // A link that names no note points into the one it is written in (#727).
  it("scrolls to the heading a link with no note name names", async () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      cb(0);
      return 0;
    });

    mockVault("Jump to [[#Second part]].\n\n## Second part\n\nBody.\n", {
      "/home.md#Second part": "home",
    });
    renderApp();

    fireEvent.click(await screen.findByRole("link", { name: "Second part" }), {
      button: 0,
    });

    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalled();
    });
    expect(
      scrollIntoView.mock.instances[0] ?? scrollIntoView.mock.contexts[0],
    ).toBe(document.getElementById("second-part"));
    expect(screen.getByTestId("pathname").textContent).toBe(
      `/v/${VAULT}/n/home`,
    );
  });

  // The live editor asks the same resolver (#544): a target only the server
  // can place still styles as resolved, and the click lands on the heading.
  it("resolves a wikilink in the editor through the server and opens the note at its heading", async () => {
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      cb(0);
      return 0;
    });
    mockVault("Jump to [[Other#Section two]].\n", {
      "Other#Section two": "other",
    });
    renderApp({ writeEnabled: true });

    await screen.findByRole("textbox", { name: "Note body" });
    const link = await waitFor(() => {
      const widget = document.querySelector(".cm-atomic-wiki-link-resolved");
      expect(widget).not.toBeNull();
      return widget as HTMLElement;
    });
    expect(link.textContent).toBe("Other");
    fireEvent.click(link, { button: 0 });

    await waitFor(() => {
      expect(screen.getByTestId("pathname").textContent).toBe(
        `/v/${VAULT}/n/other`,
      );
    });
    expect(screen.getByTestId("hash").textContent).toBe("#section-two");
    // The jump is the editor's own scroll, and it arms the trailing space the
    // way a heading jump in the reading view does.
    await waitFor(() => {
      expect(
        document.querySelector(".note-content")?.getAttribute("data-tail"),
      ).toBe("true");
    });
  });

  it("opens a link with no note name from the editor at its heading (#727)", async () => {
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      cb(0);
      return 0;
    });
    mockVault("Jump to [[#Second part]].\n\n## Second part\n\nBody.\n", {
      "/home.md#Second part": "home",
    });
    renderApp({ writeEnabled: true });

    await screen.findByRole("textbox", { name: "Note body" });
    const link = await waitFor(() => {
      const widget = document.querySelector(".cm-atomic-wiki-link-resolved");
      expect(widget).not.toBeNull();
      return widget as HTMLElement;
    });
    expect(link.textContent).toBe("Second part");
    fireEvent.click(link, { button: 0 });

    await waitFor(() => {
      expect(screen.getByTestId("hash").textContent).toBe("#second-part");
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

// A block id is a `^id` closing a paragraph. The page hides the text and
// leaves the paragraph an anchor, which a link's `#^id` lands on (#578).
describe("block links (#578)", () => {
  const OTHER =
    "# Other\n\n## Target\n\nFirst paragraph.\n\nThe one that matters. ^target\n";

  function trackScrolls() {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      cb(0);
      return 0;
    });
    const landedOn = () =>
      (scrollIntoView.mock.instances[0] ??
        scrollIntoView.mock.contexts[0]) as HTMLElement;
    return { scrollIntoView, landedOn };
  }

  it("lands on the paragraph a link into another note names", async () => {
    const { scrollIntoView, landedOn } = trackScrolls();
    mockVault(
      "Jump to [[Other#^target]].\n",
      { "Other#^target": "other" },
      [],
      OTHER,
    );
    renderApp();

    fireEvent.click(await screen.findByRole("link", { name: "Other" }), {
      button: 0,
    });

    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalled();
    });
    // The paragraph, not the heading of the same name.
    expect(landedOn().tagName).toBe("P");
    expect(landedOn().textContent).toBe("The one that matters.");
    expect(document.getElementById("target")?.tagName).toBe("H2");
  });

  it("lands on the paragraph a Markdown link names", async () => {
    const { scrollIntoView, landedOn } = trackScrolls();
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/resolve-batch")) {
          return jsonResponse({
            vault_id: VAULT,
            results: [],
            asset_results: [],
            note_link_results: [
              { target: "Other.md", slug: "other", archived: false },
            ],
          });
        }
        const slug = url.split("/notes/")[1]?.split(/[?#]/)[0] ?? "";
        return jsonResponse({
          vault_id: VAULT,
          note: {
            title: slug,
            slug,
            relative_path: `${slug}.md`,
            content: slug === "home" ? "[go](Other.md#^target)\n" : OTHER,
            content_hash: `hash-${slug}`,
            layer: null,
          },
        });
      },
    );
    renderApp();

    fireEvent.click(await screen.findByRole("link", { name: "go" }), {
      button: 0,
    });

    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalled();
    });
    expect(landedOn().textContent).toBe("The one that matters.");
  });

  it("lands on a paragraph of the same note", async () => {
    const { scrollIntoView, landedOn } = trackScrolls();
    mockVault("Jump to [[Home#^here|down]].\n\nFurther down. ^here\n", {
      "Home#^here": "home",
    });
    renderApp();

    fireEvent.click(await screen.findByRole("link", { name: "down" }), {
      button: 0,
    });

    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalled();
    });
    expect(landedOn().textContent).toBe("Further down.");
    expect(screen.queryByText(/\^here/)).toBeNull();
  });

  it("lands on a paragraph of the same note from a link with no note name (#727)", async () => {
    const { scrollIntoView, landedOn } = trackScrolls();
    mockVault("Jump to [[#^here|down]].\n\nFurther down. ^here\n", {
      "/home.md#^here": "home",
    });
    renderApp();

    fireEvent.click(await screen.findByRole("link", { name: "down" }), {
      button: 0,
    });

    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalled();
    });
    expect(landedOn().textContent).toBe("Further down.");
    expect(screen.getByTestId("pathname").textContent).toBe(
      `/v/${VAULT}/n/home`,
    );
  });

  it("lands on the paragraph when the page is loaded at the link's address", async () => {
    const { scrollIntoView, landedOn } = trackScrolls();
    mockVault("Top.\n\nFurther down. ^here\n", {});
    renderApp({}, `/v/${VAULT}/n/home#^here`);

    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalled();
    });
    expect(landedOn().textContent).toBe("Further down.");
  });

  it("opens the note at the top when the block is gone", async () => {
    const { scrollIntoView } = trackScrolls();
    mockVault("Top.\n\nFurther down. ^here\n", {});
    renderApp({}, `/v/${VAULT}/n/home#^gone`);

    await screen.findByText("Further down.");
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(scrollIntoView).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

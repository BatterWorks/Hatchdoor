import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type MockInstance,
} from "vitest";

import { HelpProvider } from "./HelpProvider";
import { useHelp } from "./useHelp";

const PAGES: Record<string, string> = {
  "/docs/index.md": [
    "# Hatchdoor documentation",
    "",
    "> [!tip]",
    "> Start read-only.",
    "",
    "Start with [Install Hatchdoor](get-started/install.md).",
    "",
    "## All pages",
    "",
    "- [Supported Markdown reference](reference/supported-markdown-reference.md)",
    "- [Hatchdoor on GitHub](https://github.com/BatterWorks/Hatchdoor)",
  ].join("\n"),
  "/docs/get-started/install.md": [
    "# Install Hatchdoor",
    "",
    "Start the container.",
    "",
    "## Where do I find my token?",
    "",
    "It is in `.env`.",
    "",
    "Back to [the manual](../index.md).",
  ].join("\n"),
  "/docs/reference/supported-markdown-reference.md": [
    "# Supported Markdown reference",
    "",
    "| Syntax | Result |",
    "| --- | --- |",
    "| `**a**` | bold |",
    "",
    "```base",
    "filters:",
    '  - file.inFolder("Projects")',
    "```",
  ].join("\n"),
  "/docs/guides/how-to-deploy-hatchdoor-with-an-agent.md":
    "# How to deploy Hatchdoor with an agent\n\nGive your agent one line.",
};

const SEARCH: Record<string, unknown[]> = {
  token: [
    {
      name: "get-started/install",
      title: "Install Hatchdoor",
      excerpt: "Where do I find my **token**? It is in `.env`.",
    },
  ],
};

let fetchMock: MockInstance<typeof fetch>;
let scrolled: string[];
const originalScrollIntoView = Element.prototype.scrollIntoView;

beforeEach(() => {
  scrolled = [];
  Element.prototype.scrollIntoView = function (this: Element) {
    scrolled.push(this.id);
  };
  fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input) => {
      const url = new URL(String(input), "http://localhost");
      if (url.pathname === "/docs/search") {
        const hits = SEARCH[url.searchParams.get("q") ?? ""] ?? [];
        return new Response(JSON.stringify({ results: hits }));
      }
      const page = PAGES[url.pathname];
      return page
        ? new Response(page, { status: 200 })
        : new Response("not found", { status: 404 });
    });
});

afterEach(() => {
  cleanup();
  Element.prototype.scrollIntoView = originalScrollIntoView;
  vi.restoreAllMocks();
});

/** A stand-in for any feature that opens Help. */
function Opener({ page, heading }: { page?: string; heading?: string }) {
  const { openHelp, isOpen } = useHelp();
  return (
    <button type="button" onClick={() => openHelp(page, heading)}>
      {isOpen ? "Help is open" : "Open help"}
    </button>
  );
}

function renderHelp(
  opener: { page?: string; heading?: string } = {},
  props: { demoMode?: boolean } = {},
) {
  render(
    <HelpProvider {...props}>
      <Opener {...opener} />
    </HelpProvider>,
  );
  const trigger = screen.getByRole("button", { name: "Open help" });
  trigger.focus();
  fireEvent.click(trigger);
  return {
    trigger,
    panel: screen.getByRole("complementary", { name: "Help" }),
  };
}

function requestedUrls(): string[] {
  return fetchMock.mock.calls.map(([input]) => String(input));
}

describe("Help panel", () => {
  it("opens at Home and renders the manual like a note", async () => {
    const { panel } = renderHelp();

    expect(
      await within(panel).findByRole("heading", {
        name: "Hatchdoor documentation",
      }),
    ).toBeVisible();
    expect(within(panel).getByText("Home")).toBeVisible();
    // Callouts go through the note renderer's callout handling.
    expect(panel.querySelector(".callout")).not.toBeNull();
    expect(requestedUrls()).toEqual(["/docs/index.md"]);
  });

  it("follows links between pages inside Help and goes back", async () => {
    const { panel } = renderHelp();

    fireEvent.click(
      await within(panel).findByRole("link", { name: "Install Hatchdoor" }),
    );
    expect(
      await within(panel).findByRole("heading", { name: "Install Hatchdoor" }),
    ).toBeVisible();
    expect(requestedUrls()).toContain("/docs/get-started/install.md");

    fireEvent.click(within(panel).getByRole("link", { name: "the manual" }));
    expect(
      await within(panel).findByRole("heading", {
        name: "Hatchdoor documentation",
      }),
    ).toBeVisible();

    fireEvent.click(within(panel).getByRole("button", { name: "Back" }));
    expect(
      await within(panel).findByRole("heading", { name: "Install Hatchdoor" }),
    ).toBeVisible();
  });

  it("opens external links in a new tab, not inside Help", async () => {
    const { panel } = renderHelp();

    const link = await within(panel).findByRole("link", {
      name: "Hatchdoor on GitHub",
    });
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });

  it("opens a given page scrolled to a given heading", async () => {
    const { panel } = renderHelp({
      page: "get-started/install",
      heading: "where-do-i-find-my-token",
    });

    const heading = await within(panel).findByRole("heading", {
      name: "Where do I find my token?",
    });
    await waitFor(() => expect(scrolled).toContain(heading.id));
    expect(heading.id).toBe("help-where-do-i-find-my-token");
  });

  it("finds pages by search and shows an empty state", async () => {
    const { panel } = renderHelp();
    await within(panel).findByRole("heading", {
      name: "Hatchdoor documentation",
    });
    const box = within(panel).getByRole("searchbox", {
      name: "Search the manual",
    });

    fireEvent.change(box, { target: { value: "token" } });
    const result = await within(panel).findByRole("button", {
      name: /Install Hatchdoor/,
    });
    expect(within(panel).getByText("1 page matches “token”")).toBeVisible();
    expect(
      within(result).getByText("Where do I find my token? It is in .env."),
    ).toBeVisible();

    fireEvent.change(box, { target: { value: "zanzibar" } });
    expect(
      await within(panel).findByRole("heading", {
        name: "Nothing matches “zanzibar”",
      }),
    ).toBeVisible();

    fireEvent.change(box, { target: { value: "token" } });
    fireEvent.click(
      await within(panel).findByRole("button", { name: /Install Hatchdoor/ }),
    );
    expect(
      await within(panel).findByRole("heading", { name: "Install Hatchdoor" }),
    ).toBeVisible();
    expect(box).toHaveValue("");
  });

  it("renders a base block as code and calls no Vault endpoint", async () => {
    const { panel } = renderHelp({
      page: "reference/supported-markdown-reference",
    });

    await within(panel).findByRole("heading", {
      name: "Supported Markdown reference",
    });
    expect(
      await within(panel).findByText(/file\.inFolder\("Projects"\)/),
    ).toBeVisible();
    expect(panel.querySelector(".saved-query")).toBeNull();
    expect(within(panel).getByRole("table")).toBeVisible();
    for (const url of requestedUrls()) {
      expect(url.startsWith("/docs/")).toBe(true);
    }
  });

  it("toggles full width", async () => {
    const { panel } = renderHelp();

    const toggle = within(panel).getByRole("button", {
      name: "Open full width",
    });
    fireEvent.click(toggle);
    expect(panel).toHaveClass("is-full");
    fireEvent.click(
      within(panel).getByRole("button", { name: "Back to side panel" }),
    );
    expect(panel).not.toHaveClass("is-full");
  });

  it("closes on Escape and returns focus where it was", async () => {
    const { trigger, panel } = renderHelp();
    await waitFor(() => expect(panel).toHaveFocus());

    fireEvent.keyDown(within(panel).getByRole("searchbox"), { key: "Escape" });

    expect(screen.queryByRole("complementary", { name: "Help" })).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it("leaves Escape to a dialog open above it", () => {
    renderHelp();
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    const input = document.createElement("input");
    dialog.append(input);
    document.body.append(dialog);

    fireEvent.keyDown(input, { key: "Escape" });

    expect(
      screen.getByRole("complementary", { name: "Help" }),
    ).toBeInTheDocument();
    dialog.remove();
  });

  it("closes from its close button", () => {
    const { trigger, panel } = renderHelp();

    fireEvent.click(within(panel).getByRole("button", { name: "Close Help" }));

    expect(screen.queryByRole("complementary", { name: "Help" })).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it("puts installing first on Home in demo mode", async () => {
    const { panel } = renderHelp({}, { demoMode: true });

    const start = await within(panel).findByRole("navigation", {
      name: "Start here",
    });
    const cards = within(start).getAllByRole("button");
    expect(cards.map((card) => card.textContent)).toEqual([
      expect.stringContaining("Install Hatchdoor"),
      expect.stringContaining("Let your agent install it"),
    ]);
    // The start cards come before the manual's own Home page.
    const home = within(panel).getByRole("heading", {
      name: "Hatchdoor documentation",
    });
    expect(
      start.compareDocumentPosition(home) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    fireEvent.click(cards[1]);
    expect(
      await within(panel).findByRole("heading", {
        name: "How to deploy Hatchdoor with an agent",
      }),
    ).toBeVisible();
  });

  it("shows no start cards outside demo mode", async () => {
    const { panel } = renderHelp();

    await within(panel).findByRole("heading", {
      name: "Hatchdoor documentation",
    });
    expect(
      within(panel).queryByRole("navigation", { name: "Start here" }),
    ).toBeNull();
  });

  it("explains a page it cannot show", async () => {
    const { panel } = renderHelp({ page: "whats-new" });

    expect(
      await within(panel).findByRole("heading", { name: "Page not available" }),
    ).toBeVisible();
    await act(async () => {
      fireEvent.click(
        within(panel).getByRole("button", {
          name: "Go to the manual's home page",
        }),
      );
    });
    expect(
      await within(panel).findByRole("heading", {
        name: "Hatchdoor documentation",
      }),
    ).toBeVisible();
  });
});

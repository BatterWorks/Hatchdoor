import { act, cleanup, render } from "@testing-library/react";
import { MemoryRouter, useNavigate } from "react-router-dom";
import { afterEach, describe, expect, it } from "vitest";

import type { VaultId } from "../types";
import { pageTitle, usePageTitle, type TitledNote } from "./pageTitle";

const VAULT = "vault-a" as VaultId;
const OTHER_VAULT = "vault-b" as VaultId;

function note(slug: string, title: string, vaultId = VAULT): TitledNote {
  return { vaultId, slug, title };
}

// The server writes the same names into the page a demo instance sends, all
// but the last two: it leaves an unknown address as the bare name.
const ROUTE_TITLES = [
  ["/", "Hatchdoor"],
  ["/graph", "Graph · Hatchdoor"],
  ["/stats", "Stats · Hatchdoor"],
  ["/settings", "Settings · Hatchdoor"],
  ["/graph/", "Graph · Hatchdoor"],
  ["/nope", "Page not found · Hatchdoor"],
  ["/n/old-link", "Page not found · Hatchdoor"],
];

describe("pageTitle", () => {
  it("gives every page that is not a note its fixed name", () => {
    for (const [pathname, title] of ROUTE_TITLES) {
      expect(pageTitle(pathname, null), pathname).toBe(title);
    }
  });

  it("names the open note, without its Vault", () => {
    expect(pageTitle("/v/vault-a/n/beacon", note("beacon", "Beacon"))).toBe(
      "Beacon · Hatchdoor",
    );
  });

  it("matches a slug the address carries percent-encoded", () => {
    expect(
      pageTitle("/v/vault-a/n/caf%C3%A9%20notes", note("café notes", "Café")),
    ).toBe("Café · Hatchdoor");
  });

  it("collapses whitespace in the note title, as the server does", () => {
    expect(
      pageTitle("/v/vault-a/n/beacon", note("beacon", "  Bea\n con  ")),
    ).toBe("Bea con · Hatchdoor");
  });

  it("is the bare name while a note is loading, failed, or untitled", () => {
    expect(pageTitle("/v/vault-a/n/beacon", null)).toBe("Hatchdoor");
    expect(pageTitle("/v/vault-a/n/beacon", note("beacon", "  "))).toBe(
      "Hatchdoor",
    );
  });

  it("never shows a note other than the one the address names", () => {
    const previous = note("beacon", "Beacon");
    expect(pageTitle("/v/vault-a/n/harbour", previous)).toBe("Hatchdoor");
    expect(
      pageTitle("/v/vault-a/n/beacon", note("beacon", "Beacon", OTHER_VAULT)),
    ).toBe("Hatchdoor");
  });

  it("drops the note title on every other route", () => {
    const previous = note("beacon", "Beacon");
    expect(pageTitle("/", previous)).toBe("Hatchdoor");
    expect(pageTitle("/graph", previous)).toBe("Graph · Hatchdoor");
    expect(pageTitle("/stats", previous)).toBe("Stats · Hatchdoor");
    expect(pageTitle("/settings", previous)).toBe("Settings · Hatchdoor");
  });
});

describe("usePageTitle", () => {
  let navigate: ReturnType<typeof useNavigate>;

  function Shell({ activeNote }: { activeNote: TitledNote | null }) {
    navigate = useNavigate();
    usePageTitle(activeNote);
    return null;
  }

  function shell(activeNote: TitledNote | null) {
    return (
      <MemoryRouter initialEntries={["/v/vault-a/n/beacon"]}>
        <Shell activeNote={activeNote} />
      </MemoryRouter>
    );
  }

  afterEach(() => {
    cleanup();
    document.title = "";
  });

  it("follows navigation and the loaded note without a reload", () => {
    const view = render(shell(null));
    expect(document.title).toBe("Hatchdoor");

    view.rerender(shell(note("beacon", "Beacon")));
    expect(document.title).toBe("Beacon · Hatchdoor");

    // The shell still holds the first note when the address moves on.
    act(() => navigate("/v/vault-a/n/harbour"));
    expect(document.title).toBe("Hatchdoor");

    view.rerender(shell(note("harbour", "Harbour")));
    expect(document.title).toBe("Harbour · Hatchdoor");

    act(() => navigate("/graph"));
    expect(document.title).toBe("Graph · Hatchdoor");

    act(() => navigate("/"));
    expect(document.title).toBe("Hatchdoor");
  });

  it("writes each page's name to the tab", () => {
    render(shell(note("beacon", "Beacon")));
    for (const [pathname, title] of ROUTE_TITLES) {
      act(() => navigate(pathname));
      expect(document.title, pathname).toBe(title);
    }
  });

  it("follows a rename that moves the note to a new address", () => {
    const view = render(shell(note("beacon", "Beacon")));
    const renamed = note("lighthouse", "Lighthouse");

    // The note changes first, the address a moment later.
    view.rerender(shell(renamed));
    expect(document.title).toBe("Hatchdoor");

    act(() => navigate("/v/vault-a/n/lighthouse"));
    expect(document.title).toBe("Lighthouse · Hatchdoor");
  });

  it("follows a rename that keeps the note's address", () => {
    const view = render(shell(note("beacon", "Beacon")));
    expect(document.title).toBe("Beacon · Hatchdoor");

    view.rerender(shell(note("beacon", "Lighthouse")));
    expect(document.title).toBe("Lighthouse · Hatchdoor");
  });
});

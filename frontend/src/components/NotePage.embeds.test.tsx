import { EditorView } from "@codemirror/view";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NOTE_PROPERTIES_COLLAPSED_KEY } from "../app/constants";
import { NotePage } from "./NotePage";
import { NoteEmbed } from "./note-page/NoteEmbed";
import { NoteEmbedContext } from "./note-page/noteEmbeds";
import { NotePreview } from "./note-page/NotePreview";

// Note embeds in the Reading view (#699): `![[Note]]`, `![[Note#Heading]]`
// and `![[Note#^id]]` draw the other note's content, and anything that stops
// that draws the plain link #573 gave them.

const VAULT = "vault-1";

/** Each note by slug; a wikilink target resolves by its name before any
 * `#` or `^`, lowercased, so `[[Sections#Second]]` names `sections`. */
type Notes = Record<string, string>;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function slugOf(target: string): string {
  return target.split(/[#^]/, 1)[0].trim().toLowerCase().replace(/ /g, "-");
}

function mockVault(notes: Notes) {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes("/resolve-batch")) {
          const body = JSON.parse(String(init?.body ?? "{}")) as {
            targets: string[];
            asset_targets: string[];
          };
          return jsonResponse({
            vault_id: VAULT,
            results: body.targets.map((target) => {
              const slug = slugOf(target);
              return {
                target,
                slug: slug in notes ? slug : null,
                archived: false,
              };
            }),
            asset_results: body.asset_targets.map((target) => ({
              target,
              path: null,
            })),
          });
        }
        const match = url.match(/\/vaults\/vault-1\/notes\/([^/?]+)$/);
        const slug = match ? decodeURIComponent(match[1]) : null;
        if (slug !== null && slug in notes) {
          return jsonResponse({
            vault_id: VAULT,
            note: {
              title: slug,
              slug,
              relative_path: `${slug}.md`,
              content: notes[slug],
              content_hash: `hash-${slug}`,
              layer: null,
            },
            saved_queries: [],
          });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );
}

function renderNote(slug: string, writeEnabled = false) {
  return render(
    <MemoryRouter initialEntries={[`/v/${VAULT}/n/${slug}`]}>
      <Routes>
        <Route
          path="/v/:vaultId/n/:slug"
          element={
            <NotePage
              onActiveNoteChange={vi.fn()}
              onTagSelect={vi.fn()}
              propertiesCollapsedStorageKey={NOTE_PROPERTIES_COLLAPSED_KEY}
              vaultRevision={null}
              writeEnabled={writeEnabled}
              editRequestId={0}
              vaults={[]}
            />
          }
        />
      </Routes>
    </MemoryRouter>,
  );
}

/** The embed frame whose source link reads `label`. */
async function findEmbed(label: string): Promise<HTMLElement> {
  return waitFor(() => {
    const frame = screen
      .getAllByTestId("note-embed")
      .find(
        (embed) =>
          embed.querySelector(".note-embed-source")?.textContent === label &&
          embed.getAttribute("aria-busy") === null,
      );
    if (!frame) {
      throw new Error(`no loaded embed of ${label}`);
    }
    return frame;
  });
}

/** The plain link an embed falls back to. */
async function findFallback(label: string): Promise<HTMLElement> {
  return waitFor(() => {
    const link = [
      ...document.querySelectorAll<HTMLElement>(
        "[data-note-embed-fallback], .broken-link",
      ),
    ].find((element) => element.textContent === label);
    if (!link) {
      throw new Error(`no fallback link for ${label}`);
    }
    return link;
  });
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.localStorage.clear();
});

const SECTIONS = [
  "# Sections",
  "",
  "## First",
  "",
  "first text",
  "",
  "## Second",
  "",
  "second text",
  "",
  "### Sub",
  "",
  "sub text",
  "",
  "## Third",
  "",
  "third text",
  "",
].join("\n");

const BLOCKS = [
  "Intro paragraph.",
  "",
  "Target paragraph. ^para",
  "",
  "- one",
  "- two ^item",
  "",
].join("\n");

describe("note embeds in the Reading view (#699)", () => {
  it("draws a whole note, without its frontmatter, inside a frame naming it", async () => {
    mockVault({
      home: "Before.\n\n![[Whole]]\n\nAfter.\n",
      whole: "---\ntags: [secret-tag]\n---\n# whole\n\nWhole body text.\n",
    });
    renderNote("home");

    const frame = await findEmbed("Whole");
    await waitFor(() =>
      expect(frame.textContent).toContain("Whole body text."),
    );
    expect(frame.textContent).not.toContain("secret-tag");
    // The title the frame already names is not said twice (#530).
    expect(frame.querySelector("h1")?.hidden).toBe(true);
    const source = frame.querySelector(".note-embed-source");
    expect(source?.getAttribute("href")).toBe(`/v/${VAULT}/n/whole`);
    // Not inside a paragraph, which cannot hold a block.
    expect(frame.closest("p")).toBeNull();
    // Its headings and blocks keep no ids, which belong to the other note.
    expect(frame.querySelector("[id]")).toBeNull();
  });

  it("draws only a heading's section, down to the next heading of its level", async () => {
    mockVault({ home: "![[Sections#Second]]\n", sections: SECTIONS });
    renderNote("home");

    const frame = await findEmbed("Sections");
    await waitFor(() => expect(frame.textContent).toContain("second text"));
    expect(frame.textContent).toContain("sub text");
    expect(frame.textContent).not.toContain("first text");
    expect(frame.textContent).not.toContain("third text");
    expect(
      frame.querySelector(".note-embed-source")?.getAttribute("href"),
    ).toBe(`/v/${VAULT}/n/sections#second`);
  });

  it("draws only the block carrying the id, a list item as a list", async () => {
    mockVault({
      home: "![[Blocks#^para]]\n\n![[Blocks#^item|the item]]\n",
      blocks: BLOCKS,
    });
    renderNote("home");

    const paragraph = await findEmbed("Blocks");
    await waitFor(() =>
      expect(paragraph.textContent).toContain("Target paragraph."),
    );
    expect(paragraph.textContent).not.toContain("Intro paragraph.");
    expect(paragraph.textContent).not.toContain("^para");

    const item = await findEmbed("the item");
    await waitFor(() => expect(item.querySelector("li")).not.toBeNull());
    expect(item.querySelectorAll("li")).toHaveLength(1);
    expect(item.textContent).toContain("two");
    expect(item.textContent).not.toContain("one");
  });

  it("splits a paragraph around an embed written inside it", async () => {
    mockVault({
      home: "Before the embed\n![[Whole]]\nafter the embed.\n",
      whole: "Whole body text.\n",
    });
    const { container } = renderNote("home");

    const frame = await findEmbed("Whole");
    const paragraphs = [...container.querySelectorAll(".note-body > p")].map(
      (p) => p.textContent?.trim(),
    );
    expect(paragraphs).toEqual(["Before the embed", "after the embed."]);
    expect(frame.closest("p")).toBeNull();
  });

  it("evaluates a saved query in the embedded note against that note", async () => {
    const fetchSpy = mockVault({
      home: "![[Dashboard]]\n",
      dashboard: "Before.\n\n```base\nfilters: 'price < 10'\n```\n",
    });
    const passThrough = fetchSpy.getMockImplementation()!;
    fetchSpy.mockImplementation(async (input, init) =>
      String(input).endsWith("/notes/dashboard/saved-queries")
        ? jsonResponse({
            collection_revision: 0,
            data: {
              queries: [
                {
                  source: "filters: 'price < 10'",
                  status: "populated",
                  columns: [{ id: "price", label: "price" }],
                  rows: [
                    {
                      vault_id: VAULT,
                      title: "Newspaper",
                      slug: "newspaper",
                      relative_path: "Newspaper.md",
                      cells: [8],
                    },
                  ],
                },
              ],
            },
          })
        : passThrough(input, init),
    );
    renderNote("home");

    const frame = await findEmbed("Dashboard");
    await waitFor(() => expect(frame.querySelector("table")).not.toBeNull());
    expect(frame.querySelector("td")?.textContent).toBe("8");
  });

  it("draws an embed written inside a callout", async () => {
    const errors = vi.spyOn(console, "error");
    mockVault({
      home: "> [!note] Read this\n> ![[Whole]]\n",
      whole: "Whole body text.\n",
    });
    renderNote("home");

    const frame = await findEmbed("Whole");
    await waitFor(() =>
      expect(frame.textContent).toContain("Whole body text."),
    );
    expect(frame.closest(".callout")).not.toBeNull();
    expect(frame.closest("p")).toBeNull();
    expect(
      errors.mock.calls.some((call) => String(call[0]).includes("descendant")),
    ).toBe(false);
  });

  it("falls back to the link for a missing note, heading or block", async () => {
    mockVault({
      home: [
        "![[Missing]]",
        "",
        "![[Sections#Nowhere]]",
        "",
        "![[Blocks#^nothere]]",
        "",
      ].join("\n"),
      sections: SECTIONS,
      blocks: BLOCKS,
    });
    renderNote("home");

    const missing = await findFallback("Missing");
    expect(missing.classList.contains("broken-link")).toBe(true);
    // The link an embed falls back to lands on what it named.
    expect((await findFallback("Sections")).getAttribute("href")).toBe(
      `/v/${VAULT}/n/sections#nowhere`,
    );
    expect(
      decodeURIComponent((await findFallback("Blocks")).getAttribute("href")!),
    ).toBe(`/v/${VAULT}/n/blocks#^nothere`);
    expect(document.querySelector("[data-testid=note-embed]")).toBeNull();
  });

  it("falls back to the link when the embedded note cannot be read", async () => {
    const fetchSpy = mockVault({ home: "![[Locked]]\n", locked: "secret" });
    const passThrough = fetchSpy.getMockImplementation()!;
    fetchSpy.mockImplementation(async (input, init) =>
      String(input).endsWith("/notes/locked")
        ? jsonResponse({ error: "forbidden" }, 403)
        : passThrough(input, init),
    );
    renderNote("home");

    await findFallback("Locked");
    expect(document.body.textContent).not.toContain("secret");
  });

  it("stops a note embedding itself", async () => {
    mockVault({ home: "Home text.\n\n![[Home]]\n" });
    renderNote("home");

    await findFallback("Home");
    expect(document.querySelector("[data-testid=note-embed]")).toBeNull();
  });

  it("stops an A to B to A cycle at the second A", async () => {
    mockVault({ a: "A text.\n\n![[B]]\n", b: "B text.\n\n![[A]]\n" });
    renderNote("a");

    const frame = await findEmbed("B");
    await waitFor(() => expect(frame.textContent).toContain("B text."));
    const back = await findFallback("A");
    expect(frame.contains(back)).toBe(true);
    expect(screen.getAllByTestId("note-embed")).toHaveLength(1);
  });

  it("nests three levels deep and draws the fourth as its link", async () => {
    mockVault({
      n0: "Level 0.\n\n![[N1]]\n",
      n1: "Level 1.\n\n![[N2]]\n",
      n2: "Level 2.\n\n![[N3]]\n",
      n3: "Level 3.\n\n![[N4]]\n",
      n4: "Level 4.\n",
    });
    renderNote("n0");

    const third = await findEmbed("N3");
    await waitFor(() => expect(third.textContent).toContain("Level 3."));
    await findFallback("N4");
    expect(screen.getAllByTestId("note-embed")).toHaveLength(3);
    expect(document.body.textContent).not.toContain("Level 4.");
  });

  it("never reaches another Vault for a target that names one", async () => {
    const fetchSpy = mockVault({
      home: "![[vault-2/Elsewhere]]\n\n![[Vault 2:Elsewhere]]\n",
    });
    const passThrough = fetchSpy.getMockImplementation()!;
    // Another Vault does hold that note; this one does not.
    fetchSpy.mockImplementation(async (input, init) =>
      String(input).includes("/vaults/vault-2/")
        ? jsonResponse({
            vault_id: "vault-2",
            note: {
              title: "Elsewhere",
              slug: "elsewhere",
              relative_path: "Elsewhere.md",
              content: "Other Vault text.",
              content_hash: "hash-elsewhere",
              layer: null,
            },
          })
        : passThrough(input, init),
    );
    renderNote("home");

    await findFallback("vault-2/Elsewhere");
    await findFallback("Vault 2:Elsewhere");
    const urls = fetchSpy.mock.calls.map(([input]) => String(input));
    expect(urls.some((url) => url.includes("vault-2"))).toBe(false);
    expect(document.body.textContent).not.toContain("Other Vault text.");
  });

  it("draws an embed for another Vault than its page's as its link", async () => {
    const fetchSpy = mockVault({ whole: "Whole body text.\n" });
    render(
      <MemoryRouter>
        <NoteEmbedContext.Provider
          value={{ vaultId: VAULT, chain: ["home"], revision: null }}
        >
          <NoteEmbed
            vaultId="vault-2"
            target={{ slug: "whole", archived: false, anchor: "" }}
          >
            Whole
          </NoteEmbed>
        </NoteEmbedContext.Provider>
      </MemoryRouter>,
    );

    await findFallback("Whole");
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("gives embedded content no ids and keeps its footnotes and references", async () => {
    mockVault({
      home: "## Mine\n\n![[Other#Part]]\n",
      other: [
        "## Part",
        "",
        "See the note[^1] and [the site][site]. ^pid",
        "",
        "- a",
        "- b",
        "",
        "> [!note] Title",
        "> body ^cid",
        "",
        "## Rest",
        "",
        "Rest text.",
        "",
        "[^1]: The footnote.",
        "",
        "[site]: https://example.com",
        "",
      ].join("\n"),
    });
    renderNote("home");

    const frame = await findEmbed("Other");
    await waitFor(() => expect(frame.textContent).toContain("The footnote."));
    expect(frame.textContent).not.toContain("Rest text.");
    expect(
      frame.querySelector('a[href="https://example.com"]')?.textContent,
    ).toBe("the site");
    expect(frame.querySelector("[id]")).toBeNull();
    // The page's own heading keeps its id.
    expect(screen.getByRole("heading", { name: "Mine" }).id).toBe("mine");
  });

  it("draws a hand-written embed link that names no note as a link", async () => {
    const fetchSpy = mockVault({ home: "[odd](/__embed__/bogus)\n" });
    const passThrough = fetchSpy.getMockImplementation()!;
    fetchSpy.mockImplementation(async (input, init) =>
      String(input).endsWith("/notes/bogus")
        ? jsonResponse({ vault_id: VAULT })
        : passThrough(input, init),
    );
    renderNote("home");

    await findFallback("odd");
  });

  it("draws embeds in the source-mode preview too", async () => {
    mockVault({ home: "", whole: "Whole body text.\n" });
    render(
      <MemoryRouter>
        <NotePreview
          vaultId={VAULT}
          content="![[Whole]]\n"
          relativePath="home.md"
          slug="home"
        />
      </MemoryRouter>,
    );

    const frame = await findEmbed("Whole");
    await waitFor(() =>
      expect(frame.textContent).toContain("Whole body text."),
    );
  });

  it("leaves the live editor showing the embed's text", async () => {
    const fetchSpy = mockVault({
      home: "![[Whole]]\n",
      whole: "Whole body text.\n",
    });
    renderNote("home", true);

    const content = await waitFor(() => {
      const element = document.querySelector<HTMLElement>(
        ".live-editor .cm-content",
      );
      if (!element) throw new Error("no live editor yet");
      return element;
    });
    const view = EditorView.findFromDOM(content);
    expect(view?.state.doc.toString()).toContain("![[Whole]]");
    expect(document.querySelector("[data-testid=note-embed]")).toBeNull();
    expect(
      fetchSpy.mock.calls.some(([input]) =>
        String(input).endsWith("/notes/whole"),
      ),
    ).toBe(false);
  });
});

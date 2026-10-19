import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NOTE_PROPERTIES_COLLAPSED_KEY } from "../app/constants";
import { NotePage } from "./NotePage";
import { NotePreview } from "./note-page/NotePreview";

const VAULT = "vault-1";

// Comments, a commented-out heading, code that looks like a comment, and a
// note embed (#573).
const CONTENT = [
  "Visible %%secret one%% tail",
  "",
  "<!-- secret two",
  "still secret -->",
  "",
  "%%",
  "## Secret heading",
  "%%",
  "",
  "## Shown heading",
  "",
  "Inline `%%kept%%` code.",
  "",
  "```",
  "%%fenced%%",
  "<!-- fenced too -->",
  "```",
  "",
  "![[Other note]]",
  "",
  "![[Missing note]]",
  "",
].join("\n");

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function mockVault() {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/resolve-batch")) {
        const body = JSON.parse(String(init?.body ?? "{}")) as {
          targets: string[];
          asset_targets: string[];
        };
        return jsonResponse({
          vault_id: VAULT,
          results: body.targets.map((target) => ({
            target,
            slug: target === "Other note" ? "other-note" : null,
            archived: false,
          })),
          asset_results: body.asset_targets.map((target) => ({
            target,
            path: null,
          })),
        });
      }
      if (url.includes("/notes/home")) {
        return jsonResponse({
          vault_id: VAULT,
          note: {
            title: "home",
            slug: "home",
            relative_path: "home.md",
            content: CONTENT,
            content_hash: "hash-home",
            layer: null,
          },
        });
      }
      return jsonResponse({ error: "not found" }, 404);
    },
  );
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.localStorage.clear();
});

function expectReaderView(body: HTMLElement) {
  const text = body.textContent ?? "";
  expect(text).toContain("Visible");
  expect(text).toContain("tail");
  expect(text).not.toContain("secret");
  expect(text).not.toContain("%%secret");
  expect(text).toContain("%%kept%%");
  expect(text).toContain("%%fenced%%");
  expect(text).toContain("<!-- fenced too -->");

  const embed = screen.getByRole("link", { name: "Other note" });
  expect(embed.getAttribute("href")).toBe(`/v/${VAULT}/n/other-note`);
  // What a missing wikilink draws as.
  expect(
    body.querySelector('.broken-link[title="Missing: Missing note"]'),
  ).not.toBeNull();
  expect(body.querySelector("img")).toBeNull();
}

describe("comments and note embeds (#573)", () => {
  it("hides comments and draws note embeds as links in the Reading view", async () => {
    mockVault();
    const onHeadingsChange = vi.fn();
    const { container } = render(
      <MemoryRouter initialEntries={[`/v/${VAULT}/n/home`]}>
        <Routes>
          <Route
            path="/v/:vaultId/n/:slug"
            element={
              <NotePage
                onActiveNoteChange={vi.fn()}
                onTagSelect={vi.fn()}
                onHeadingsChange={onHeadingsChange}
                propertiesCollapsedStorageKey={NOTE_PROPERTIES_COLLAPSED_KEY}
                vaultRevision={null}
                writeEnabled={false}
                editRequestId={0}
                vaults={[]}
              />
            }
          />
        </Routes>
      </MemoryRouter>,
    );

    await screen.findByRole("link", { name: "Other note" });
    expectReaderView(container);

    // The commented heading is neither drawn nor listed, and the one after
    // it keeps the id the table of contents jumps to.
    expect(
      screen.queryByRole("heading", { name: "Secret heading" }),
    ).toBeNull();
    const shown = screen.getByRole("heading", { name: "Shown heading" });
    expect(shown.id).toBe("shown-heading");
    await waitFor(() => {
      const last = onHeadingsChange.mock.calls.at(-1)?.[0] ?? [];
      // Line 10 of the body, where it sits in the file, so a jump to it
      // lands on the right line.
      expect(
        last.map((h: { text: string; sourceLine: number }) => [
          h.text,
          h.sourceLine,
        ]),
      ).toEqual([["Shown heading", 10]]);
    });
  });

  it("hides comments and draws note embeds as links in the source preview", async () => {
    mockVault();
    const { container } = render(
      <MemoryRouter>
        <NotePreview vaultId={VAULT} content={CONTENT} relativePath="home.md" />
      </MemoryRouter>,
    );

    await screen.findByRole("link", { name: "Other note" });
    expectReaderView(container);
    expect(
      screen.queryByRole("heading", { name: "Secret heading" }),
    ).toBeNull();
  });
});

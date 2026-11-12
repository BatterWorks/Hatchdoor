import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  cachedAssetHref,
  resolveNoteTargets,
  useResolvedWikilinks,
} from "./wikilinks";

const VAULT_ID = "vault-1";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("useResolvedWikilinks asset targets (#158)", () => {
  it("asks the server to resolve embed targets and renders the answer", async () => {
    const bodies: unknown[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        bodies.push(JSON.parse(String(init?.body)));
        return jsonResponse({
          vault_id: VAULT_ID,
          results: [],
          asset_results: [
            {
              target: "Some document.pdf",
              path: "98_Attachments/Some document.pdf",
            },
          ],
        });
      },
    );

    const { result } = renderHook(() =>
      useResolvedWikilinks(
        VAULT_ID,
        "![[Some document.pdf]]",
        "97_Notes/Some note.md",
      ),
    );

    await waitFor(() => {
      expect(result.current.resolved).toContain("98_Attachments");
    });

    expect(bodies[0]).toEqual({
      targets: [],
      asset_targets: ["Some document.pdf"],
      note_path: "97_Notes/Some note.md",
    });
    expect(result.current.resolved).toBe(
      "![Some document\\.pdf](/api/v1/vaults/vault-1/assets/98_Attachments/Some%20document.pdf)",
    );
  });

  // The live editor draws embeds from the same answers (#544): before the
  // batch lands an embed reads relative to its note, after it the server's
  // path wins, as it does in the rendered body.
  it("hands the editor the server's path once the batch has landed", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
      jsonResponse({
        vault_id: VAULT_ID,
        results: [],
        asset_results: [
          { target: "report.pdf", path: "98_Attachments/report.pdf" },
        ],
      }),
    );
    expect(
      cachedAssetHref(VAULT_ID, "report.pdf#page=2", "97_Notes/A.md"),
    ).toBe("/api/v1/vaults/vault-1/assets/97_Notes/report.pdf#page=2");
    const { result } = renderHook(() =>
      useResolvedWikilinks(VAULT_ID, "![[report.pdf#page=2]]", "97_Notes/A.md"),
    );
    await waitFor(() => {
      expect(result.current.resolvedFor).toBe("![[report.pdf#page=2]]");
    });
    expect(
      cachedAssetHref(VAULT_ID, "report.pdf#page=2", "97_Notes/A.md"),
    ).toBe("/api/v1/vaults/vault-1/assets/98_Attachments/report.pdf#page=2");
  });

  it("does not reuse one note's asset resolution in a note at another depth", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        const body = JSON.parse(String(init?.body)) as { note_path: string };
        return jsonResponse({
          vault_id: VAULT_ID,
          results: [],
          asset_results: [
            {
              target: "shot.png",
              path: body.note_path.startsWith("97_Notes")
                ? "97_Notes/shot.png"
                : "98_Attachments/shot.png",
            },
          ],
        });
      },
    );

    const first = renderHook(() =>
      useResolvedWikilinks(VAULT_ID, "![[shot.png]]", "97_Notes/A.md"),
    );
    await waitFor(() => {
      expect(first.result.current.resolved).toContain("97_Notes/shot.png");
    });

    const second = renderHook(() =>
      useResolvedWikilinks(VAULT_ID, "![[shot.png]]", "02_Spaces/B.md"),
    );
    await waitFor(() => {
      expect(second.result.current.resolved).toContain(
        "98_Attachments/shot.png",
      );
    });
  });
});

describe("resolveNoteTargets (#544)", () => {
  it("asks the server once for the targets it has not seen, in one request", async () => {
    const bodies: Array<Record<string, unknown>> = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        bodies.push(JSON.parse(String(init?.body)));
        return jsonResponse({
          vault_id: "vault-batch",
          results: [
            { target: "Alpha#Goals", slug: "alpha", archived: false },
            { target: "Nowhere", slug: null, archived: false },
          ],
          asset_results: [],
        });
      },
    );

    const first = await resolveNoteTargets("vault-batch", "Home.md", [
      "Alpha#Goals",
      "Nowhere",
    ]);
    expect(first.get("Alpha#Goals")).toEqual({
      slug: "alpha",
      archived: false,
    });
    expect(first.get("Nowhere")).toBeNull();
    expect(bodies).toEqual([
      {
        targets: ["Alpha#Goals", "Nowhere"],
        asset_targets: [],
        note_path: "Home.md",
      },
    ]);

    // Both answers are cached: asking again sends nothing.
    const second = await resolveNoteTargets("vault-batch", "Home.md", [
      "Nowhere",
      "Alpha#Goals",
    ]);
    expect(second.get("Alpha#Goals")).toEqual({
      slug: "alpha",
      archived: false,
    });
    expect(bodies).toHaveLength(1);
  });

  it("asks again for a target written in a different note (#626)", async () => {
    const bodies: Array<Record<string, unknown>> = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
        bodies.push(body);
        // Between two namesakes the server picks the one nearest the note.
        const slug = body.note_path === "a/Home.md" ? "plan" : "plan-2";
        return jsonResponse({
          vault_id: "vault-nearest",
          results: [{ target: "Plan", slug, archived: false }],
          asset_results: [],
        });
      },
    );

    const fromA = await resolveNoteTargets("vault-nearest", "a/Home.md", [
      "Plan",
    ]);
    const fromB = await resolveNoteTargets("vault-nearest", "b/Home.md", [
      "Plan",
    ]);

    expect(fromA.get("Plan")).toEqual({ slug: "plan", archived: false });
    expect(fromB.get("Plan")).toEqual({ slug: "plan-2", archived: false });
    expect(bodies).toHaveLength(2);
  });

  it("leaves a target unresolved when the server cannot be reached", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new Error("offline"));
    const result = await resolveNoteTargets("vault-offline", "Home.md", [
      "Beta",
    ]);
    expect(result.get("Beta")).toBeNull();
  });
});

describe("useResolvedWikilinks Markdown note links (ADR-28)", () => {
  it("sends note-link targets with the note's path and routes the answer", async () => {
    const bodies: Array<Record<string, unknown>> = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        bodies.push(JSON.parse(String(init?.body)));
        return jsonResponse({
          vault_id: VAULT_ID,
          results: [],
          asset_results: [],
          note_link_results: [
            {
              target: "../20-projects/Beacon%20Launch.md",
              slug: "beacon-launch",
              archived: false,
            },
          ],
        });
      },
    );

    const { result } = renderHook(() =>
      useResolvedWikilinks(
        VAULT_ID,
        "[x](../20-projects/Beacon%20Launch.md#Goals)",
        "00-inbox/Home",
      ),
    );

    await waitFor(() => {
      expect(result.current.resolved).toBe(
        "[x](/v/vault-1/n/beacon-launch#goals)",
      );
    });
    expect(bodies[0]).toMatchObject({
      note_link_targets: ["../20-projects/Beacon%20Launch.md"],
      note_path: "00-inbox/Home",
    });
  });
});

// A link that names only a place in its own note has no note name for the
// server to look up (#727). It goes out as the note's own path from the Vault
// root, which nothing but that note answers to, and its answer comes back
// under the target as written.
describe("links into their own note (#727)", () => {
  function answerByPath(vaultId: string, bodies: unknown[]) {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        const body = JSON.parse(String(init?.body)) as { targets: string[] };
        bodies.push(body);
        return jsonResponse({
          vault_id: vaultId,
          results: body.targets.map((target) => ({
            target,
            slug: target.startsWith("/Notes/Here.md#") ? "notes/here" : null,
            archived: false,
          })),
          asset_results: [],
        });
      },
    );
  }

  it("rewrites a heading and a block link to the note's own route", async () => {
    const bodies: unknown[] = [];
    answerByPath("vault-self", bodies);

    const { result } = renderHook(() =>
      useResolvedWikilinks(
        "vault-self",
        "[[#Second part]] and [[#^here|down]] and [[#Gone]]",
        "Notes/Here.md",
      ),
    );

    await waitFor(() => {
      expect(result.current.resolved).toContain("/n/notes/here");
    });
    expect(bodies[0]).toEqual({
      targets: [
        "/Notes/Here.md#Second part",
        "/Notes/Here.md#^here",
        "/Notes/Here.md#Gone",
      ],
      asset_targets: [],
      note_path: "Notes/Here.md",
    });
    expect(result.current.resolved).toBe(
      "[Second part](/v/vault-self/n/notes/here#second-part) and " +
        "[down](/v/vault-self/n/notes/here#^here) and " +
        "[Gone](/v/vault-self/n/notes/here#gone)",
    );
  });

  it("shows a heading whole and leaves embeds and bare carets as they were", async () => {
    const bodies: unknown[] = [];
    answerByPath("vault-self-edges", bodies);

    const { result } = renderHook(() =>
      useResolvedWikilinks(
        "vault-self-edges",
        "[[#Input/Output]]\n![[#Second part]]\n[[^here]]",
        "Notes/Here.md",
      ),
    );

    await waitFor(() => {
      expect(result.current.resolved).toContain("/n/notes/here");
    });
    expect(bodies[0]).toMatchObject({
      targets: [
        "/Notes/Here.md#Input/Output",
        "/Notes/Here.md#Second part",
        "^here",
      ],
    });
    const [link, embed, caret] = result.current.resolved.split("\n");
    expect(link).toBe(
      "[Input/Output](/v/vault-self-edges/n/notes/here#inputoutput)",
    );
    expect(embed).not.toContain("/n/notes/here");
    expect(embed).not.toContain("__embed__");
    expect(caret).toContain("/__missing__/");
  });

  it("asks as written from a note whose path the server would cut short", async () => {
    const bodies: unknown[] = [];
    answerByPath("vault-self-hash", bodies);

    renderHook(() =>
      useResolvedWikilinks("vault-self-hash", "[[#Setup]]", "C# basics.md"),
    );

    await waitFor(() => {
      expect(bodies).toHaveLength(1);
    });
    expect(bodies[0]).toMatchObject({ targets: ["#Setup"] });
  });

  it("answers the editor under the target as written", async () => {
    const bodies: unknown[] = [];
    answerByPath("vault-self-editor", bodies);

    const answers = await resolveNoteTargets(
      "vault-self-editor",
      "Notes/Here.md",
      ["#Second part", "#^here"],
    );

    expect(answers.get("#Second part")).toEqual({
      slug: "notes/here",
      archived: false,
    });
    expect(answers.get("#^here")).toEqual({
      slug: "notes/here",
      archived: false,
    });
    expect(bodies).toEqual([
      {
        targets: ["/Notes/Here.md#Second part", "/Notes/Here.md#^here"],
        asset_targets: [],
        note_path: "Notes/Here.md",
      },
    ]);
  });
});

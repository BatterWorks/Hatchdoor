import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useResolvedWikilinks } from "./wikilinks";

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

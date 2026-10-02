import { describe, expect, it } from "vitest";

import { flattenNoteCandidates } from "./noteCandidates";

describe("flattenNoteCandidates", () => {
  it("returns an empty list for a null tree", () => {
    expect(flattenNoteCandidates(null)).toEqual([]);
  });

  it("flattens, de-duplicates by slug, and sorts by title", () => {
    const tree = {
      name: "Vault",
      notes: [{ title: "Zeta", slug: "zeta", vault_id: "vault-1" }],
      folders: [
        {
          name: "Projects",
          notes: [
            { title: "Alpha", slug: "alpha", vault_id: "vault-1" },
            { title: "Zeta", slug: "zeta", vault_id: "vault-1" },
          ],
          folders: [
            {
              name: "Sub",
              notes: [{ title: "Mid", slug: "mid", vault_id: "vault-1" }],
              folders: [],
            },
          ],
        },
      ],
    };

    expect(flattenNoteCandidates(tree).map((n) => n.slug)).toEqual([
      "alpha",
      "mid",
      "zeta",
    ]);
  });

  it("records each note's Vault-relative path for a Markdown link", () => {
    const tree = {
      name: "My Vault",
      notes: [{ title: "Home", slug: "home", vault_id: "vault-1" }],
      folders: [
        {
          name: "Projects",
          notes: [],
          folders: [
            {
              name: "Sub dir",
              notes: [{ title: "Plan", slug: "plan", vault_id: "vault-1" }],
              folders: [],
            },
          ],
        },
      ],
    };

    expect(
      flattenNoteCandidates(tree).map((note) => note.relativePath),
    ).toEqual(["Home.md", "Projects/Sub dir/Plan.md"]);
  });

  it("keeps same-slug notes from different Vaults apart", () => {
    const tree = {
      name: "Vaults",
      notes: [
        { title: "Plan", slug: "plan", vault_id: "vault-1" },
        { title: "Plan", slug: "plan", vault_id: "vault-2" },
      ],
      folders: [],
    };

    expect(flattenNoteCandidates(tree).map((note) => note.vault_id)).toEqual([
      "vault-1",
      "vault-2",
    ]);
  });
});

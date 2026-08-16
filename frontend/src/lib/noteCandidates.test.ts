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
});

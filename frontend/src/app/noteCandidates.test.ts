import { describe, expect, it } from "vitest";

import { flattenNoteCandidates } from "./noteCandidates";

describe("flattenNoteCandidates", () => {
  it("returns an empty list for a null tree", () => {
    expect(flattenNoteCandidates(null)).toEqual([]);
  });

  it("flattens, de-duplicates by slug, and sorts by title", () => {
    const tree = {
      name: "Vault",
      notes: [{ title: "Zeta", slug: "zeta" }],
      folders: [
        {
          name: "Projects",
          notes: [
            { title: "Alpha", slug: "alpha" },
            { title: "Zeta", slug: "zeta" },
          ],
          folders: [
            {
              name: "Sub",
              notes: [{ title: "Mid", slug: "mid" }],
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

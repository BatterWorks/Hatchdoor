import { describe, expect, it } from "vitest";

import { collectFolderPaths } from "./folderPaths";

describe("collectFolderPaths", () => {
  it("returns an empty list for a null tree", () => {
    expect(collectFolderPaths(null)).toEqual([]);
  });

  it("flattens nested folders into sorted paths", () => {
    const tree = {
      name: "Vault",
      notes: [],
      folders: [
        {
          name: "Projects",
          notes: [],
          folders: [{ name: "2026", notes: [], folders: [] }],
        },
        { name: "Archive", notes: [], folders: [] },
      ],
    };

    expect(collectFolderPaths(tree)).toEqual([
      "Archive",
      "Projects",
      "Projects/2026",
    ]);
  });
});

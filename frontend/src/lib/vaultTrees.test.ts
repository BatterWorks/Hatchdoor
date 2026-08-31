import { describe, expect, it } from "vitest";

import type { VaultId, WireVaultTree } from "../types";
import { attributeVaultTree } from "./vaultTrees";

const VAULT_ID = "01JAAAAAAAAAAAAAAAAAAAAAAA" as VaultId;

const WIRE: WireVaultTree = {
  vault_id: VAULT_ID,
  vault_name: "Reference",
  tree: {
    name: "Vault",
    note_count: 1,
    folders: [
      {
        name: "40-reference",
        note_count: 0,
        truncated: true,
        folders: [
          {
            name: "Parenting",
            note_count: 1,
            folders: [],
            notes: [{ title: "Kid", slug: "kid" }],
          },
        ],
        notes: [],
      },
    ],
    notes: [{ title: "Home", slug: "home" }],
  },
};

describe("attributeVaultTree", () => {
  it("stamps the tree's Vault onto every note, at every depth", () => {
    const tree = attributeVaultTree(WIRE).tree;

    expect(tree.notes).toEqual([
      { vault_id: VAULT_ID, title: "Home", slug: "home" },
    ]);
    expect(tree.folders[0].folders[0].notes).toEqual([
      { vault_id: VAULT_ID, title: "Kid", slug: "kid" },
    ]);
  });

  it("keeps the Vault's own identity on the tree it came from", () => {
    const attributed = attributeVaultTree(WIRE);

    expect(attributed.vault_id).toBe(VAULT_ID);
    expect(attributed.vault_name).toBe("Reference");
    expect(attributed.tree.name).toBe("Vault");
    expect(attributed.tree.folders[0].name).toBe("40-reference");
  });
});

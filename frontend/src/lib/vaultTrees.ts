import type {
  ExplorerFolder,
  VaultId,
  VaultTree,
  WireExplorerFolder,
  WireVaultTree,
} from "../types";

/**
 * Stamp every note in one Vault's tree with that Vault's ID.
 *
 * The tree arrives grouped per Vault and states its `vault_id` once, rather
 * than repeating it on every note (#192). That grouping is the first thing the
 * app throws away: trees from several Vaults are merged into one root for the
 * narrowed explorer, and flattened again into a single candidate list for
 * wikilink autocomplete. Attaching the ID here, where the group is still
 * intact, is what keeps a note link pointing at the Vault the note actually
 * came from.
 */
export function attributeVaultTree(wire: WireVaultTree): VaultTree {
  return {
    vault_id: wire.vault_id,
    vault_name: wire.vault_name,
    tree: attributeFolder(wire.tree, wire.vault_id),
  };
}

function attributeFolder(
  folder: WireExplorerFolder,
  vaultId: VaultId,
): ExplorerFolder {
  return {
    name: folder.name,
    folders: folder.folders.map((child) => attributeFolder(child, vaultId)),
    notes: folder.notes.map((note) => ({
      vault_id: vaultId,
      title: note.title,
      slug: note.slug,
    })),
  };
}

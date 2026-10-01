import type { ExplorerFolder, ExplorerNote } from "../types";

/** An autocomplete candidate, with where it lives for a Markdown link. */
export type NoteCandidate = ExplorerNote & {
  /** Vault-relative, `.md` included. A note's title is its file name. */
  relativePath: string;
};

/**
 * Flatten the explorer tree into a title-sorted list of notes, each listed
 * once per Vault, used as the candidate pool for wikilink autocomplete.
 */
export function flattenNoteCandidates(
  root: ExplorerFolder | null,
): NoteCandidate[] {
  if (!root) {
    return [];
  }

  // The root folder is the Vault itself, or with several Vaults a synthetic
  // folder whose children are each Vault's top level, so paths start below it.
  // Keyed by Vault as well: a slug is only unique within its own Vault, and a
  // Markdown link's shortest path needs every note of that Vault.
  const bySlug = new Map<string, NoteCandidate>();
  const visit = (folder: ExplorerFolder, prefix: string) => {
    for (const note of folder.notes) {
      const key = `${note.vault_id}:${note.slug}`;
      if (!bySlug.has(key)) {
        bySlug.set(key, {
          ...note,
          relativePath: `${prefix}${note.title}.md`,
        });
      }
    }
    for (const child of folder.folders) {
      visit(child, `${prefix}${child.name}/`);
    }
  };

  visit(root, "");
  return Array.from(bySlug.values()).sort((a, b) =>
    a.title.localeCompare(b.title),
  );
}

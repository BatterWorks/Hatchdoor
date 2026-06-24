import type { ExplorerFolder, ExplorerNote } from "../types";

/**
 * Flatten the explorer tree into a de-duplicated, title-sorted list of notes,
 * used as the candidate pool for wikilink autocomplete.
 */
export function flattenNoteCandidates(
  root: ExplorerFolder | null,
): ExplorerNote[] {
  if (!root) {
    return [];
  }

  const bySlug = new Map<string, ExplorerNote>();
  const visit = (folder: ExplorerFolder) => {
    for (const note of folder.notes) {
      if (!bySlug.has(note.slug)) {
        bySlug.set(note.slug, note);
      }
    }
    for (const child of folder.folders) {
      visit(child);
    }
  };

  visit(root);
  return Array.from(bySlug.values()).sort((a, b) =>
    a.title.localeCompare(b.title),
  );
}

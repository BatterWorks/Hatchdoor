import type { ExplorerFolder } from "../types";

/**
 * Flatten the explorer tree into a sorted list of folder paths
 * (e.g. "Projects", "Projects/2026"). Used to power folder autocomplete in the
 * create/move dialogs. The root itself is not included.
 */
export function collectFolderPaths(root: ExplorerFolder | null): string[] {
  if (!root) {
    return [];
  }

  const paths: string[] = [];
  const visit = (folder: ExplorerFolder, prefix: string) => {
    for (const child of folder.folders) {
      const path = prefix ? `${prefix}/${child.name}` : child.name;
      paths.push(path);
      visit(child, path);
    }
  };

  visit(root, "");
  return paths.sort((a, b) => a.localeCompare(b));
}

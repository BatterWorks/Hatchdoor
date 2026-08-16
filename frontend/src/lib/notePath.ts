export type NoteIdentity = { vaultId: string; slug: string };

/** Parses a note route (`/v/:vaultId/n/:slug`) into its Vault and slug.
 * Shared by Explorer.tsx's active-path folder highlighting and the explorer
 * accordion's landing default (#142), which needs this synchronously off the
 * URL rather than waiting on `activeNote`'s own content fetch to resolve. */
export function pathToNoteIdentity(pathname: string): NoteIdentity | null {
  const match = pathname.match(/^\/v\/([^/]+)\/n\/([^/]+)$/);
  if (!match) {
    return null;
  }

  return {
    vaultId: decodeURIComponent(match[1]),
    slug: decodeURIComponent(match[2]),
  };
}

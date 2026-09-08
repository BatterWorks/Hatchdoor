export type NoteIdentity = { vaultId: string; slug: string };

const NOTE_ROUTE = /^\/v\/([^/]+)\/n\/([^/]+)$/;

/** Parses a note route (`/v/:vaultId/n/:slug`) into its Vault and slug.
 * Shared by Explorer.tsx's active-path folder highlighting and the explorer
 * accordion's landing default (#142), which needs this synchronously off the
 * URL rather than waiting on `activeNote`'s own content fetch to resolve. */
export function pathToNoteIdentity(pathname: string): NoteIdentity | null {
  const match = pathname.match(NOTE_ROUTE);
  if (!match) {
    return null;
  }

  return {
    vaultId: decodeURIComponent(match[1]),
    slug: decodeURIComponent(match[2]),
  };
}

/** Whether a pathname is a note route, without decoding it. The note body is
 * the one place this is asked of a string an author wrote rather than one the
 * browser produced, so a malformed escape has to answer "not a route" instead
 * of throwing out of a render. */
export function isNoteRoutePath(pathname: string): boolean {
  return NOTE_ROUTE.test(pathname);
}

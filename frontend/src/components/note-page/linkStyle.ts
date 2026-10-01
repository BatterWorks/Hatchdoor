// The text Hatchdoor inserts for a new note link or attachment embed, in the
// Vault's own link style (ADR-33). The style itself is read by the server and
// arrives on the Vault's discovery entry; nothing here fetches it.

import type { LinkPathForm, LinkStyle, VaultSummary } from "../../types";

export type VaultLinkStyle = { style: LinkStyle; pathForm: LinkPathForm };

/** What a Vault that reports no style gets: exactly what Hatchdoor always
 * inserted. */
export const WIKILINK_STYLE: VaultLinkStyle = {
  style: "wikilink",
  pathForm: "shortest",
};

export function linkStyleOf(vault: VaultSummary | undefined): VaultLinkStyle {
  if (vault?.link_style !== "markdown") {
    return WIKILINK_STYLE;
  }
  return { style: "markdown", pathForm: vault.link_path_form ?? "relative" };
}

/**
 * A path made safe for a Markdown link destination, encoding only what would
 * break the link: whitespace and control characters, `%`, `#`, brackets and
 * parentheses. Mirrors `encode_link_path` in `src/vault/markdown_links.rs`,
 * which the rename rewriter writes with, so both write the same bytes.
 */
export function encodeLinkPath(path: string): string {
  let out = "";
  for (const char of path) {
    if (/[\s\p{Cc}%#[\]()]/u.test(char)) {
      for (const byte of new TextEncoder().encode(char)) {
        out += `%${byte.toString(16).toUpperCase().padStart(2, "0")}`;
      }
    } else {
      out += char;
    }
  }
  return out;
}

/** Link text that cannot end the link early or open a code span. */
export function escapeLinkText(text: string): string {
  return text.replace(/[\\[\]`]/g, "\\$&");
}

function folderOf(path: string): string[] {
  const parts = path.split("/").filter((part) => part.length > 0);
  parts.pop();
  return parts;
}

/** `target` (Vault-relative) as seen from the folder of the note at
 * `fromNotePath`. */
export function relativeLinkPath(fromNotePath: string, target: string): string {
  const from = folderOf(fromNotePath);
  const to = target.split("/").filter((part) => part.length > 0);
  let shared = 0;
  while (
    shared < from.length &&
    shared < to.length - 1 &&
    from[shared] === to[shared]
  ) {
    shared += 1;
  }
  return [
    ...Array<string>(from.length - shared).fill(".."),
    ...to.slice(shared),
  ].join("/");
}

/**
 * The paths `shortest` may write for `target`, preferred first: the bare file
 * name, the path from the Vault root, and the root-anchored path, which always
 * resolves. The caller keeps the first one that resolves back to `target`.
 */
export function shortestPathCandidates(target: string): string[] {
  const name = target.split("/").pop() ?? target;
  return [...new Set([name, target, `/${target}`])];
}

/**
 * The path a Markdown link from the note at `fromNotePath` writes for
 * `target`, both Vault-relative. `resolvesTo` answers whether a candidate
 * resolves to `target` from that note, and is asked only for `shortest`.
 */
export function markdownLinkPath(
  pathForm: LinkPathForm,
  fromNotePath: string,
  target: string,
  resolvesTo: (candidate: string) => boolean,
): string {
  if (pathForm === "absolute") {
    return `/${target}`;
  }
  if (pathForm === "relative") {
    return relativeLinkPath(fromNotePath, target);
  }
  const candidates = shortestPathCandidates(target);
  return candidates.find(resolvesTo) ?? `/${target}`;
}

/**
 * Whether `candidate` names the note at `target` from a note in
 * `fromNotePath`'s folder, by the path ladder Markdown note links resolve
 * through (ADR-28): note-relative, then from the Vault root, then a bare name
 * anywhere. Note paths compare case-insensitively, as the server's do.
 */
export function noteCandidateResolves(
  candidate: string,
  target: string,
  fromNotePath: string,
  vaultNotePaths: string[],
): boolean {
  const key = (path: string) => path.toLowerCase();
  const wanted = key(target);
  if (candidate.startsWith("/")) {
    return key(candidate.slice(1)) === wanted;
  }
  const notes = new Set(vaultNotePaths.map(key));
  const besideNote = [...folderOf(fromNotePath), candidate].join("/");
  if (notes.has(key(besideNote))) {
    return key(besideNote) === wanted;
  }
  if (notes.has(key(candidate))) {
    return key(candidate) === wanted;
  }
  if (candidate.includes("/")) {
    return false;
  }
  const named = vaultNotePaths.filter(
    (path) => key(path.split("/").pop() ?? path) === key(candidate),
  );
  return named.length === 1 && key(named[0]) === wanted;
}

/**
 * The text autocomplete inserts for the note titled `title` at `targetPath`
 * (Vault-relative, `.md` included), linked from the note at `fromNotePath`.
 * `vaultNotePaths` is every note in the Vault, for the `shortest` path form.
 */
export function noteLinkText(
  linkStyle: VaultLinkStyle,
  title: string,
  targetPath: string,
  fromNotePath: string,
  vaultNotePaths: string[],
): string {
  if (linkStyle.style === "wikilink") {
    return `[[${title}]]`;
  }
  const path = markdownLinkPath(
    linkStyle.pathForm,
    fromNotePath,
    targetPath,
    (candidate) =>
      noteCandidateResolves(
        candidate,
        targetPath,
        fromNotePath,
        vaultNotePaths,
      ),
  );
  return `[${escapeLinkText(title)}](${encodeLinkPath(path)})`;
}

import { useEffect, useState } from "react";

import { escapeMarkdownLabel, parseWikilinkTarget } from "../../lib/markdown";
import { slugifyHeading } from "../../lib/noteHeadings";
import { apiFetch, withAccessToken } from "../../api/api";
import type { VaultId, VaultResolveBatchResponse } from "../../types";
import { decodePercent, findMarkdownNoteLinks } from "./markdownLinks";

export type ResolvedWikilink = { slug: string; archived: boolean };

// The character class must exclude newlines. Without that, an unclosed [[
// matches forward to the next ]] anywhere in the note and the replacement
// collapses every line between them, so the rendered body has fewer lines than
// the source and every block below it is misaddressed. A dangling [[ is
// exactly what the wikilink autocomplete leaves behind mid-typing. Obsidian
// does not support multi-line wikilinks either.
const WIKILINK_PATTERN = /(!?)\[\[([^\]\r\n]+)\]\]/g;

// Resolution results are stable for the life of the page, and every content
// change re-runs this effect, so without a cache each keystroke-driven commit
// re-POSTs every target in the note and widens the settling window. Keyed by
// `vaultId:target` — wikilinks are always resolved within their own Vault
// (cross-Vault resolution is ruled out by #62), and the same target string
// can legitimately resolve differently in different Vaults.
const resolveCache = new Map<string, ResolvedWikilink | null>();

// Assets resolve relative to the note that embeds them, so the same target in
// two notes at different depths can legitimately be two different files. The
// note's path is part of the key for exactly that reason.
const assetResolveCache = new Map<string, string | null>();

// Markdown note links (ADR-28) name a note by path from the linking note's
// folder, so like assets they are keyed by the note's path as well.
const noteLinkResolveCache = new Map<string, ResolvedWikilink | null>();

function cacheKey(vaultId: VaultId, target: string): string {
  return `${vaultId}:${target}`;
}

// For targets read relative to the note they are written in: assets and
// Markdown note links.
function pathCacheKey(
  vaultId: VaultId,
  noteRelativePath: string,
  target: string,
): string {
  return `${vaultId}:${noteRelativePath}:${target}`;
}

// The answers `cache` already holds for `targets`, and the targets it lacks.
function fromCache<T>(
  cache: Map<string, T | null>,
  targets: string[],
  keyOf: (target: string) => string,
): { found: Map<string, T | null>; missing: string[] } {
  const found = new Map<string, T | null>();
  const missing: string[] = [];
  for (const target of targets) {
    const key = keyOf(target);
    if (cache.has(key)) {
      found.set(target, cache.get(key) ?? null);
    } else {
      missing.push(target);
    }
  }
  return { found, missing };
}

// The href a resolved note link gets, wikilink or Markdown link alike, and
// the one a missing one gets. `anchor` is already a heading slug.
function noteRouteHref(
  vaultId: VaultId,
  hit: ResolvedWikilink,
  anchor: string,
): string {
  const prefix = hit.archived
    ? "/__archived__/"
    : `/v/${encodeURIComponent(vaultId)}/n/`;
  return `${prefix}${hit.slug}${anchor ? `#${anchor}` : ""}`;
}

function missingNoteHref(target: string): string {
  return `/__missing__/${encodeURIComponent(target)}`;
}

/**
 * Rewrite every wikilink in `markdown` to a markdown link or image, and every
 * Markdown note link to the same in-app route a wikilink gets.
 *
 * Line-count preserving by contract: the result always has exactly as many
 * lines as the input, which is what lets a rendered node's position be mapped
 * back to a line in the file.
 */
export function rewriteWikilinks(
  vaultId: VaultId,
  markdown: string,
  noteRelativePath: string,
  resolved: Map<string, ResolvedWikilink | null>,
  resolvedAssets: Map<string, string | null> = new Map(),
  resolvedNoteLinks: Map<string, ResolvedWikilink | null> = new Map(),
): string {
  return rewriteMarkdownNoteLinks(vaultId, markdown, resolvedNoteLinks).replace(
    WIKILINK_PATTERN,
    (_whole, bang: string, body: string) => {
      const parsed = parseWikilinkTarget(body);

      if (bang === "!") {
        const source = assetHref(
          vaultId,
          parsed.target,
          noteRelativePath,
          resolvedAssets,
        );
        return `![${escapeMarkdownLabel(parsed.label)}](${source})`;
      }

      if (isPdfAssetTarget(parsed.target)) {
        const source = assetHref(
          vaultId,
          parsed.target,
          noteRelativePath,
          resolvedAssets,
        );
        return `[${escapeMarkdownLabel(parsed.label)}](${source})`;
      }

      const hit = resolved.get(parsed.target) ?? null;
      if (hit) {
        const href = noteRouteHref(vaultId, hit, extractAnchor(parsed.target));
        const label = wikilinkDisplayLabel(body, parsed.label, hit.archived);
        return `[${escapeMarkdownLabel(label)}](${href})`;
      }
      return `[${escapeMarkdownLabel(parsed.label)}](${missingNoteHref(parsed.target)})`;
    },
  );
}

/**
 * Point each Markdown note link's destination at its note's route, or at the
 * `/__missing__/` form a missing wikilink gets, so the renderer needs no
 * branch of its own for them. Only the destination changes: the link text and
 * title stay as written, and so does the line count.
 */
function rewriteMarkdownNoteLinks(
  vaultId: VaultId,
  markdown: string,
  resolved: Map<string, ResolvedWikilink | null>,
): string {
  let out = "";
  let cursor = 0;
  for (const link of findMarkdownNoteLinks(markdown)) {
    const hit = resolved.get(link.rawPath) ?? null;
    const href = hit
      ? noteRouteHref(
          vaultId,
          hit,
          link.anchor ? slugifyHeading(decodePercent(link.anchor)) : "",
        )
      : missingNoteHref(decodePercent(link.rawPath));
    const start = link.angle ? link.start - 1 : link.start;
    const end = link.angle ? link.end + 1 : link.end;
    out += markdown.slice(cursor, start) + href;
    cursor = end;
  }
  return out + markdown.slice(cursor);
}

/**
 * Resolved markdown, plus the input it was resolved from.
 *
 * Resolution awaits a network round-trip, so between a content change and the
 * response the rendered tree describes the *previous* document. Every block
 * range read during that window is stale, and acting on one edits the wrong
 * lines. Callers compare `resolvedFor` against the current input to know when
 * it is safe to address blocks by line.
 */
export function useResolvedWikilinks(
  vaultId: VaultId,
  markdown: string,
  noteRelativePath: string,
): { resolved: string; resolvedFor: string } {
  const [state, setState] = useState({
    resolved: markdown,
    resolvedFor: markdown,
  });

  useEffect(() => {
    let cancelled = false;

    if (!markdown) {
      queueMicrotask(() => setState({ resolved: "", resolvedFor: "" }));
      return;
    }

    void (async () => {
      const matches = [...markdown.matchAll(WIKILINK_PATTERN)];
      const isAsset = (match: RegExpMatchArray) =>
        match[1] === "!" ||
        isPdfAssetTarget(parseWikilinkTarget(match[2]).target);
      const rawTargets = matches
        .filter((m) => !isAsset(m))
        .map((m) => parseWikilinkTarget(m[2]).target)
        .filter((target) => target.length > 0);
      const uniqueTargets = [...new Set(rawTargets)];
      // Assets are sent by path part: the server resolves a file, and a
      // `#page=3` suffix addresses the viewer rather than naming a different
      // one.
      const uniqueAssetTargets = [
        ...new Set(
          matches
            .filter(isAsset)
            .map((m) => splitPathSuffix(parseWikilinkTarget(m[2]).target)[0])
            .filter((target) => target.length > 0),
        ),
      ];

      const uniqueNoteLinkTargets = [
        ...new Set(findMarkdownNoteLinks(markdown).map((link) => link.rawPath)),
      ];

      const pathKey = (target: string) =>
        pathCacheKey(vaultId, noteRelativePath, target);
      const { found: map, missing } = fromCache(
        resolveCache,
        uniqueTargets,
        (target) => cacheKey(vaultId, target),
      );
      const { found: assetMap, missing: missingAssets } = fromCache(
        assetResolveCache,
        uniqueAssetTargets,
        pathKey,
      );
      const { found: noteLinkMap, missing: missingNoteLinks } = fromCache(
        noteLinkResolveCache,
        uniqueNoteLinkTargets,
        pathKey,
      );

      if (
        missing.length > 0 ||
        missingAssets.length > 0 ||
        missingNoteLinks.length > 0
      ) {
        try {
          const res = await apiFetch(
            `/api/v1/vaults/${encodeURIComponent(vaultId)}/resolve-batch`,
            {
              method: "POST",
              headers: {
                "Content-Type": "application/json",
              },
              body: JSON.stringify({
                targets: missing,
                asset_targets: missingAssets,
                // Sent only when there are some, so a note without Markdown
                // note links asks exactly what it asked before ADR-28.
                ...(missingNoteLinks.length > 0 && {
                  note_link_targets: missingNoteLinks,
                }),
                note_path: noteRelativePath,
              }),
            },
          );

          if (res.ok) {
            const json = (await res.json()) as VaultResolveBatchResponse;
            for (const result of json.results) {
              const resolved = result.slug
                ? { slug: result.slug, archived: result.archived }
                : null;
              map.set(result.target, resolved);
              resolveCache.set(cacheKey(vaultId, result.target), resolved);
            }
            for (const result of json.note_link_results ?? []) {
              const resolved = result.slug
                ? { slug: result.slug, archived: result.archived }
                : null;
              noteLinkMap.set(result.target, resolved);
              noteLinkResolveCache.set(pathKey(result.target), resolved);
            }
            for (const result of json.asset_results ?? []) {
              assetMap.set(result.target, result.path);
              assetResolveCache.set(pathKey(result.target), result.path);
            }
          }
        } catch {
          // Leave unresolved values as null fallback.
        }
      }

      const rewritten = rewriteWikilinks(
        vaultId,
        markdown,
        noteRelativePath,
        map,
        assetMap,
        noteLinkMap,
      );

      if (!cancelled) {
        setState({ resolved: rewritten, resolvedFor: markdown });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [vaultId, markdown, noteRelativePath]);

  return state;
}

export function resolveAssetHref(
  vaultId: VaultId,
  rawTarget: string,
  noteRelativePath: string,
): string {
  if (/^(https?:|data:|blob:)/i.test(rawTarget) || rawTarget.startsWith("/")) {
    return rawTarget;
  }

  const [pathPart, suffix] = splitPathSuffix(rawTarget);
  const noteDir = dirname(noteRelativePath);
  const normalized = normalizeRelativePath(noteDir, pathPart);

  if (!normalized) {
    return rawTarget;
  }

  const encoded = normalized.split("/").map(encodeURIComponent).join("/");
  return withAccessToken(
    `/api/v1/vaults/${encodeURIComponent(vaultId)}/assets/${encoded}${suffix}`,
  );
}

/**
 * The href for one wikilink asset target.
 *
 * Prefers the path the server resolved for it, which is what makes Obsidian's
 * bare-filename embeds work from a note that is not the attachment's sibling
 * (#158). Falls back to the note-relative reading when nothing resolved, so a
 * target the server has not answered for yet — the first render of a note,
 * before the batch returns — renders exactly as it did before.
 */
function assetHref(
  vaultId: VaultId,
  rawTarget: string,
  noteRelativePath: string,
  resolvedAssets: Map<string, string | null>,
): string {
  // Keyed by the path part, the way the target is sent for resolution: the
  // suffix rides along from the written target instead, because `#page=3` on a
  // PDF embed addresses the viewer rather than the file.
  const [pathPart, suffix] = splitPathSuffix(rawTarget);
  const resolvedPath = resolvedAssets.get(pathPart);
  if (!resolvedPath) {
    return resolveAssetHref(vaultId, rawTarget, noteRelativePath);
  }
  const encoded = resolvedPath.split("/").map(encodeURIComponent).join("/");
  return withAccessToken(
    `/api/v1/vaults/${encodeURIComponent(vaultId)}/assets/${encoded}${suffix}`,
  );
}

function isPdfAssetTarget(target: string): boolean {
  return splitPathSuffix(target)[0].toLowerCase().endsWith(".pdf");
}

function splitPathSuffix(input: string): [string, string] {
  const markerIndex = input.search(/[?#]/);
  if (markerIndex < 0) {
    return [input, ""];
  }
  return [input.slice(0, markerIndex), input.slice(markerIndex)];
}

function dirname(relativePath: string): string {
  const parts = relativePath.split("/").filter((part) => part.length > 0);
  parts.pop();
  return parts.join("/");
}

function extractAnchor(target: string): string {
  const hashIdx = target.indexOf("#");
  if (hashIdx >= 0) {
    return slugifyHeading(target.slice(hashIdx + 1));
  }
  const caretIdx = target.indexOf("^");
  if (caretIdx >= 0) {
    return target.slice(caretIdx + 1);
  }
  return "";
}

function wikilinkDisplayLabel(
  body: string,
  fallbackLabel: string,
  archived: boolean,
): string {
  if (body.includes("|") || archived) {
    return fallbackLabel;
  }
  const parts = fallbackLabel.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts[parts.length - 1] ?? fallbackLabel;
}

function normalizeRelativePath(baseDir: string, target: string): string {
  const targetParts = target.replace(/\\/g, "/").split("/");
  const initial = baseDir ? baseDir.split("/").filter(Boolean) : [];
  const stack = [...initial];

  for (const part of targetParts) {
    if (!part || part === ".") {
      continue;
    }
    if (part === "..") {
      if (stack.length > 0) {
        stack.pop();
      }
      continue;
    }
    stack.push(part);
  }

  return stack.join("/");
}

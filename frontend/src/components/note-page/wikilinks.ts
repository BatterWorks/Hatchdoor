import { useEffect, useState } from "react";

import { escapeMarkdownLabel, parseWikilinkTarget } from "../../lib/markdown";
import { slugifyHeading } from "../../lib/noteHeadings";
import { apiFetch, withAccessToken } from "../../api/api";
import type { ResolveBatchResponse } from "../../types";

export function useResolvedWikilinks(
  markdown: string,
  noteRelativePath: string,
): string {
  const [resolved, setResolved] = useState(markdown);

  useEffect(() => {
    let cancelled = false;

    if (!markdown) {
      queueMicrotask(() => setResolved(""));
      return;
    }

    void (async () => {
      const matches = [...markdown.matchAll(/(!?)\[\[([^\]]+)\]\]/g)];
      const rawTargets = matches
        .filter((m) => m[1] !== "!")
        .map((m) => parseWikilinkTarget(m[2]).target)
        .filter((target) => target.length > 0);
      const uniqueTargets = [...new Set(rawTargets)];

      const map = new Map<string, { slug: string; archived: boolean } | null>();

      if (uniqueTargets.length > 0) {
        try {
          const res = await apiFetch("/api/resolve-batch", {
            method: "POST",
            headers: {
              "Content-Type": "application/json",
            },
            body: JSON.stringify({ targets: uniqueTargets }),
          });

          if (res.ok) {
            const json = (await res.json()) as ResolveBatchResponse;
            for (const result of json.results) {
              map.set(
                result.target,
                result.slug
                  ? { slug: result.slug, archived: result.archived }
                  : null,
              );
            }
          }
        } catch {
          // Leave unresolved values as null fallback.
        }
      }

      const rewritten = markdown.replace(
        /(!?)\[\[([^\]]+)\]\]/g,
        (_whole, bang: string, body: string) => {
          const parsed = parseWikilinkTarget(body);

          if (bang === "!") {
            const source = resolveAssetHref(parsed.target, noteRelativePath);
            return `![${escapeMarkdownLabel(parsed.label)}](${source})`;
          }

          const resolved = map.get(parsed.target) ?? null;
          if (resolved) {
            const anchor = extractAnchor(parsed.target);
            const hash = anchor ? `#${anchor}` : "";
            const prefix = resolved.archived ? "/__archived__/" : "/n/";
            const label = wikilinkDisplayLabel(
              body,
              parsed.label,
              resolved.archived,
            );
            return `[${escapeMarkdownLabel(label)}](${prefix}${resolved.slug}${hash})`;
          }
          return `[${escapeMarkdownLabel(parsed.label)}](/__missing__/${encodeURIComponent(parsed.target)})`;
        },
      );

      if (!cancelled) {
        setResolved(rewritten);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [markdown, noteRelativePath]);

  return resolved;
}

export function resolveAssetHref(
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
  return withAccessToken(`/vault-assets/${encoded}${suffix}`);
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

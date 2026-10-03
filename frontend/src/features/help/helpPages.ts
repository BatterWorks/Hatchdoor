// The bundled manual as Help reads it: the public `/docs/` addresses (ADR-38).
// These requests never go through `apiFetch`. A 401 here only means a private
// page, and must not raise the token prompt the way a Vault request does.

import { getToken } from "../../api/api";

/** The manual's front page plus a list of every page the reader may open. */
export const HELP_HOME = "index";

/** What `/docs/deploy.md` serves; links to the short address land here. */
const DEPLOY_PAGE = "guides/how-to-deploy-hatchdoor-with-an-agent";

/** Pages Help can be opened at by name. */
export const HELP_PAGES = {
  install: "get-started/install-hatchdoor-with-docker-compose",
  deploy: DEPLOY_PAGE,
} as const;

export type HelpLocation = { page: string; heading?: string };

export type HelpPageResult =
  | { kind: "page"; markdown: string }
  | { kind: "missing" }
  | { kind: "private" }
  | { kind: "error" };

export type HelpSearchHit = { name: string; title: string; excerpt: string };

const BASE = "http://help.invalid";

/**
 * Where a link on `fromPage` goes inside Help, or `null` for a link that
 * leaves the manual. Page links arrive relative to the page's own address,
 * `/docs/<page>.md`, so they resolve the way a browser would resolve them.
 */
export function resolveHelpLink(
  href: string | undefined,
  fromPage: string,
): HelpLocation | null {
  if (!href) {
    return null;
  }
  let url: URL;
  try {
    url = new URL(href, `${BASE}/docs/${fromPage}.md`);
  } catch {
    return null;
  }
  if (url.origin !== BASE) {
    return null;
  }
  const path = decodeURIComponent(url.pathname);
  if (!path.startsWith("/docs/") || !path.endsWith(".md")) {
    return null;
  }
  const name = path.slice("/docs/".length, -".md".length);
  const page = name === "deploy" ? DEPLOY_PAGE : name;
  const heading = decodeURIComponent(url.hash.slice(1));
  return heading ? { page, heading } : { page };
}

/** The page's title: its first top-level heading outside code. */
export function helpPageTitle(markdown: string, fallback = ""): string {
  let fenced = false;
  for (const line of markdown.split(/\r?\n/)) {
    if (/^\s*(```|~~~)/.test(line)) {
      fenced = !fenced;
      continue;
    }
    const match = !fenced && /^#\s+(.+?)\s*#*\s*$/.exec(line);
    if (match) {
      return match[1];
    }
  }
  return fallback;
}

/** A search excerpt as plain words: search hits quote raw Markdown. */
export function plainExcerpt(excerpt: string): string {
  return excerpt
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/`|\*\*|__/g, "")
    .replace(/\|/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function docsRequest(signal?: AbortSignal): RequestInit {
  const token = getToken();
  return {
    signal,
    headers: token ? { Authorization: `Bearer ${token}` } : undefined,
  };
}

export async function fetchHelpPage(
  page: string,
  signal?: AbortSignal,
): Promise<HelpPageResult> {
  let response: Response;
  try {
    response = await fetch(`/docs/${page}.md`, docsRequest(signal));
  } catch (error) {
    if (signal?.aborted) {
      throw error;
    }
    return { kind: "error" };
  }
  if (response.ok) {
    return { kind: "page", markdown: await response.text() };
  }
  if (response.status === 404) {
    return { kind: "missing" };
  }
  if (response.status === 401) {
    return { kind: "private" };
  }
  return { kind: "error" };
}

/** The same word search as the MCP `search_docs` tool. Throws on failure. */
export async function searchHelp(
  query: string,
  signal?: AbortSignal,
): Promise<HelpSearchHit[]> {
  const params = new URLSearchParams({ q: query });
  const response = await fetch(`/docs/search?${params}`, docsRequest(signal));
  if (!response.ok) {
    throw new Error(`Help search failed with ${response.status}`);
  }
  const body = (await response.json()) as { results?: HelpSearchHit[] };
  return body.results ?? [];
}

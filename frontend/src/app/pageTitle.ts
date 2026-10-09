import { useEffect } from "react";
import { matchPath, useLocation } from "react-router-dom";

import type { ActiveNoteMeta } from "../types";

const SITE_NAME = "Hatchdoor";

/** The pages that carry a fixed name. The server writes the same wording into
 * the page a demo instance sends (`src/handlers/link_preview.rs`), so a tab
 * does not change its name when the app takes over. */
const NAMED_ROUTES: [path: string, name: string][] = [
  ["/graph", "Graph"],
  ["/stats", "Stats"],
  ["/settings", "Settings"],
];

export type TitledNote = Pick<ActiveNoteMeta, "vaultId" | "slug" | "title">;

function withSiteName(name: string): string {
  return `${name} · ${SITE_NAME}`;
}

/** `matchPath` hands back a param as the address spells it, percent-encoded,
 * where the note carries its slug decoded. */
function decoded(param: string | undefined): string | undefined {
  try {
    return param === undefined ? undefined : decodeURIComponent(param);
  } catch {
    return param;
  }
}

/** The name the topbar's breadcrumb shows for an address that is not a note
 * (#530): the named routes above, `Home` on `"/"`, and nothing on a note
 * route, where the crumb carries the note's own path instead. */
export function pageName(pathname: string): string | null {
  if (matchPath("/", pathname)) {
    return "Home";
  }
  for (const [path, name] of NAMED_ROUTES) {
    if (matchPath(path, pathname)) {
      return name;
    }
  }
  return null;
}

/** The browser tab title for an address. `activeNote` is the note the shell
 * last saw loaded, which outlives the note route and lags a move from one
 * note to the next, so it only counts while the address names that note. */
export function pageTitle(
  pathname: string,
  activeNote: TitledNote | null,
): string {
  if (matchPath("/", pathname)) {
    return SITE_NAME;
  }
  for (const [path, name] of NAMED_ROUTES) {
    if (matchPath(path, pathname)) {
      return withSiteName(name);
    }
  }
  const noteRoute = matchPath("/v/:vaultId/n/:slug", pathname);
  if (!noteRoute) {
    return withSiteName("Page not found");
  }
  const title =
    activeNote !== null &&
    activeNote.vaultId === decoded(noteRoute.params.vaultId) &&
    activeNote.slug === decoded(noteRoute.params.slug)
      ? activeNote.title.replace(/\s+/g, " ").trim()
      : "";
  return title ? withSiteName(title) : SITE_NAME;
}

/** Keeps the browser tab title on the page being viewed. */
export function usePageTitle(activeNote: TitledNote | null): void {
  const { pathname } = useLocation();
  const title = pageTitle(pathname, activeNote);
  useEffect(() => {
    document.title = title;
  }, [title]);
}

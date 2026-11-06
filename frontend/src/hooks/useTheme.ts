import { useEffect, useState } from "react";
import { THEME_KEY } from "../app/constants";
import { safeGetItem, safeRemoveItem, safeSetItem } from "../lib/storage";

export type Theme = "auto" | "light" | "dark";

function readStoredTheme(): Theme {
  const v = safeGetItem(THEME_KEY);
  return v === "light" || v === "dark" ? v : "auto";
}

const OVERRIDE_ATTR = "data-theme-override";

/** The browser bar takes the page background, `--bg`, read from the computed
 * style so it follows base.css rather than restating its hexes (#648). The
 * build's two tags, one per colour scheme, stay in place for auto. A chosen
 * theme puts one tag of its own in front of them: the browser uses the first
 * `theme-color` tag that matches, so it wins until auto takes it out. */
function syncThemeColor(theme: Theme) {
  document
    .querySelectorAll(`meta[name="theme-color"][${OVERRIDE_ATTR}]`)
    .forEach((node) => node.remove());
  if (theme === "auto") return;
  const bg = getComputedStyle(document.documentElement)
    .getPropertyValue("--bg")
    .trim();
  // No stylesheet loaded yet: the build's tags are better than an empty one.
  if (!bg) return;
  const meta = document.createElement("meta");
  meta.name = "theme-color";
  meta.content = bg;
  meta.setAttribute(OVERRIDE_ATTR, "");
  const first = document.querySelector('meta[name="theme-color"]');
  if (first) first.before(meta);
  else document.head.append(meta);
}

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(readStoredTheme);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    if (theme === "auto") {
      safeRemoveItem(THEME_KEY);
    } else {
      safeSetItem(THEME_KEY, theme);
    }
    syncThemeColor(theme);
  }, [theme]);

  const cycleTheme = () =>
    setTheme((t) => (t === "auto" ? "light" : t === "light" ? "dark" : "auto"));

  // Picked by name from the topbar's menu (#530); the cycle stays for the
  // keyboard and for callers that have no menu.
  return { theme, cycleTheme, setTheme };
}

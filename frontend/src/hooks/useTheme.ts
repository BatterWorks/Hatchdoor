import { useEffect, useState } from "react";
import { THEME_KEY } from "../app/constants";

export type Theme = "auto" | "light" | "dark";

const THEME_COLORS: Record<Exclude<Theme, "auto">, string> = {
  light: "#f4f1e8",
  dark: "#0c0c0a",
};

function readStoredTheme(): Theme {
  const v = localStorage.getItem(THEME_KEY);
  return v === "light" || v === "dark" ? v : "auto";
}

function clearThemeColorMeta() {
  document
    .querySelectorAll('meta[name="theme-color"]')
    .forEach((node) => node.remove());
}

function addThemeColorMeta(content: string, media?: string) {
  const meta = document.createElement("meta");
  meta.name = "theme-color";
  meta.content = content;
  if (media) {
    meta.setAttribute("media", media);
  }
  document.head.append(meta);
}

function syncThemeColor(theme: Theme) {
  clearThemeColorMeta();
  if (theme === "auto") {
    addThemeColorMeta(THEME_COLORS.light, "(prefers-color-scheme: light)");
    addThemeColorMeta(THEME_COLORS.dark, "(prefers-color-scheme: dark)");
  } else {
    addThemeColorMeta(THEME_COLORS[theme]);
  }
}

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(readStoredTheme);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    if (theme === "auto") {
      localStorage.removeItem(THEME_KEY);
    } else {
      localStorage.setItem(THEME_KEY, theme);
    }
    syncThemeColor(theme);
  }, [theme]);

  const cycleTheme = () =>
    setTheme((t) => (t === "auto" ? "light" : t === "light" ? "dark" : "auto"));

  return { theme, cycleTheme };
}

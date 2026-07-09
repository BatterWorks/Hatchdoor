import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { useTheme } from "./useTheme";

function ThemeHarness() {
  const { theme, cycleTheme } = useTheme();
  return (
    <button type="button" onClick={cycleTheme}>
      {theme}
    </button>
  );
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  document.documentElement.dataset.theme = "auto";
  document.head
    .querySelectorAll('meta[name="theme-color"]')
    .forEach((node) => node.remove());
});

describe("useTheme", () => {
  it("updates browser chrome theme-color for manual theme overrides", async () => {
    document.head.insertAdjacentHTML(
      "beforeend",
      '<meta name="theme-color" content="#f4f1e8" media="(prefers-color-scheme: light)" />',
    );
    document.head.insertAdjacentHTML(
      "beforeend",
      '<meta name="theme-color" content="#0c0c0a" media="(prefers-color-scheme: dark)" />',
    );

    render(<ThemeHarness />);

    const toggle = screen.getByRole("button");
    expect(toggle).toHaveTextContent("auto");

    fireEvent.click(toggle);
    expect(toggle).toHaveTextContent("light");
    await waitFor(() =>
      expect(
        document.querySelector('meta[name="theme-color"]'),
      ).toHaveAttribute("content", "#f4f1e8"),
    );
    expect(
      document.querySelector('meta[name="theme-color"]'),
    ).not.toHaveAttribute("media");

    fireEvent.click(toggle);
    expect(toggle).toHaveTextContent("dark");
    await waitFor(() =>
      expect(
        document.querySelector('meta[name="theme-color"]'),
      ).toHaveAttribute("content", "#0c0c0a"),
    );
    expect(
      document.querySelector('meta[name="theme-color"]'),
    ).not.toHaveAttribute("media");

    fireEvent.click(toggle);
    expect(toggle).toHaveTextContent("auto");
    await waitFor(() => {
      expect(
        document.querySelector(
          'meta[name="theme-color"][media="(prefers-color-scheme: light)"]',
        ),
      ).toHaveAttribute("content", "#f4f1e8");
      expect(
        document.querySelector(
          'meta[name="theme-color"][media="(prefers-color-scheme: dark)"]',
        ),
      ).toHaveAttribute("content", "#0c0c0a");
    });
  });
});

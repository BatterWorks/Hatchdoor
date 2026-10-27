import { render } from "@testing-library/react";
import ReactMarkdown from "react-markdown";
import { describe, expect, it } from "vitest";

import { NOTE_SYNTAX_PLUGINS, stripHighlights } from "./markdownSyntax";

function marksIn(markdown: string): string[] {
  const { container } = render(
    <ReactMarkdown remarkPlugins={NOTE_SYNTAX_PLUGINS}>
      {markdown}
    </ReactMarkdown>,
  );
  return Array.from(container.querySelectorAll("mark.note-highlight")).map(
    (mark) => mark.textContent ?? "",
  );
}

describe("==highlight== syntax (#572)", () => {
  it.each([
    ["one word", "a ==b== c", ["b"]],
    ["two on a line", "==a== and ==b==", ["a", "b"]],
    ["next to punctuation", "(==a==).", ["a"]],
    ["around a single sign", "==a = b==", ["a = b"]],
    ["inside a heading", "## Title ==hot==", ["hot"]],
    ["inside a table cell", "| a |\n| - |\n| ==b== |", ["b"]],
    ["inside emphasis", "**bold ==mark== bold**", ["mark"]],
    ["across two lines of a paragraph", "==a\nb==", ["a\nb"]],
  ])("draws %s", (_, markdown, expected) => {
    expect(marksIn(markdown)).toEqual(expected);
  });

  it.each([
    ["a comparison in prose", "if finished == null then"],
    ["comparisons on both sides", "a == b and c == d"],
    ["a run of three", "===a==="],
    ["a run of four", "====a===="],
    ["a pair across two paragraphs", "==a\n\nb=="],
    ["escaped signs", "\\==a=="],
    ["a pair after an escaped sign", "\\===a=="],
    ["an empty pair", "===="],
    ["a space inside the opener", "== a=="],
    ["signs in inline math", "$==a==$ and $a == b$"],
    ["signs in block math", "$$\n==a==\n$$"],
  ])("leaves %s as written", (_, markdown) => {
    expect(marksIn(markdown)).toEqual([]);
  });
});

describe("stripHighlights", () => {
  it("drops a highlight's signs and keeps comparisons", () => {
    expect(stripHighlights("The ==fleet sails== on, finished == null.")).toBe(
      "The fleet sails on, finished == null.",
    );
    expect(stripHighlights("===a=== ==b== \\==c==")).toBe("===a=== b \\==c==");
    expect(stripHighlights("Write `==this==` for ==this==.")).toBe(
      "Write `==this==` for this.",
    );
  });

  it("follows the page's rules where a pattern would not", () => {
    // Not a highlight on the page: the closer is not flanked.
    expect(stripHighlights("==(a)==b")).toBe("==(a)==b");
    // An escaped backslash, then a real highlight.
    expect(stripHighlights("\\\\==a==")).toBe("\\\\a");
    // Double-backtick code and a fence keep their signs.
    expect(stripHighlights("``x ==y== z`` ==w==")).toBe("``x ==y== z`` w");
    expect(stripHighlights("```\n==x==\n```\n==y==")).toBe(
      "```\n==x==\n```\ny",
    );
    // A stray backtick does not shield a highlight after it.
    expect(stripHighlights("it`s ==a== and")).toBe("it`s a and");
  });
});

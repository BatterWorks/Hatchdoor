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

function renderSyntax(markdown: string): HTMLElement {
  return render(
    <ReactMarkdown remarkPlugins={NOTE_SYNTAX_PLUGINS}>
      {markdown}
    </ReactMarkdown>,
  ).container;
}

/** Each anchored element as `tag#id`, in document order. */
function anchorsIn(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll("[id]")).map(
    (element) => `${element.tagName.toLowerCase()}#${element.id}`,
  );
}

describe("block ids (#578)", () => {
  it.each([
    ["a paragraph", "Some text. ^abc", ["p#^abc"]],
    ["a paragraph of several lines", "One\ntwo. ^abc", ["p#^abc"]],
    ["a list item", "- first\n- second ^abc\n- third", ["li#^abc"]],
    [
      "a list item with a nested list",
      "- parent ^abc\n  - child ^def",
      ["li#^abc", "li#^def"],
    ],
    ["a task", "- [ ] do it ^abc", ["li#^abc"]],
    ["a quote", "> Quoted. ^abc", ["blockquote#^abc"]],
    [
      "a quote with the id on its own line",
      "> Quoted.\n\n^abc",
      ["blockquote#^abc"],
    ],
    [
      "a table with the id on its own line",
      "| a |\n| - |\n| b |\n\n^abc",
      ["table#^abc"],
    ],
    ["an id after a link", "See [it](https://example.com) ^abc", ["p#^abc"]],
    ["an id with a hyphen", "Text ^a-1", ["p#^a-1"]],
  ])("anchors %s and hides the id", (_, markdown, expected) => {
    const container = renderSyntax(markdown);
    expect(anchorsIn(container)).toEqual(expected);
    expect(container.textContent).not.toContain("^");
  });

  it.each([
    ["a fenced code block", "```\nlet x = 1; ^abc\n```"],
    ["an indented code block", "    let x = 1; ^abc"],
    ["inline code", "Write `text ^abc`"],
    ["block math", "$$\nx ^abc\n$$"],
    ["a caret inside a word", "2^abc"],
    ["an id with nothing above it", "^abc"],
    ["a heading that is only a caret word", "Para\n\n## ^abc"],
    ["an id under a code block", "```\ncode\n```\n\n^abc"],
  ])("leaves %s as written", (_, markdown) => {
    const container = renderSyntax(markdown);
    expect(anchorsIn(container)).toEqual([]);
    expect(container.textContent).toContain("abc");
  });

  it("hides an id on a heading without replacing the heading's anchor", () => {
    const container = renderSyntax("## Title ^abc");
    expect(container.querySelector("h2")?.textContent).toBe("Title");
    expect(anchorsIn(container)).toEqual([]);
  });

  it("hides an id closing an earlier line of a paragraph", () => {
    const container = renderSyntax("One ^abc\ntwo");
    expect(container.querySelector("p")?.textContent).toBe("One\ntwo");
  });

  it("keeps every block on the lines it was written on", () => {
    // Line-addressed editing reads these positions (lib/sourceMap.ts).
    const lines: number[] = [];
    render(
      <ReactMarkdown
        remarkPlugins={NOTE_SYNTAX_PLUGINS}
        components={{
          p(props) {
            lines.push(props.node?.position?.start.line ?? -1);
            return <p>{props.children}</p>;
          },
        }}
      >
        {"One. ^abc\n\n> Quote.\n\n^def\n\nThree."}
      </ReactMarkdown>,
    );
    expect(lines).toEqual([1, 3, 7]);
  });
});

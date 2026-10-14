import { describe, expect, it } from "vitest";

import cases from "./noteComments.cases.json";
import { hideComments } from "./noteComments";

const trimLineEnds = (text: string) =>
  text
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n");

describe("hideComments", () => {
  // The same cases run against the link preview's `strip_comments` in
  // `src/handlers/link_preview.rs`, so the page and the preview card cannot
  // come to disagree about what a comment is.
  it.each(cases)(
    "agrees with the link preview on $name",
    ({ input, output }) => {
      expect(trimLineEnds(hideComments(input))).toBe(output);
    },
  );

  it("keeps the line count", () => {
    const input = "a %%one\ntwo\nthree%% b\n<!--\n\n-->\nc %%open";
    expect(hideComments(input).split("\n")).toHaveLength(
      input.split("\n").length,
    );
  });

  it("leaves comment markers in inline code and math as written", () => {
    expect(hideComments("`%%not a comment%%` and %%gone%%")).toBe(
      "`%%not a comment%%` and",
    );
    expect(hideComments("``a ` %%x%% ``")).toBe("``a ` %%x%% ``");
    expect(hideComments("$a %% b %% c$")).toBe("$a %% b %% c$");
    expect(hideComments("`<!-- x -->`")).toBe("`<!-- x -->`");
  });

  it("leaves fenced code blocks and math blocks as written", () => {
    const input = [
      "```",
      "%%x%%",
      "<!-- x -->",
      "```",
      "~~~~md",
      "%% open",
      "~~~~",
      "$$",
      "a %% b",
      "$$",
      "> ```",
      "> %%quoted%%",
      "> ```",
      "after %%gone%%",
    ].join("\n");
    const expected = input.replace("after %%gone%%", "after");
    expect(hideComments(input)).toBe(expected);
  });

  it("does not take an inline span for a block opener", () => {
    // remark reads both as inline spans, so hiding must carry on after them.
    expect(
      hideComments("```npm i``` installs it %%one%%\n\nLater %%two%% text."),
    ).toBe("```npm i``` installs it\n\nLater  text.");
    expect(hideComments("$$x$$ is squared %%one%%\n\nLater %%two%%")).toBe(
      "$$x$$ is squared\n\nLater",
    );
  });

  it("closes a math block only on a line of bare dollar signs", () => {
    expect(hideComments("$$\nx = y $$ %% kept\n$$\n%%gone%% after")).toBe(
      "$$\nx = y $$ %% kept\n$$\n after",
    );
  });

  it("finds a fence that opens on a list item's line", () => {
    expect(
      hideComments("1. ```\n   code\n   ```\n\n%%one%%\n\n<!-- two --> shown"),
    ).toBe("1. ```\n   code\n   ```\n\n\n\n shown");
    expect(hideComments("- ```html\n  <!-- x\n  ```\n\nShown after.")).toBe(
      "- ```html\n  <!-- x\n  ```\n\nShown after.",
    );
  });

  it("hides a fence that sits inside a comment", () => {
    expect(hideComments("%%\n```\ncode\n```\n%%\nshown")).toBe(
      "\n\n\n\n\nshown",
    );
  });

  it("keeps a saved query name marker for remarkHideQueryMarkers", () => {
    const input = "<!-- hatchdoor-query: tasks -->\n```base\n```";
    expect(hideComments(input)).toBe(input);
    expect(hideComments("  <!--hatchdoor-query:x-->")).toBe(
      "  <!--hatchdoor-query:x-->",
    );
  });

  it("keeps a backslash-escaped backtick from opening a code span", () => {
    expect(hideComments("\\` %%gone%% `")).toBe("\\`  `");
  });

  it("keeps carriage returns", () => {
    expect(hideComments("a %%x%%\r\nb\r\n")).toBe("a\r\nb\r\n");
  });
});

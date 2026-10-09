import { describe, expect, it } from "vitest";

import {
  duplicateTitleHeadingLine,
  assignHeadingId,
  extractMarkdownHeadings,
  slugifyHeading,
} from "./noteHeadings";

describe("noteHeadings", () => {
  it("extracts headings and skips fenced code blocks", () => {
    const headings = extractMarkdownHeadings(
      ["# Title", "", "```md", "## Not a heading", "```", "## Overview"].join(
        "\n",
      ),
    );

    expect(headings).toEqual([
      { level: 1, text: "Title", id: "title", sourceLine: 1 },
      { level: 2, text: "Overview", id: "overview", sourceLine: 6 },
    ]);
  });

  it("assignHeadingId appends numeric suffixes for duplicates", () => {
    const counts = new Map<string, number>();

    expect(assignHeadingId("Alpha", counts)).toBe("alpha");
    expect(assignHeadingId("Alpha", counts)).toBe("alpha-2");
  });

  it("slugifyHeading normalizes markdown punctuation", () => {
    expect(slugifyHeading("**API** [Guide](x) / v1")).toBe("api-guide-v1");
  });

  it("slugifyHeading folds accents and keeps other scripts", () => {
    expect(slugifyHeading("Gerard Veá")).toBe("gerard-vea");
    expect(slugifyHeading("Cafe\u0301")).toBe(slugifyHeading("Caf\u00e9"));
    expect(slugifyHeading("Straße")).toBe("strasse");
    expect(slugifyHeading("Łódź")).toBe("lodz");
    expect(slugifyHeading("資料 Обзор")).toBe("資料-обзор");
    expect(slugifyHeading("हिन्दी")).toBe("हिन्दी");
  });

  it("slugifyHeading falls back when nothing addressable is left", () => {
    expect(slugifyHeading("!!! ???")).toBe("section");
  });

  it("extracts obsidian wikilink heading labels for toc ids", () => {
    const headings = extractMarkdownHeadings("## [[Project/Plan|Plan Home]]");

    expect(headings).toEqual([
      { level: 2, text: "Plan Home", id: "plan-home", sourceLine: 1 },
    ]);
  });
});

describe("duplicateTitleHeadingLine (#530)", () => {
  const body = "# RackGate\n\nBody\n\n## Role\n";
  const headings = extractMarkdownHeadings(body);

  it("names the first H1 that repeats the title", () => {
    expect(duplicateTitleHeadingLine(body, headings, "RackGate")).toBe(1);
  });

  it("ignores case and runs of whitespace", () => {
    expect(duplicateTitleHeadingLine(body, headings, "rack  gate")).toBe(
      undefined,
    );
    expect(duplicateTitleHeadingLine(body, headings, "  rackgate ")).toBe(1);
  });

  it("allows only blank lines before the heading", () => {
    const led = "\n\n# RackGate\n";
    expect(
      duplicateTitleHeadingLine(led, extractMarkdownHeadings(led), "RackGate"),
    ).toBe(3);
    const prose = "Intro\n\n# RackGate\n";
    expect(
      duplicateTitleHeadingLine(
        prose,
        extractMarkdownHeadings(prose),
        "RackGate",
      ),
    ).toBeUndefined();
  });

  it("never names an H2 or a later H1", () => {
    const h2 = "## RackGate\n";
    expect(
      duplicateTitleHeadingLine(h2, extractMarkdownHeadings(h2), "RackGate"),
    ).toBeUndefined();
    const later = "## Intro\n\n# RackGate\n";
    expect(
      duplicateTitleHeadingLine(
        later,
        extractMarkdownHeadings(later),
        "RackGate",
      ),
    ).toBeUndefined();
  });

  it("is nothing without a title or a heading", () => {
    expect(
      duplicateTitleHeadingLine(body, headings, undefined),
    ).toBeUndefined();
    expect(duplicateTitleHeadingLine("", [], "RackGate")).toBeUndefined();
  });
});

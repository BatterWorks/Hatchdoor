import { describe, expect, it } from "vitest";

import type { VaultSummary } from "../../types";
import {
  encodeLinkPath,
  escapeLinkText,
  linkStyleOf,
  markdownLinkPath,
  noteCandidateResolves,
  noteLinkText,
  relativeLinkPath,
  WIKILINK_STYLE,
  type VaultLinkStyle,
} from "./linkStyle";
import { decodePercent, findMarkdownNoteLinks } from "./markdownLinks";

const markdown = (pathForm: VaultLinkStyle["pathForm"]): VaultLinkStyle => ({
  style: "markdown",
  pathForm,
});

describe("linkStyleOf", () => {
  it("reads a Markdown Vault's style and path form", () => {
    expect(
      linkStyleOf({
        link_style: "markdown",
        link_path_form: "absolute",
      } as VaultSummary),
    ).toEqual(markdown("absolute"));
  });

  it("falls back to wikilinks when the Vault reports none", () => {
    expect(linkStyleOf(undefined)).toBe(WIKILINK_STYLE);
    expect(linkStyleOf({} as VaultSummary)).toBe(WIKILINK_STYLE);
    expect(
      linkStyleOf({
        link_style: "wikilink",
        link_path_form: "relative",
      } as VaultSummary),
    ).toBe(WIKILINK_STYLE);
  });
});

describe("encodeLinkPath", () => {
  it("encodes only what would break the link", () => {
    expect(encodeLinkPath("My Notes/a#b (1) [x] 100%.md")).toBe(
      "My%20Notes/a%23b%20%281%29%20%5Bx%5D%20100%25.md",
    );
  });

  it("keeps accented and non-Latin letters readable", () => {
    expect(encodeLinkPath("Café/日本語 ملاحظة.md")).toBe(
      "Café/日本語%20ملاحظة.md",
    );
  });

  it("encodes a control character byte by byte", () => {
    expect(encodeLinkPath("a\tb")).toBe("a%09b");
  });
});

describe("escapeLinkText", () => {
  it("escapes what could close the text or open a code span", () => {
    expect(escapeLinkText("a [b] `c` \\d")).toBe("a \\[b\\] \\`c\\` \\\\d");
  });
});

describe("relativeLinkPath", () => {
  it("walks from the linking note's folder to the target", () => {
    expect(relativeLinkPath("Home", "Target.md")).toBe("Target.md");
    expect(relativeLinkPath("a/b/Note", "a/c/Target.md")).toBe(
      "../c/Target.md",
    );
    expect(relativeLinkPath("a/Note", "a/b/Target.md")).toBe("b/Target.md");
    expect(relativeLinkPath("a/b/Note", "Target.md")).toBe("../../Target.md");
    expect(relativeLinkPath("a/Note", "a/Target.md")).toBe("Target.md");
  });
});

describe("markdownLinkPath", () => {
  const never = () => false;

  it("writes absolute paths from the Vault root", () => {
    expect(markdownLinkPath("absolute", "a/Note", "b/T.md", never)).toBe(
      "/b/T.md",
    );
  });

  it("takes the first shortest candidate that resolves, else the root path", () => {
    expect(
      markdownLinkPath("shortest", "a/Note", "b/T.md", (c) => c === "T.md"),
    ).toBe("T.md");
    expect(
      markdownLinkPath("shortest", "a/Note", "b/T.md", (c) => c === "b/T.md"),
    ).toBe("b/T.md");
    expect(markdownLinkPath("shortest", "a/Note", "b/T.md", never)).toBe(
      "/b/T.md",
    );
  });
});

describe("noteCandidateResolves", () => {
  const notes = ["a/Plan.md", "b/Plan.md", "b/Solo.md", "a/b/Plan.md"];

  it("accepts a bare name only when the name is unique in the Vault", () => {
    expect(noteCandidateResolves("Solo.md", "b/Solo.md", "x/Home", notes)).toBe(
      true,
    );
    expect(noteCandidateResolves("Plan.md", "b/Plan.md", "x/Home", notes)).toBe(
      false,
    );
  });

  it("matches names case-insensitively, as the server does", () => {
    expect(noteCandidateResolves("solo.md", "b/Solo.md", "x/Home", notes)).toBe(
      true,
    );
  });

  it("rejects a Vault path a note-relative path would shadow", () => {
    // From a/Home, "b/Plan.md" first reads as a/b/Plan.md.
    expect(
      noteCandidateResolves("b/Plan.md", "b/Plan.md", "a/Home", notes),
    ).toBe(false);
    expect(
      noteCandidateResolves("b/Plan.md", "b/Plan.md", "x/Home", notes),
    ).toBe(true);
    expect(
      noteCandidateResolves("/b/Plan.md", "b/Plan.md", "a/Home", notes),
    ).toBe(true);
  });
});

describe("noteLinkText", () => {
  const notes = ["Projects/Plan (v2) #1 [draft].md", "Home.md", "日本語.md"];

  it("writes [[title]] in a wikilink Vault", () => {
    expect(
      noteLinkText(WIKILINK_STYLE, "Plan", "Projects/Plan.md", "Home", notes),
    ).toBe("[[Plan]]");
  });

  it("writes each Markdown path form, round-tripping awkward names", () => {
    const title = "Plan (v2) #1 [draft]";
    const target = "Projects/Plan (v2) #1 [draft].md";
    const cases: [VaultLinkStyle["pathForm"], string, string][] = [
      ["relative", "Archive/Home", "../Projects/Plan (v2) #1 [draft].md"],
      ["absolute", "Archive/Home", "/Projects/Plan (v2) #1 [draft].md"],
      ["shortest", "Archive/Home", "Plan (v2) #1 [draft].md"],
    ];
    for (const [form, from, path] of cases) {
      const text = noteLinkText(markdown(form), title, target, from, notes);
      expect(text).toBe(`[Plan (v2) #1 \\[draft\\]](${encodeLinkPath(path)})`);
      // The link reads back as one note link to that path, with no anchor.
      const links = findMarkdownNoteLinks(text);
      expect(links).toHaveLength(1);
      expect(decodePercent(links[0].rawPath)).toBe(path);
      expect(links[0].anchor).toBe("");
    }
  });

  it("keeps non-Latin titles readable", () => {
    expect(
      noteLinkText(markdown("shortest"), "日本語", "日本語.md", "Home", notes),
    ).toBe("[日本語](日本語.md)");
  });
});

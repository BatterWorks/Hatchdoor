import { describe, expect, it } from "vitest";

import {
  normalizeTags,
  parseFrontmatter,
  parseWikilinkTarget,
  stripVaultNoteLinks,
} from "./markdown";

describe("parseFrontmatter", () => {
  it("extracts inline-array frontmatter and strips it from body", () => {
    const input = `---
tags: [type/reference, status/active]
created: 2026-02-06
---

# Title
Body`;

    const parsed = parseFrontmatter(input);
    expect(parsed.properties.tags).toEqual(["type/reference", "status/active"]);
    expect(parsed.properties.created).toBe("2026-02-06");
    expect(parsed.body).toContain("# Title");
    expect(parsed.body).not.toContain("tags:");
  });

  it("extracts multi-line yaml lists", () => {
    const input = `---
tags:
  - alpha
  - beta
---
Hello`;

    const parsed = parseFrontmatter(input);
    expect(parsed.properties.tags).toEqual(["alpha", "beta"]);
    expect(parsed.body).toBe("Hello");
  });

  it("keeps markdown content when leading hr is not frontmatter", () => {
    const input = `---
This is just note text
---

# Title`;

    const parsed = parseFrontmatter(input);
    expect(parsed.properties).toEqual({});
    expect(parsed.body).toBe(input);
  });

  it("accepts keys with spaces and dots", () => {
    const input = `---
publish date: 2026-02-08
build.version: "1.2.3"
---
Body`;

    const parsed = parseFrontmatter(input);
    expect(parsed.properties["publish date"]).toBe("2026-02-08");
    expect(parsed.properties["build.version"]).toBe("1.2.3");
    expect(parsed.body).toBe("Body");
  });
});

describe("normalizeTags", () => {
  it("normalizes leading hash and deduplicates", () => {
    expect(normalizeTags(["#alpha", "alpha", "beta"])).toEqual([
      "alpha",
      "beta",
    ]);
  });

  it("splits csv strings", () => {
    expect(normalizeTags("type/reference, status/active")).toEqual([
      "type/reference",
      "status/active",
    ]);
  });
});

describe("stripVaultNoteLinks", () => {
  it("strips vault-only note links to readable labels", () => {
    expect(
      stripVaultNoteLinks(
        "See [[Projects/Plan|Plan Home]], [[Topic#Part]], [Internal](/n/topic), [Missing](/__missing__/Topic), and [External](https://example.com).",
      ),
    ).toBe(
      "See Plan Home, Topic, Internal, Missing, and [External](https://example.com).",
    );
  });
});

describe("stripVaultNoteLinks line counts", () => {
  // The wikilink pattern used [^\]]+, which excludes ] but not newline, so an
  // unclosed [[ matched across lines until it found ]] anywhere later and
  // collapsed everything between into one line. A dangling [[ is exactly what
  // the autocomplete leaves behind mid-typing.
  it("preserves line count when a note contains a dangling open bracket", () => {
    const input =
      "TODO link to [[\n\nAnother paragraph with [[Real Note]] here.";

    const output = stripVaultNoteLinks(input);

    expect(output.split("\n")).toHaveLength(input.split("\n").length);
  });

  it("does not treat a dangling open bracket as a link", () => {
    const output = stripVaultNoteLinks("TODO link to [[\n\nSee [[Real Note]].");

    expect(output).toBe("TODO link to [[\n\nSee Real Note.");
  });

  it("leaves a wikilink split across lines as literal text", () => {
    const input = "See [[Real\nNote]] there.";

    expect(stripVaultNoteLinks(input)).toBe(input);
  });
});

describe("parseWikilinkTarget", () => {
  it("reads an escaped alias pipe as syntax, not as part of the target", () => {
    // The form a markdown table cell forces: a bare pipe would end the cell.
    expect(parseWikilinkTarget("Some Note\\|alias")).toEqual({
      target: "Some Note",
      label: "alias",
    });
  });

  it("gives the escaped form the same target as the unescaped one", () => {
    expect(parseWikilinkTarget("Some Note\\|alias").target).toBe(
      parseWikilinkTarget("Some Note|alias").target,
    );
  });

  it("keeps a backslash that is a path separator rather than an escape", () => {
    expect(parseWikilinkTarget("folder\\Some Note")).toEqual({
      target: "folder\\Some Note",
      label: "folder\\Some Note",
    });
  });

  it("reads an escaped pipe after a path or an anchor", () => {
    expect(parseWikilinkTarget("folder/Old\\|alias").target).toBe("folder/Old");
    expect(parseWikilinkTarget("Old#Heading\\|alias").target).toBe(
      "Old#Heading",
    );
  });

  it("reads an escaped size suffix on an asset embed", () => {
    expect(parseWikilinkTarget("image.png\\|200")).toEqual({
      target: "image.png",
      label: "200",
    });
  });
});

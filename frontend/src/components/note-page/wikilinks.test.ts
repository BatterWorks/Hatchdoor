import { describe, expect, it } from "vitest";

import { attachmentEmbedPath } from "./attachmentDrop";
import { resolveAssetHref, rewriteWikilinks } from "./wikilinks";

const VAULT_ID = "vault-1";

describe("resolveAssetHref", () => {
  it("passes through absolute and root paths", () => {
    expect(
      resolveAssetHref(VAULT_ID, "https://cdn.example.com/a.png", "Notes/Home"),
    ).toBe("https://cdn.example.com/a.png");
    expect(resolveAssetHref(VAULT_ID, "/public/a.png", "Notes/Home")).toBe(
      "/public/a.png",
    );
    expect(
      resolveAssetHref(VAULT_ID, "data:image/png;base64,abc", "Notes/Home"),
    ).toBe("data:image/png;base64,abc");
  });

  it("resolves note-relative paths to vault assets with encoding", () => {
    expect(
      resolveAssetHref(
        VAULT_ID,
        "Media stack.png",
        "Notes/40-reference/Homelab Atlas",
      ),
    ).toBe(
      "/api/v1/vaults/vault-1/assets/Notes/40-reference/Media%20stack.png",
    );
    expect(
      resolveAssetHref(VAULT_ID, "../img/a b.png#v=1", "Notes/Sub/Entry"),
    ).toBe("/api/v1/vaults/vault-1/assets/Notes/img/a%20b.png#v=1");
    expect(
      resolveAssetHref(VAULT_ID, "..\\img\\a b.png?raw=1", "Notes/Sub/Entry"),
    ).toBe("/api/v1/vaults/vault-1/assets/Notes/img/a%20b.png?raw=1");
  });

  it("returns raw target when normalization becomes empty", () => {
    expect(resolveAssetHref(VAULT_ID, "../../..", "Notes/Sub/Entry")).toBe(
      "../../..",
    );
  });

  it("keys the resolved href to the given vault", () => {
    expect(resolveAssetHref("vault-2", "Media stack.png", "Notes/Home")).toBe(
      "/api/v1/vaults/vault-2/assets/Notes/Media%20stack.png",
    );
  });
});

describe("uploaded attachment round trip", () => {
  // The embed we write and the href we resolve are computed by different
  // modules. If they ever disagree, every attachment in a subfolder note 404s,
  // which is exactly the bug this pairing fixes.
  it.each(["Home.md", "Projects/Foo.md", "Projects/2026/Q3/Foo.md"])(
    "resolves back to the uploaded path from %s",
    (notePath) => {
      const embed = attachmentEmbedPath("Attachments/report.pdf", notePath);

      expect(resolveAssetHref(VAULT_ID, embed, notePath)).toBe(
        "/api/v1/vaults/vault-1/assets/Attachments/report.pdf",
      );
    },
  );
});

describe("rewriteWikilinks line counts", () => {
  // Under autosave, a rendered tree with fewer lines than the source means
  // every block below the collapse is misaddressed, so an edit writes to the
  // wrong lines and confirms the hash: silent, persisted file corruption.
  it("preserves line count when a note contains a dangling open bracket", () => {
    const input = "TODO link to [[\n\nAnother paragraph with [[Real Note]].";

    const output = rewriteWikilinks(VAULT_ID, input, "Home.md", new Map());

    expect(output.split("\n")).toHaveLength(input.split("\n").length);
  });

  it("leaves a wikilink split across lines as literal text", () => {
    const input = "See [[Real\nNote]] there.";

    expect(rewriteWikilinks(VAULT_ID, input, "Home.md", new Map())).toBe(input);
  });

  it("preserves line count when an embed target spans lines", () => {
    const input = "![[Attachments/a\nb.png]]\n\nAfter.";

    const output = rewriteWikilinks(VAULT_ID, input, "Home.md", new Map());

    expect(output.split("\n")).toHaveLength(input.split("\n").length);
  });

  it("still rewrites an ordinary wikilink to its resolved slug", () => {
    const resolved = new Map([
      ["Real Note", { slug: "real-note", archived: false }],
    ]);

    expect(
      rewriteWikilinks(VAULT_ID, "See [[Real Note]].", "Home.md", resolved),
    ).toBe("See [Real Note](/v/vault-1/n/real-note).");
  });
});

describe("rewriteWikilinks asset resolution", () => {
  // Obsidian's default link format writes a bare filename and resolves it by
  // searching the vault, so a note that is not a sibling of the attachment
  // rendered a broken embed before the server started resolving these (#158).
  it("uses the server-resolved path for a bare embed target", () => {
    const assets = new Map([
      ["Some document.pdf", "98_Attachments/Some document.pdf"],
    ]);

    expect(
      rewriteWikilinks(
        VAULT_ID,
        "![[Some document.pdf]]",
        "97_Notes/Some note.md",
        new Map(),
        assets,
      ),
    ).toBe(
      "![Some document\\.pdf](/api/v1/vaults/vault-1/assets/98_Attachments/Some%20document.pdf)",
    );
  });

  it("uses the server-resolved path for a bare PDF wikilink", () => {
    const assets = new Map([["Plan.pdf", "98_Attachments/Plan.pdf"]]);

    expect(
      rewriteWikilinks(
        VAULT_ID,
        "[[Plan.pdf]]",
        "97_Notes/Some note.md",
        new Map(),
        assets,
      ),
    ).toBe(
      "[Plan\\.pdf](/api/v1/vaults/vault-1/assets/98_Attachments/Plan.pdf)",
    );
  });

  it("keeps the anchor suffix when the target resolves", () => {
    const assets = new Map([["Plan.pdf", "98_Attachments/Plan.pdf"]]);

    expect(
      rewriteWikilinks(
        VAULT_ID,
        "![[Plan.pdf#page=3]]",
        "97_Notes/Some note.md",
        new Map(),
        assets,
      ),
    ).toBe(
      "![Plan\\.pdf](/api/v1/vaults/vault-1/assets/98_Attachments/Plan.pdf#page=3)",
    );
  });

  it("falls back to the note-relative path when nothing resolved", () => {
    expect(
      rewriteWikilinks(
        VAULT_ID,
        "![[shot.png]]",
        "97_Notes/Some note.md",
        new Map(),
        new Map([["shot.png", null]]),
      ),
    ).toBe("![shot\\.png](/api/v1/vaults/vault-1/assets/97_Notes/shot.png)");
  });
});

describe("rewriteWikilinks with note embeds (#573)", () => {
  const other = { slug: "other-note", archived: false };
  const embed = (
    markdown: string,
    notes: [string, { slug: string; archived: boolean } | null][],
    assets: [string, string | null][] = [],
  ) =>
    rewriteWikilinks(
      VAULT_ID,
      markdown,
      "Home.md",
      new Map(notes),
      new Map(assets),
    );

  it("draws an embedded note as a link to it, not an image", () => {
    const output = embed("![[Other note]]", [["Other note", other]]);
    expect(output).toBe("[Other note](/v/vault-1/n/other-note)");
    expect(output.startsWith("!")).toBe(false);
  });

  it("links a heading embed to the heading and a block embed to the note", () => {
    expect(
      embed("![[Other note#Some Heading]]", [
        ["Other note#Some Heading", other],
      ]),
    ).toBe("[Other note](/v/vault-1/n/other-note#some-heading)");
    expect(
      embed("![[Other note#^abc123]]", [["Other note#^abc123", other]]),
    ).toBe("[Other note](/v/vault-1/n/other-note)");
  });

  it("labels and routes an embed the way the same wikilink is", () => {
    const archived = { slug: "Old/Other note.md", archived: true };
    expect(embed("![[Old/Other note]]", [["Old/Other note", archived]])).toBe(
      embed("[[Old/Other note]]", [["Old/Other note", archived]]),
    );
    expect(embed("![[Other note|alias]]", [["Other note", other]])).toBe(
      "[alias](/v/vault-1/n/other-note)",
    );
  });

  it("draws an embed of a missing note as the missing-note link", () => {
    expect(embed("![[Missing note]]", [["Missing note", null]])).toBe(
      embed("[[Missing note]]", [["Missing note", null]]),
    );
  });

  it("tries a .md embed as a note even when a file answered to it", () => {
    expect(
      embed(
        "![[Other note.md]]",
        [["Other note.md", other]],
        [["Other note.md", "Other note.md"]],
      ),
    ).toBe("[Other note\\.md](/v/vault-1/n/other-note)");
  });

  it("finds a note whose name ends like an extension when no file matches", () => {
    expect(embed("![[Release 2.8]]", [["Release 2.8", other]])).toBe(
      "[Release 2\\.8](/v/vault-1/n/other-note)",
    );
  });

  it("keeps image and PDF embeds as they were", () => {
    expect(
      embed(
        "![[picture.png]] ![[report.pdf]]",
        [
          ["picture.png", null],
          ["report.pdf", null],
        ],
        [
          ["picture.png", "Attachments/picture.png"],
          ["report.pdf", "Attachments/report.pdf"],
        ],
      ),
    ).toBe(
      "![picture\\.png](/api/v1/vaults/vault-1/assets/Attachments/picture.png) ![report\\.pdf](/api/v1/vaults/vault-1/assets/Attachments/report.pdf)",
    );
    // A file name nothing resolved keeps the note-relative reading.
    expect(
      embed("![[shot.png]]", [["shot.png", null]], [["shot.png", null]]),
    ).toBe("![shot\\.png](/api/v1/vaults/vault-1/assets/shot.png)");
  });
});

describe("rewriteWikilinks with an escaped alias pipe", () => {
  it("renders a table-cell link as a resolved link labelled with its alias", () => {
    const resolved = new Map([
      ["Host - BatterGate", { slug: "host-battergate", archived: false }],
    ]);

    expect(
      rewriteWikilinks(
        VAULT_ID,
        "| 443 | [[Host - BatterGate\\|battergate]] |",
        "Home.md",
        resolved,
      ),
    ).toBe("| 443 | [battergate](/v/vault-1/n/host-battergate) |");
  });

  it("resolves an escaped embed's asset and labels it with the size suffix", () => {
    const assets = new Map([["image.png", "98_Attachments/image.png"]]);

    expect(
      rewriteWikilinks(
        VAULT_ID,
        "| pic | ![[image.png\\|200]] |",
        "97_Notes/Some note.md",
        new Map(),
        assets,
      ),
    ).toBe(
      "| pic | ![200](/api/v1/vaults/vault-1/assets/98_Attachments/image.png) |",
    );
  });
});

describe("rewriteWikilinks with Markdown note links (ADR-28)", () => {
  const resolved = new Map([
    [
      "../20-projects/Beacon%20Launch.md",
      { slug: "beacon-launch", archived: false },
    ],
    [
      "../20-projects/Beacon Launch.md",
      { slug: "beacon-launch", archived: false },
    ],
    ["Install.md", { slug: "install", archived: false }],
    ["Old.md", { slug: "old", archived: true }],
  ]);

  function rewrite(markdown: string): string {
    return rewriteWikilinks(
      VAULT_ID,
      markdown,
      "00-inbox/Home",
      new Map(),
      new Map(),
      resolved,
    );
  }

  it("routes a resolved link in-app and keeps its text and title", () => {
    expect(rewrite('[x](../20-projects/Beacon%20Launch.md "t")')).toBe(
      '[x](/v/vault-1/n/beacon-launch "t")',
    );
    expect(rewrite("[x](<../20-projects/Beacon Launch.md>)")).toBe(
      "[x](/v/vault-1/n/beacon-launch)",
    );
  });

  it("lands both anchor spellings on the same heading", () => {
    expect(
      rewrite("[a](Install.md#First%20Run) [b](Install.md#first-run)"),
    ).toBe(
      "[a](/v/vault-1/n/install#first-run) [b](/v/vault-1/n/install#first-run)",
    );
  });

  it("rewrites a reference definition so every use follows", () => {
    expect(rewrite("[see][i] and [i]\n\n[i]: Install.md")).toBe(
      "[see][i] and [i]\n\n[i]: /v/vault-1/n/install",
    );
  });

  it("sends an unresolved target to the missing-link form and an archived one to the archive", () => {
    expect(rewrite("[x](Nope.md) [y](Old.md)")).toBe(
      "[x](/__missing__/Nope.md) [y](/__archived__/old)",
    );
  });

  it("leaves other links, images and code alone and keeps the line count", () => {
    const markdown = [
      "[pdf](report.pdf) ![x](Install.md) [web](https://example.com/a.md)",
      "`[code](Install.md)`",
      "```",
      "[fenced](Install.md)",
      "```",
    ].join("\n");
    const out = rewrite(markdown);
    expect(out).toBe(markdown);
    expect(out.split("\n")).toHaveLength(markdown.split("\n").length);
  });
});

describe("rewriteWikilinks with Markdown images (ADR-33 inserts)", () => {
  const rewriteImages = (
    markdown: string,
    assets: Map<string, string | null>,
  ) => rewriteWikilinks(VAULT_ID, markdown, "Notes/Home.md", new Map(), assets);

  it("points root-anchored, bare-name and encoded images at the resolved file", () => {
    const assets = new Map<string, string | null>([
      ["/Attachments/a b.png", "Attachments/a b.png"],
      ["c.png", "Attachments/c.png"],
      ["../Attachments/d (1).pdf", "Attachments/d (1).pdf"],
    ]);
    expect(
      rewriteImages(
        "![](/Attachments/a%20b.png)\n![](c.png) ![](<../Attachments/d (1).pdf>)",
        assets,
      ),
    ).toBe(
      "![](/api/v1/vaults/vault-1/assets/Attachments/a%20b.png)\n" +
        "![](/api/v1/vaults/vault-1/assets/Attachments/c.png) " +
        "![](/api/v1/vaults/vault-1/assets/Attachments/d%20(1).pdf)",
    );
  });

  it("leaves unresolved, external and code images as written", () => {
    const markdown = [
      "![](missing.png) ![](https://example.com/x.png)",
      "`![](c.png)`",
    ].join("\n");
    expect(rewriteImages(markdown, new Map([["missing.png", null]]))).toBe(
      markdown,
    );
  });
});

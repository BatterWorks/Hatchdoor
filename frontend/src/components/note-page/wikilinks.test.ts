import { describe, expect, it } from "vitest";

import { attachmentEmbedPath } from "./attachmentDrop";
import { resolveAssetHref, rewriteWikilinks } from "./wikilinks";

describe("resolveAssetHref", () => {
  it("passes through absolute and root paths", () => {
    expect(
      resolveAssetHref("https://cdn.example.com/a.png", "Notes/Home"),
    ).toBe("https://cdn.example.com/a.png");
    expect(resolveAssetHref("/public/a.png", "Notes/Home")).toBe(
      "/public/a.png",
    );
    expect(resolveAssetHref("data:image/png;base64,abc", "Notes/Home")).toBe(
      "data:image/png;base64,abc",
    );
  });

  it("resolves note-relative paths to vault-assets with encoding", () => {
    expect(
      resolveAssetHref("Media stack.png", "Notes/40-reference/Homelab Atlas"),
    ).toBe("/vault-assets/Notes/40-reference/Media%20stack.png");
    expect(resolveAssetHref("../img/a b.png#v=1", "Notes/Sub/Entry")).toBe(
      "/vault-assets/Notes/img/a%20b.png#v=1",
    );
    expect(resolveAssetHref("..\\img\\a b.png?raw=1", "Notes/Sub/Entry")).toBe(
      "/vault-assets/Notes/img/a%20b.png?raw=1",
    );
  });

  it("returns raw target when normalization becomes empty", () => {
    expect(resolveAssetHref("../../..", "Notes/Sub/Entry")).toBe("../../..");
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

      expect(resolveAssetHref(embed, notePath)).toBe(
        "/vault-assets/Attachments/report.pdf",
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

    const output = rewriteWikilinks(input, "Home.md", new Map());

    expect(output.split("\n")).toHaveLength(input.split("\n").length);
  });

  it("leaves a wikilink split across lines as literal text", () => {
    const input = "See [[Real\nNote]] there.";

    expect(rewriteWikilinks(input, "Home.md", new Map())).toBe(input);
  });

  it("preserves line count when an embed target spans lines", () => {
    const input = "![[Attachments/a\nb.png]]\n\nAfter.";

    const output = rewriteWikilinks(input, "Home.md", new Map());

    expect(output.split("\n")).toHaveLength(input.split("\n").length);
  });

  it("still rewrites an ordinary wikilink to its resolved slug", () => {
    const resolved = new Map([
      ["Real Note", { slug: "real-note", archived: false }],
    ]);

    expect(rewriteWikilinks("See [[Real Note]].", "Home.md", resolved)).toBe(
      "See [Real Note](/n/real-note).",
    );
  });
});

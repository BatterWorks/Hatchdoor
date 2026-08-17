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
    ).toBe("[Plan\\.pdf](/api/v1/vaults/vault-1/assets/98_Attachments/Plan.pdf)");
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

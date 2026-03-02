import { describe, expect, it } from "vitest";

import { resolveAssetHref } from "./wikilinks";

describe("resolveAssetHref", () => {
  it("passes through absolute and root paths", () => {
    expect(resolveAssetHref("https://cdn.example.com/a.png", "Notes/Home")).toBe(
      "https://cdn.example.com/a.png",
    );
    expect(resolveAssetHref("/public/a.png", "Notes/Home")).toBe("/public/a.png");
    expect(resolveAssetHref("data:image/png;base64,abc", "Notes/Home")).toBe(
      "data:image/png;base64,abc",
    );
  });

  it("resolves note-relative paths to vault-assets with encoding", () => {
    expect(resolveAssetHref("Media stack.png", "Notes/40-reference/Homelab Atlas")).toBe(
      "/vault-assets/Notes/40-reference/Media%20stack.png",
    );
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

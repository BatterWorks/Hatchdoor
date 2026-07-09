import { describe, expect, it } from "vitest";

import { readErrorMessage } from "./apiError";

describe("readErrorMessage", () => {
  it("returns the server's structured error message when present", async () => {
    const res = new Response(
      JSON.stringify({ error: "Note not found: home" }),
      {
        status: 404,
      },
    );
    expect(await readErrorMessage(res, "Failed loading note")).toBe(
      "Note not found: home",
    );
  });

  it("falls back to the status when the body has no error string", async () => {
    const res = new Response("not json at all", { status: 500 });
    expect(await readErrorMessage(res, "Failed loading note")).toBe(
      "Failed loading note: 500",
    );
  });

  it("falls back when the error field is empty or not a string", async () => {
    const res = new Response(JSON.stringify({ error: "" }), { status: 503 });
    expect(await readErrorMessage(res, "Failed loading tree")).toBe(
      "Failed loading tree: 503",
    );
  });
});

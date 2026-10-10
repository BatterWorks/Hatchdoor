import { describe, expect, it, vi } from "vitest";

import { createNoteLinkResolver, wikilinkLabel } from "./noteLinks";

describe("createNoteLinkResolver", () => {
  it("folds the targets asked in one turn into one batch and answers each", async () => {
    const batch = vi.fn(async (targets: string[]) => {
      const answers = new Map<string, { slug: string } | null>();
      for (const target of targets) {
        answers.set(target, target === "Missing" ? null : { slug: "found" });
      }
      return answers;
    });
    const resolve = createNoteLinkResolver(batch);

    const [alpha, missing, again] = await Promise.all([
      resolve("Alpha"),
      resolve("Missing"),
      resolve("Alpha"),
    ]);

    expect(batch).toHaveBeenCalledTimes(1);
    expect(batch).toHaveBeenCalledWith(["Alpha", "Missing"]);
    expect(alpha).toEqual({ slug: "found" });
    expect(missing).toBeNull();
    expect(again).toEqual({ slug: "found" });
  });

  it("starts a new batch for the next turn", async () => {
    const batch = vi.fn(async (targets: string[]) => {
      return new Map(targets.map((target) => [target, { slug: target }]));
    });
    const resolve = createNoteLinkResolver(batch);
    await resolve("One");
    await resolve("Two");
    expect(batch).toHaveBeenCalledTimes(2);
  });

  it("answers null for every target when the batch fails", async () => {
    const resolve = createNoteLinkResolver(async () => {
      throw new Error("offline");
    });
    await expect(resolve("Alpha")).resolves.toBeNull();
  });
});

describe("wikilinkLabel", () => {
  it("shows the target without its folders or heading, as the reading view does", () => {
    expect(wikilinkLabel("20-projects/Beacon Launch#Goal", false)).toBe(
      "Beacon Launch",
    );
    expect(wikilinkLabel("Beacon Launch^abc", false)).toBe("Beacon Launch");
  });

  it("keeps an archived note's path whole", () => {
    expect(wikilinkLabel("90-archive/Old CRM Trial", true)).toBe(
      "90-archive/Old CRM Trial",
    );
  });
});

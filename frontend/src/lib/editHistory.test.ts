import { describe, expect, it } from "vitest";

import { createEditHistory } from "./editHistory";

describe("createEditHistory", () => {
  it("returns null when there is nothing to undo", () => {
    const history = createEditHistory("one");

    expect(history.undo()).toBeNull();
  });

  it("undoes back to the previous committed state", () => {
    const history = createEditHistory("one");
    history.record("two", 1000);

    expect(history.undo()).toEqual({ content: "one" });
  });

  it("redoes what it just undid", () => {
    const history = createEditHistory("one");
    history.record("two", 1000);
    history.undo();

    expect(history.redo()).toEqual({ content: "two" });
  });

  it("returns null when there is nothing to redo", () => {
    const history = createEditHistory("one");

    expect(history.redo()).toBeNull();
  });

  // Continuous typing is one undo, not one per keystroke.
  it("coalesces edits inside the pause window", () => {
    const history = createEditHistory("");
    history.record("a", 1000);
    history.record("ab", 1100);
    history.record("abc", 1200);

    expect(history.undo()).toEqual({ content: "" });
  });

  it("breaks the coalescing window after a pause", () => {
    const history = createEditHistory("");
    history.record("a", 1000);
    history.record("ab", 3000);

    expect(history.undo()).toEqual({ content: "a" });
  });

  it("forces a break when told to, regardless of timing", () => {
    const history = createEditHistory("");
    history.record("a", 1000);
    history.breakRun();
    history.record("ab", 1100);

    expect(history.undo()).toEqual({ content: "a" });
  });

  it("drops the redo stack once a new edit is recorded", () => {
    const history = createEditHistory("one");
    history.record("two", 1000);
    history.undo();
    history.record("three", 5000);

    expect(history.redo()).toBeNull();
  });

  it("walks back through several entries in order", () => {
    const history = createEditHistory("one");
    history.record("two", 1000);
    history.breakRun();
    history.record("three", 5000);

    expect(history.undo()).toEqual({ content: "two" });
    expect(history.undo()).toEqual({ content: "one" });
    expect(history.undo()).toBeNull();
  });

  it("ignores a recorded state identical to the current one", () => {
    const history = createEditHistory("one");
    history.record("one", 1000);

    expect(history.undo()).toBeNull();
  });
});

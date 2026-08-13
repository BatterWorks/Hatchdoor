import { afterEach, describe, expect, it } from "vitest";

import {
  clampSidebarWidth,
  getStoredExpandedFolders,
  getStoredNumber,
  getStoredRecentNotes,
  getStoredScope,
  getStoredString,
  isEditableTarget,
  setStoredScope,
} from "./storage";

afterEach(() => {
  window.localStorage.clear();
});

describe("storage helpers", () => {
  it("getStoredNumber clamps values and falls back for invalid numbers", () => {
    window.localStorage.setItem("width", "999");
    expect(getStoredNumber("width", 268, 220, 420)).toBe(420);

    window.localStorage.setItem("width", "oops");
    expect(getStoredNumber("width", 268, 220, 420)).toBe(268);
  });

  it("getStoredRecentNotes filters malformed entries and handles bad json", () => {
    window.localStorage.setItem(
      "hatchdoor.recentNotes",
      JSON.stringify([
        {
          vaultId: "v1",
          slug: "home",
          title: "Home",
          relativePath: "Home",
          viewedAt: 1,
        },
        { slug: "bad", title: "Bad", relativePath: "Bad" },
      ]),
    );
    const parsed = getStoredRecentNotes();
    expect(parsed).toHaveLength(1);
    expect(parsed[0].slug).toBe("home");
    expect(parsed[0].vaultId).toBe("v1");

    window.localStorage.setItem("hatchdoor.recentNotes", "{");
    expect(getStoredRecentNotes()).toEqual([]);
  });

  it("getStoredScope defaults to all and round-trips a selected Vault", () => {
    expect(getStoredScope()).toBe("all");

    setStoredScope("vault-123");
    expect(getStoredScope()).toBe("vault-123");

    setStoredScope("all");
    expect(getStoredScope()).toBe("all");
  });

  it("getStoredExpandedFolders keeps only boolean map entries", () => {
    window.localStorage.setItem(
      "hatchdoor.expandedFolders",
      JSON.stringify({ Projects: true, Docs: "yes", "": true }),
    );
    expect(getStoredExpandedFolders()).toEqual({ Projects: true });

    window.localStorage.setItem("hatchdoor.expandedFolders", "{");
    expect(getStoredExpandedFolders()).toEqual({});
  });

  it("getStoredString trims blanks and clampSidebarWidth enforces bounds", () => {
    window.localStorage.setItem("key", "  abc  ");
    expect(getStoredString("key")).toBe("abc");
    window.localStorage.setItem("key", "   ");
    expect(getStoredString("key")).toBeNull();

    expect(clampSidebarWidth(150)).toBe(220);
    expect(clampSidebarWidth(333)).toBe(333);
    expect(clampSidebarWidth(500)).toBe(420);
  });

  it("isEditableTarget detects editable controls", () => {
    const input = document.createElement("input");
    expect(isEditableTarget(input)).toBe(true);

    const div = document.createElement("div");
    Object.defineProperty(div, "isContentEditable", {
      configurable: true,
      value: true,
    });
    expect(isEditableTarget(div)).toBe(true);

    const span = document.createElement("span");
    expect(isEditableTarget(span)).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
  });
});

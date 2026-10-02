import { afterEach, describe, expect, it, vi } from "vitest";

import {
  holdAppReload,
  isAppReloadHeld,
  resetAppReloadHolds,
  whenAppReloadReleased,
} from "./reloadGuard";

afterEach(() => resetAppReloadHolds());

describe("reloadGuard (#330)", () => {
  it("runs an action immediately when nothing is holding", () => {
    const reload = vi.fn();

    whenAppReloadReleased(reload);

    expect(reload).toHaveBeenCalledOnce();
  });

  it("defers an action until the last hold is released", () => {
    const reload = vi.fn();
    holdAppReload("note:one", true);
    holdAppReload("note:two", true);

    whenAppReloadReleased(reload);
    expect(reload).not.toHaveBeenCalled();

    holdAppReload("note:one", false);
    expect(isAppReloadHeld()).toBe(true);
    expect(reload).not.toHaveBeenCalled();

    holdAppReload("note:two", false);
    expect(isAppReloadHeld()).toBe(false);
    expect(reload).toHaveBeenCalledOnce();
  });

  it("releases by name, so one holder cannot drop another's hold", () => {
    holdAppReload("note:one", true);

    holdAppReload("note:two", false);

    expect(isAppReloadHeld()).toBe(true);
  });

  it("keeps a hold taken from inside the drain", () => {
    const later = vi.fn();
    holdAppReload("note:one", true);
    whenAppReloadReleased(() => {
      holdAppReload("note:two", true);
      whenAppReloadReleased(later);
    });

    holdAppReload("note:one", false);

    expect(isAppReloadHeld()).toBe(true);
    expect(later).not.toHaveBeenCalled();

    holdAppReload("note:two", false);
    expect(later).toHaveBeenCalledOnce();
  });

  it("runs a deferred action once, not again on the next release", () => {
    const reload = vi.fn();
    holdAppReload("note:one", true);
    whenAppReloadReleased(reload);

    holdAppReload("note:one", false);
    holdAppReload("note:one", true);
    holdAppReload("note:one", false);

    expect(reload).toHaveBeenCalledOnce();
  });
});

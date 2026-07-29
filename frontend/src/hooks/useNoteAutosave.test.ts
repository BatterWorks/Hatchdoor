import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useNoteAutosave } from "./useNoteAutosave";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function setup(save = vi.fn().mockResolvedValue({ content_hash: "h1" })) {
  const hook = renderHook(
    ({ content }: { content: string }) =>
      useNoteAutosave({
        content,
        baseHash: "h0",
        enabled: true,
        save,
      }),
    { initialProps: { content: "one" } },
  );
  return { hook, save };
}

describe("useNoteAutosave", () => {
  it("does not write when nothing has changed", () => {
    const { save } = setup();

    act(() => {
      vi.advanceTimersByTime(5000);
    });

    expect(save).not.toHaveBeenCalled();
  });

  it("writes immediately when a unit is committed", async () => {
    const { hook, save } = setup();

    await act(async () => {
      hook.result.current.commit("two");
    });

    expect(save).toHaveBeenCalledWith("two", "h0");
  });

  it("writes after an idle pause while still typing in one unit", async () => {
    const { hook, save } = setup();

    act(() => {
      hook.result.current.touch("typing");
    });
    expect(save).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(2100);
    });

    expect(save).toHaveBeenCalledWith("typing", "h0");
  });

  it("coalesces rapid typing into one write", async () => {
    const { hook, save } = setup();

    act(() => {
      hook.result.current.touch("a");
      vi.advanceTimersByTime(500);
      hook.result.current.touch("ab");
      vi.advanceTimersByTime(500);
      hook.result.current.touch("abc");
    });
    await act(async () => {
      vi.advanceTimersByTime(2100);
    });

    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith("abc", "h0");
  });

  it("saves against the hash the server last confirmed", async () => {
    const save = vi
      .fn()
      .mockResolvedValueOnce({ content_hash: "h1" })
      .mockResolvedValueOnce({ content_hash: "h2" });
    const { hook } = setup(save);

    await act(async () => {
      hook.result.current.commit("two");
    });
    await act(async () => {
      hook.result.current.commit("three");
    });

    expect(save).toHaveBeenNthCalledWith(2, "three", "h1");
  });

  // Each write produces two revision bumps, and a bump from write N can land
  // after write N+1 is confirmed. Comparing against only the latest hash
  // reports false divergence.
  it("recognises every hash it has written, not just the most recent", async () => {
    const save = vi
      .fn()
      .mockResolvedValueOnce({ content_hash: "h1" })
      .mockResolvedValueOnce({ content_hash: "h2" });
    const { hook } = setup(save);

    await act(async () => {
      hook.result.current.commit("two");
    });
    await act(async () => {
      hook.result.current.commit("three");
    });

    expect(hook.result.current.isOwnWrite("h1")).toBe(true);
    expect(hook.result.current.isOwnWrite("h2")).toBe(true);
    expect(hook.result.current.isOwnWrite("someone-else")).toBe(false);
  });

  it("stops autosaving and reports a conflict on a 409", async () => {
    const conflict = new Error("changed on disk");
    conflict.name = "ConflictError";
    const save = vi.fn().mockRejectedValue(conflict);
    const { hook } = setup(save);

    await act(async () => {
      hook.result.current.commit("two");
    });

    expect(hook.result.current.status).toBe("conflict");

    await act(async () => {
      hook.result.current.commit("three");
    });
    expect(save).toHaveBeenCalledTimes(1);
  });

  it("reports the saving and saved states around a write", async () => {
    const { hook, save } = setup();
    expect(hook.result.current.status).toBe("idle");

    await act(async () => {
      hook.result.current.commit("two");
    });

    expect(save).toHaveBeenCalled();
    expect(hook.result.current.status).toBe("saved");
  });
});

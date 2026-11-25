import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type Sender, useNoteAutosave } from "./useNoteAutosave";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function setup(save = vi.fn().mockResolvedValue({ content_hash: "h1" })) {
  const hook = renderHook(() =>
    useNoteAutosave({ noteKey: "a", baseHash: "h0", enabled: true, save }),
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

  it("keeps the idle deadline and original base across rerenders", async () => {
    const send = vi.fn().mockResolvedValue({ content_hash: "saved" });
    const saved = vi.fn();
    const hook = renderHook(
      ({ baseHash }) =>
        useNoteAutosave({
          noteKey: "a",
          baseHash,
          enabled: true,
          save: (content, hash) => send(content, hash),
          onSaved: (result) => saved(result),
        }),
      { initialProps: { baseHash: "h0" } },
    );
    act(() => hook.result.current.touch("typed on h0"));
    act(() => vi.advanceTimersByTime(500));
    hook.rerender({ baseHash: "external" });
    await act(async () => vi.advanceTimersByTime(1500));
    expect(send).toHaveBeenCalledExactlyOnceWith("typed on h0", "h0");
    expect(saved).toHaveBeenCalledWith({ content_hash: "saved" });
  });

  it("stops reporting Saved as soon as more text is typed", async () => {
    const { hook } = setup();
    await act(async () => hook.result.current.commit("saved text"));
    expect(hook.result.current.status).toBe("saved");
    act(() => hook.result.current.touch("not saved yet"));
    expect(hook.result.current.status).not.toBe("saved");
  });

  it("does not report Saved when typing continues during a save", async () => {
    let finish!: (result: { content_hash: string }) => void;
    const save = vi.fn(
      () =>
        new Promise<{ content_hash: string }>((resolve) => {
          finish = resolve;
        }),
    );
    const { hook } = setup(save);
    act(() => hook.result.current.commit("first"));
    act(() => hook.result.current.touch("newer"));
    await act(async () => finish({ content_hash: "h1" }));
    expect(hook.result.current.status).not.toBe("saved");
    await act(async () => vi.advanceTimersByTime(2000));
    expect(save).toHaveBeenLastCalledWith("newer", "h1");
  });

  it("warns before unloading refused edits, but not a saved document", async () => {
    const save = vi.fn().mockRejectedValue(new Error("unavailable"));
    const { hook } = setup(save);
    const clean = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(clean);
    expect(clean.defaultPrevented).toBe(false);
    await act(async () => hook.result.current.commit("refused text"));
    const dirty = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(dirty);
    expect(dirty.defaultPrevented).toBe(true);
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

describe("overlapping writes", () => {
  /** A save whose promise the test resolves by hand. */
  function deferredSave() {
    const calls: Array<{
      content: string;
      hash: string;
      resolve: (hash: string) => void;
    }> = [];
    const save = vi.fn((content: string, hash: string) => {
      return new Promise<{ content_hash: string }>((resolve) => {
        calls.push({
          content,
          hash,
          resolve: (h) => resolve({ content_hash: h }),
        });
      });
    });
    return { save, calls };
  }

  // Writes take about a second against a real vault, and every content change
  // commits, so a second edit landing mid-flight is routine rather than exotic.
  it("does not drop an edit made while a write is in flight", async () => {
    const { save, calls } = deferredSave();
    const hook = renderHook(() =>
      useNoteAutosave({ noteKey: "a", baseHash: "h0", enabled: true, save }),
    );

    act(() => {
      hook.result.current.commit("edit-A");
    });
    act(() => {
      hook.result.current.commit("edit-B");
    });
    expect(calls).toHaveLength(1);

    await act(async () => {
      calls[0].resolve("h1");
    });

    expect(calls).toHaveLength(2);
    expect(calls[1].content).toBe("edit-B");
    expect(calls[1].hash).toBe("h1");
  });

  it("writes only the newest content when several edits queue up", async () => {
    const { save, calls } = deferredSave();
    const hook = renderHook(() =>
      useNoteAutosave({ noteKey: "a", baseHash: "h0", enabled: true, save }),
    );

    act(() => {
      hook.result.current.commit("A");
      hook.result.current.commit("B");
      hook.result.current.commit("C");
    });

    await act(async () => {
      calls[0].resolve("h1");
    });

    expect(calls).toHaveLength(2);
    expect(calls[1].content).toBe("C");
  });

  // An ordinary fetch started from `pagehide` is cancelled when the document
  // is torn down, so the last seconds of typing die with the page. The unload
  // send is a separate, fire-and-forget path with keepalive semantics (#330).
  it("hands the unload flush to flushSave rather than the awaited save", () => {
    const save = vi.fn().mockResolvedValue({ content_hash: "h1" });
    const flushSave = vi.fn().mockResolvedValue({ content_hash: "h1" });
    const hook = renderHook(() =>
      useNoteAutosave({
        noteKey: "a",
        baseHash: "h0",
        enabled: true,
        save,
        flushSave,
      }),
    );

    act(() => {
      hook.result.current.touch("still typing");
    });
    act(() => {
      window.dispatchEvent(new Event("pagehide"));
    });

    expect(flushSave).toHaveBeenCalledExactlyOnceWith("still typing", "h0");
    expect(save).not.toHaveBeenCalled();
  });

  it("flushes an edit parked behind an in-flight save, not just the idle pause", async () => {
    const { save, calls } = deferredSave();
    const flushSave = vi.fn().mockResolvedValue({ content_hash: "h1" });
    const hook = renderHook(() =>
      useNoteAutosave({
        noteKey: "a",
        baseHash: "h0",
        enabled: true,
        save,
        flushSave,
      }),
    );

    act(() => {
      hook.result.current.commit("edit-A");
    });
    act(() => {
      // Queued behind the in-flight write, and nothing else holds it.
      hook.result.current.commit("edit-B");
    });
    expect(calls).toHaveLength(1);

    act(() => {
      window.dispatchEvent(new Event("pagehide"));
    });

    expect(flushSave).toHaveBeenCalledExactlyOnceWith("edit-B", "h0");
  });

  // Hiding a tab is not closing it. The flush still goes out with keepalive,
  // but its result has to be booked, or the hook comes back holding a hash the
  // vault has already superseded and the next keystroke conflicts (#330).
  it("books the flush that went out on a tab switch, so the next save is not a conflict", async () => {
    const save = vi.fn().mockResolvedValue({ content_hash: "h2" });
    const flushSave = vi.fn().mockResolvedValue({ content_hash: "h1" });
    const onSaved = vi.fn();
    const hook = renderHook(() =>
      useNoteAutosave({
        noteKey: "a",
        baseHash: "h0",
        enabled: true,
        save,
        flushSave,
        onSaved,
      }),
    );

    act(() => {
      hook.result.current.touch("still typing");
    });
    const visibility = vi
      .spyOn(document, "visibilityState", "get")
      .mockReturnValue("hidden");
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    visibility.mockReturnValue("visible");

    expect(flushSave).toHaveBeenCalledExactlyOnceWith("still typing", "h0");
    expect(onSaved).toHaveBeenCalledWith(
      { content_hash: "h1" },
      "still typing",
      "h0",
    );
    expect(hook.result.current.status).toBe("saved");

    await act(async () => {
      hook.result.current.commit("typing again");
    });

    expect(save).toHaveBeenCalledExactlyOnceWith("typing again", "h1");
  });

  it("sends nothing on unload once autosave has stopped", async () => {
    const failure = new Error("vault unreachable");
    const save = vi.fn().mockRejectedValue(failure);
    const flushSave = vi.fn().mockResolvedValue({ content_hash: "h1" });
    const hook = renderHook(() =>
      useNoteAutosave({
        noteKey: "a",
        baseHash: "h0",
        enabled: true,
        save,
        flushSave,
      }),
    );

    await act(async () => {
      hook.result.current.commit("one");
    });
    act(() => {
      hook.result.current.touch("two");
    });
    act(() => {
      window.dispatchEvent(new Event("pagehide"));
    });

    expect(flushSave).not.toHaveBeenCalled();
  });

  it("does not report saved while an edit is still queued", async () => {
    const { save, calls } = deferredSave();
    const hook = renderHook(() =>
      useNoteAutosave({ noteKey: "a", baseHash: "h0", enabled: true, save }),
    );

    act(() => {
      hook.result.current.commit("A");
      hook.result.current.commit("B");
    });
    await act(async () => {
      calls[0].resolve("h1");
    });

    expect(hook.result.current.status).toBe("saving");

    await act(async () => {
      calls[1].resolve("h2");
    });
    expect(hook.result.current.status).toBe("saved");
  });
});

describe("a reply for the note just left (#658)", () => {
  function twoNotes(save: Parameters<typeof useNoteAutosave>[0]["save"]) {
    const saved = vi.fn();
    const hook = renderHook(
      ({ noteKey, baseHash }) =>
        useNoteAutosave({
          noteKey,
          baseHash,
          enabled: true,
          save,
          onSaved: (result, content, sentAgainst) =>
            saved(noteKey, result, content, sentAgainst),
        }),
      { initialProps: { noteKey: "a", baseHash: "a-1" } },
    );
    return { hook, saved };
  }

  function held() {
    const replies: Array<(outcome: Error | { content_hash: string }) => void> =
      [];
    const save = vi.fn<Sender>(
      () =>
        new Promise<{ content_hash: string }>((resolve, reject) => {
          replies.push((outcome) =>
            outcome instanceof Error ? reject(outcome) : resolve(outcome),
          );
        }),
    );
    return { save, replies };
  }

  function conflict() {
    const error = new Error("changed");
    error.name = "ConflictError";
    return error;
  }

  it("keeps the next note's hash when the late save succeeds", async () => {
    const { save, replies } = held();
    const { hook, saved } = twoNotes(save);
    await act(async () => hook.result.current.commit("a text"));
    hook.rerender({ noteKey: "b", baseHash: "b-1" });
    await act(async () => replies[0]({ content_hash: "a-2" }));

    // The reply names the hash it replaced, even after the page moved on (#695).
    expect(saved).toHaveBeenCalledWith(
      "a",
      { content_hash: "a-2" },
      "a text",
      "a-1",
    );
    expect(hook.result.current.status).toBe("idle");
    expect(hook.result.current.isOwnWrite("a-2")).toBe(false);
    await act(async () => hook.result.current.commit("b text"));
    expect(save).toHaveBeenLastCalledWith("b text", "b-1");
  });

  it("keeps saving the next note when the late save is refused", async () => {
    const { save, replies } = held();
    const { hook, saved } = twoNotes(save);
    await act(async () => hook.result.current.commit("a text"));
    hook.rerender({ noteKey: "b", baseHash: "b-1" });
    await act(async () => replies[0](conflict()));

    expect(hook.result.current.status).toBe("idle");
    expect(saved).not.toHaveBeenCalled();
    await act(async () => hook.result.current.commit("b text"));
    expect(save).toHaveBeenLastCalledWith("b text", "b-1");
  });

  it("sends the next note's edit by its own save, not behind the late one", async () => {
    const { save, replies } = held();
    const { hook } = twoNotes(save);
    await act(async () => hook.result.current.commit("a text"));
    hook.rerender({ noteKey: "b", baseHash: "b-1" });
    await act(async () => hook.result.current.commit("b text"));
    expect(save).toHaveBeenCalledTimes(2);
    expect(save).toHaveBeenLastCalledWith("b text", "b-1");

    await act(async () => replies[1]({ content_hash: "b-2" }));
    expect(hook.result.current.status).toBe("saved");
    await act(async () => replies[0]({ content_hash: "a-2" }));
    expect(save).toHaveBeenCalledTimes(2);
    expect(hook.result.current.status).toBe("saved");
    await act(async () => hook.result.current.commit("b again"));
    expect(save).toHaveBeenLastCalledWith("b again", "b-2");
  });

  it("starts the next note with saving on after a refusal on the last one", async () => {
    const save = vi.fn().mockRejectedValueOnce(conflict());
    save.mockResolvedValue({ content_hash: "b-2" });
    const { hook } = twoNotes(save);
    await act(async () => hook.result.current.commit("a text"));
    expect(hook.result.current.status).toBe("conflict");
    hook.rerender({ noteKey: "b", baseHash: "b-1" });
    expect(hook.result.current.status).toBe("idle");
    await act(async () => hook.result.current.commit("b text"));
    expect(save).toHaveBeenLastCalledWith("b text", "b-1");
  });

  it("drops an idle pause left by the note just left", async () => {
    const save = vi.fn().mockResolvedValue({ content_hash: "x" });
    const { hook } = twoNotes(save);
    act(() => hook.result.current.touch("a typing"));
    hook.rerender({ noteKey: "b", baseHash: "b-1" });
    await act(async () => vi.advanceTimersByTime(5000));
    expect(save).not.toHaveBeenCalled();
    expect(hook.result.current.hasUnsaved()).toBe(false);
  });
});

describe("a return to the note whose save is still out (#803)", () => {
  function visits(save: Sender) {
    const saved = vi.fn();
    const hook = renderHook(
      ({ noteKey, noteId, baseHash }) =>
        useNoteAutosave({
          noteKey,
          noteId,
          baseHash,
          enabled: true,
          save,
          onSaved: (result, content, sentAgainst) =>
            saved(noteKey, result, content, sentAgainst),
        }),
      { initialProps: { noteKey: "a#0", noteId: "a", baseHash: "a-1" } },
    );
    return { hook, saved };
  }

  function held() {
    const replies: Array<(outcome: Error | { content_hash: string }) => void> =
      [];
    const save = vi.fn<Sender>(
      () =>
        new Promise<{ content_hash: string }>((resolve, reject) => {
          replies.push((outcome) =>
            outcome instanceof Error ? reject(outcome) : resolve(outcome),
          );
        }),
    );
    return { save, replies };
  }

  function conflict() {
    const error = new Error("changed");
    error.name = "ConflictError";
    return error;
  }

  /** Commit in a, visit b, come back to a, all before a's save answers. */
  async function returnMidSave(save: Sender) {
    const visited = visits(save);
    await act(async () => visited.hook.result.current.commit("a text"));
    visited.hook.rerender({ noteKey: "b#1", noteId: "b", baseHash: "b-1" });
    visited.hook.rerender({ noteKey: "a#2", noteId: "a", baseHash: "a-1" });
    return visited;
  }

  it("does not send the same text again, and takes over the save's hash", async () => {
    const { save, replies } = held();
    const { hook, saved } = await returnMidSave(save);
    await act(async () => hook.result.current.commitRestored("a text"));
    expect(save).toHaveBeenCalledTimes(1);
    expect(hook.result.current.status).toBe("saving");

    await act(async () => replies[0]({ content_hash: "a-2" }));
    expect(save).toHaveBeenCalledTimes(1);
    expect(hook.result.current.status).toBe("saved");
    expect(hook.result.current.hasUnsaved()).toBe(false);
    expect(hook.result.current.isOwnWrite("a-2")).toBe(true);
    // Booked for the visit that sent it, then for the visit that took it over.
    expect(saved.mock.calls.map((call) => call[0])).toEqual(["a#0", "a#2"]);

    await act(async () => hook.result.current.commit("a again"));
    expect(save).toHaveBeenLastCalledWith("a again", "a-2");
  });

  it("writes a newer edit after the save, against the hash it produced", async () => {
    const { save, replies } = held();
    const { hook } = await returnMidSave(save);
    await act(async () => hook.result.current.commitRestored("a newer"));
    expect(save).toHaveBeenCalledTimes(1);

    await act(async () => replies[0]({ content_hash: "a-2" }));
    expect(save).toHaveBeenCalledTimes(2);
    expect(save).toHaveBeenLastCalledWith("a newer", "a-2");
    await act(async () => replies[1]({ content_hash: "a-3" }));
    expect(hook.result.current.status).toBe("saved");
  });

  it("holds the same text unsaved, with saving on, when the save is refused", async () => {
    const { save, replies } = held();
    const { hook, saved } = await returnMidSave(save);
    await act(async () => hook.result.current.commitRestored("a text"));

    await act(async () => replies[0](conflict()));
    expect(save).toHaveBeenCalledTimes(1);
    expect(hook.result.current.status).toBe("idle");
    expect(hook.result.current.hasUnsaved()).toBe(true);
    expect(saved).not.toHaveBeenCalled();
  });

  it("sends a newer edit against its own hash when the save is refused", async () => {
    const { save, replies } = held();
    const { hook } = await returnMidSave(save);
    await act(async () => hook.result.current.commitRestored("a newer"));

    await act(async () => replies[0](conflict()));
    expect(save).toHaveBeenLastCalledWith("a newer", "a-1");
  });

  it("keeps its own hash when it read another version than the save replaced", async () => {
    const { save, replies } = held();
    const visited = visits(save);
    await act(async () => visited.hook.result.current.commit("a text"));
    visited.hook.rerender({ noteKey: "b#1", noteId: "b", baseHash: "b-1" });
    visited.hook.rerender({ noteKey: "a#2", noteId: "a", baseHash: "a-9" });
    await act(async () => visited.hook.result.current.commitRestored("a text"));

    await act(async () => replies[0]({ content_hash: "a-2" }));
    expect(save).toHaveBeenLastCalledWith("a text", "a-9");
    expect(visited.hook.result.current.isOwnWrite("a-2")).toBe(false);
  });

  it("waits for the save even when another note saved in between", async () => {
    const { save, replies } = held();
    const visited = visits(save);
    await act(async () => visited.hook.result.current.commit("a text"));
    visited.hook.rerender({ noteKey: "b#1", noteId: "b", baseHash: "b-1" });
    await act(async () => visited.hook.result.current.commit("b text"));
    visited.hook.rerender({ noteKey: "a#2", noteId: "a", baseHash: "a-1" });
    await act(async () =>
      visited.hook.result.current.commitRestored("a newer"),
    );
    expect(save).toHaveBeenCalledTimes(2);

    await act(async () => replies[1]({ content_hash: "b-2" }));
    expect(save).toHaveBeenCalledTimes(2);
    await act(async () => replies[0]({ content_hash: "a-2" }));
    expect(save).toHaveBeenLastCalledWith("a newer", "a-2");
  });

  it("carries on from a save that answered before the return wrote", async () => {
    const { save, replies } = held();
    const { hook, saved } = await returnMidSave(save);
    await act(async () => replies[0]({ content_hash: "a-2" }));
    expect(hook.result.current.status).toBe("idle");

    await act(async () => hook.result.current.commitRestored("a text"));
    expect(save).toHaveBeenCalledTimes(1);
    expect(hook.result.current.status).toBe("saved");
    expect(saved.mock.calls.map((call) => call[0])).toEqual(["a#0", "a#2"]);
    await act(async () => hook.result.current.commit("a newer"));
    expect(save).toHaveBeenLastCalledWith("a newer", "a-2");
  });

  it("carries on from a save that answered only once", async () => {
    const { save, replies } = held();
    const visited = await returnMidSave(save);
    await act(async () => replies[0]({ content_hash: "a-2" }));
    await act(async () => visited.hook.result.current.commitRestored("a text"));
    expect(save).toHaveBeenCalledTimes(1);
    visited.hook.rerender({ noteKey: "b#3", noteId: "b", baseHash: "b-1" });
    visited.hook.rerender({ noteKey: "a#4", noteId: "a", baseHash: "a-1" });
    await act(async () =>
      visited.hook.result.current.commitRestored("a newer"),
    );
    expect(save).toHaveBeenLastCalledWith("a newer", "a-1");
  });

  it("does not take over the save's hash for text that was not restored", async () => {
    const { save, replies } = held();
    const { hook, saved } = await returnMidSave(save);
    // Typed on the note as read, without the edit the earlier save carries.
    await act(async () => hook.result.current.commit("a typed afresh"));
    expect(save).toHaveBeenCalledTimes(1);

    await act(async () => replies[0]({ content_hash: "a-2" }));
    expect(save).toHaveBeenLastCalledWith("a typed afresh", "a-1");
    expect(saved.mock.calls.map((call) => call[0])).toEqual(["a#0"]);
  });

  it("does not take over a save of another note id", async () => {
    const { save } = held();
    const visited = visits(save);
    await act(async () => visited.hook.result.current.commit("a text"));
    visited.hook.rerender({ noteKey: "a#1", noteId: "a~1", baseHash: "a-1" });
    await act(async () => visited.hook.result.current.commitRestored("a text"));
    expect(save).toHaveBeenCalledTimes(2);
  });
});

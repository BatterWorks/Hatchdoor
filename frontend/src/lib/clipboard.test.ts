import { afterEach, describe, expect, it, vi } from "vitest";

import { copyText } from "./clipboard";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("copyText", () => {
  it("uses a Range-based fallback so iOS WebKit can copy outside secure contexts", async () => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: undefined,
    });
    const removeAllRanges = vi.fn();
    const addRange = vi.fn();
    vi.spyOn(window, "getSelection").mockReturnValue({
      removeAllRanges,
      addRange,
    } as unknown as Selection);
    const execCommand = vi.fn((command: string) => command === "copy");
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: execCommand,
    });

    await expect(copyText("https://hatchdoor.local/n/home")).resolves.toBe(
      true,
    );

    expect(removeAllRanges).toHaveBeenCalled();
    expect(addRange).toHaveBeenCalledWith(expect.any(Range));
    expect(execCommand).toHaveBeenCalledWith("copy");
  });
});

import { act, cleanup, renderHook } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

import { apiFetch } from "../api/api";
import { healthyVault } from "../test/fixtures/vaults";
import { useNoteActions } from "./useNoteActions";

vi.mock("../api/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../api/api")>()),
  apiFetch: vi.fn(),
}));

const mockedApiFetch = vi.mocked(apiFetch);

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function wrapper({ children }: { children: ReactNode }) {
  return <MemoryRouter>{children}</MemoryRouter>;
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function demoReadOnlyResponse(): Response {
  return jsonResponse(
    {
      code: "demo_read_only",
      message:
        "This is a public read-only demo instance; mutations and Vault-control operations are disabled.",
      retryable: false,
    },
    403,
  );
}

describe("useNoteActions — demo_read_only refusal (#152)", () => {
  it("closes the dialog and defers to onDemoRefusal instead of setting an inline error", async () => {
    mockedApiFetch.mockResolvedValueOnce(demoReadOnlyResponse());
    const onDemoRefusal = vi.fn().mockReturnValue(true);
    const vaults = [healthyVault("Alpha")];

    const { result } = renderHook(
      () =>
        useNoteActions({
          activeNote: null,
          vaults,
          refreshVault: vi.fn().mockResolvedValue(undefined),
          setWriteNotice: vi.fn(),
          onDemoRefusal,
        }),
      { wrapper },
    );

    act(() => result.current.openCreateDialog(""));
    expect(result.current.noteActionDialog).toBe("create");

    await act(async () => {
      await result.current.handleCreateNote("Home.md", "# Home");
    });

    expect(onDemoRefusal).toHaveBeenCalledTimes(1);
    expect(result.current.noteActionDialog).toBeNull();
    expect(result.current.noteActionError).toBeNull();
  });

  it("falls back to the ordinary inline error when onDemoRefusal declines the error", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse(
        { code: "internal_error", message: "boom", retryable: false },
        500,
      ),
    );
    const onDemoRefusal = vi.fn().mockReturnValue(false);
    const vaults = [healthyVault("Alpha")];

    const { result } = renderHook(
      () =>
        useNoteActions({
          activeNote: null,
          vaults,
          refreshVault: vi.fn().mockResolvedValue(undefined),
          setWriteNotice: vi.fn(),
          onDemoRefusal,
        }),
      { wrapper },
    );

    act(() => result.current.openCreateDialog(""));

    await act(async () => {
      await result.current.handleCreateNote("Home.md", "# Home");
    });

    expect(onDemoRefusal).toHaveBeenCalledTimes(1);
    expect(result.current.noteActionDialog).toBe("create");
    expect(result.current.noteActionError).toBe("boom");
  });
});

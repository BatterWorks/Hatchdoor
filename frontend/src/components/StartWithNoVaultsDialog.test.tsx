import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { clearToken } from "../api/api";
import { StartWithNoVaultsDialog } from "./StartWithNoVaultsDialog";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  clearToken();
});

describe("StartWithNoVaultsDialog", () => {
  it("states the documented consequences before confirming", () => {
    render(
      <StartWithNoVaultsDialog onClose={() => {}} onConfirmed={() => {}} />,
    );

    expect(
      screen.getByText(
        "Notes and history are untouched, old settings will be ignored from now on, and the folder must be added by hand.",
      ),
    ).toBeVisible();
  });

  it("confirms by posting confirm: true and calls onConfirmed", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(
        new Response(JSON.stringify({ vaults: [] }), { status: 200 }),
      );
    const onConfirmed = vi.fn();

    render(
      <StartWithNoVaultsDialog onClose={() => {}} onConfirmed={onConfirmed} />,
    );
    await act(async () => {
      screen.getByRole("button", { name: "Start with no Vaults" }).click();
    });

    const [path, init] = fetchMock.mock.calls.at(-1) ?? [];
    expect(path).toBe("/api/v1/vaults/start-with-no-vaults");
    expect(init).toEqual(
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ confirm: true }),
      }),
    );
    expect(onConfirmed).toHaveBeenCalledTimes(1);
  });

  it("shows the server's message and stays open on failure", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          code: "registry_revision_conflict",
          message: "expected registry revision 0, current revision is 1",
        }),
        { status: 409 },
      ),
    );
    const onConfirmed = vi.fn();

    render(
      <StartWithNoVaultsDialog onClose={() => {}} onConfirmed={onConfirmed} />,
    );
    await act(async () => {
      screen.getByRole("button", { name: "Start with no Vaults" }).click();
    });

    expect(
      screen.getByText("expected registry revision 0, current revision is 1"),
    ).toBeVisible();
    expect(onConfirmed).not.toHaveBeenCalled();
  });

  it("closes on Cancel and on a backdrop click", async () => {
    const onClose = vi.fn();
    render(
      <StartWithNoVaultsDialog onClose={onClose} onConfirmed={() => {}} />,
    );

    screen.getByRole("button", { name: "Cancel" }).click();
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

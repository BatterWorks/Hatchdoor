import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { StartupGate } from "./StartupGate";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function statusResponse(body: object) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

describe("StartupGate", () => {
  it("shows understandable indexing progress without mounting the vault", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      statusResponse({
        state: "indexing",
        notes_completed: 12,
        notes_total: 40,
        chunks_completed: 18,
        chunks_total: 70,
        tokens_completed: 4_000,
        tokens_total: 20_000,
        percent: 20,
        eta_seconds: 80,
      }),
    );

    render(
      <StartupGate>
        <div>Private vault</div>
      </StartupGate>,
    );

    expect(
      await screen.findByRole("heading", { name: "Preparing your vault" }),
    ).toBeVisible();
    expect(screen.queryByText("Private vault")).not.toBeInTheDocument();
    expect(screen.getByText("12 of 40 notes")).toBeVisible();
    expect(screen.getByText("18 of 70 chunks")).toBeVisible();
    expect(screen.getByText("About 1 min remaining")).toBeVisible();
    expect(screen.getByRole("progressbar")).toHaveAttribute(
      "aria-valuenow",
      "20",
    );
  });

  it("mounts the vault immediately when indexing is ready", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      statusResponse({ state: "ready" }),
    );

    render(
      <StartupGate>
        <div>Private vault</div>
      </StartupGate>,
    );

    expect(await screen.findByText("Private vault")).toBeVisible();
  });

  it("polls until the backend becomes ready", async () => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(
        statusResponse({
          state: "indexing",
          notes_completed: 1,
          notes_total: 2,
          chunks_completed: 1,
          chunks_total: 2,
          tokens_completed: 10,
          tokens_total: 20,
          percent: 50,
          eta_seconds: 2,
        }),
      )
      .mockResolvedValue(statusResponse({ state: "ready" }));

    render(
      <StartupGate>
        <div>Private vault</div>
      </StartupGate>,
    );
    await act(async () => {});
    expect(screen.queryByText("Private vault")).not.toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });

    expect(screen.getByText("Private vault")).toBeVisible();
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("shows a safe error when indexing fails", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      statusResponse({
        state: "failed",
        message: "Indexing could not be completed.",
      }),
    );

    render(
      <StartupGate>
        <div>Private vault</div>
      </StartupGate>,
    );

    expect(
      await screen.findByRole("heading", { name: "Vault unavailable" }),
    ).toBeVisible();
    expect(screen.queryByText("Private vault")).not.toBeInTheDocument();
  });
});

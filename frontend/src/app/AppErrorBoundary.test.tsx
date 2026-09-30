import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppErrorBoundary } from "./AppErrorBoundary";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function Throws(): never {
  throw new Error("render fault");
}

describe("AppErrorBoundary (#339)", () => {
  it("renders a message and a reload action instead of a blank page", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    render(
      <AppErrorBoundary>
        <Throws />
      </AppErrorBoundary>,
    );

    expect(
      screen.getByRole("heading", { name: "Something Went Wrong" }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Reload" })).toBeVisible();
  });

  it("renders its children when nothing throws", () => {
    render(
      <AppErrorBoundary>
        <p>Workspace</p>
      </AppErrorBoundary>,
    );

    expect(screen.getByText("Workspace")).toBeVisible();
  });
});

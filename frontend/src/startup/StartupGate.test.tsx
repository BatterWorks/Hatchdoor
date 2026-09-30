import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { StartupGate } from "./StartupGate";
import type { StartupStatus } from "./useStartupStatus";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function renderGate(
  overrides: Partial<Parameters<typeof StartupGate>[0]> = {},
) {
  const props: Parameters<typeof StartupGate>[0] = {
    status: null,
    connectionIssue: false,
    hasSteppedPastGate: false,
    discoveryLoading: false,
    hasRegistryRecovery: false,
    hasNoVaults: false,
    onAcceptGemma: vi.fn(),
    onDeclineGemma: vi.fn(),
    children: <div>Private vault</div>,
    ...overrides,
  };
  render(<StartupGate {...props} />);
  return props;
}

describe("StartupGate", () => {
  it("holds the workspace unmounted until the first status answer lands (#339)", () => {
    // Mounting it now would only have it torn down a fetch later if that
    // answer is a model step, discarding its state and fetches.
    renderGate({ status: null });

    expect(screen.queryByText("Private vault")).not.toBeInTheDocument();
  });

  it("releases the hold when the first status poll fails, so a missing answer never locks anyone out", () => {
    renderGate({ status: null, connectionIssue: true });

    expect(screen.getByText("Private vault")).toBeVisible();
  });

  it("never holds a zero-Vault or broken-registry workspace, which never polls", () => {
    renderGate({ status: null, hasNoVaults: true });
    expect(screen.getByText("Private vault")).toBeVisible();

    cleanup();
    renderGate({ status: null, hasRegistryRecovery: true });
    expect(screen.getByText("Private vault")).toBeVisible();
  });

  it("offers the model choice again when terms are required after the gate has stepped aside (#339)", () => {
    // Nothing else in the app can accept or decline Gemma: a latched client
    // must not be left with search silently dead.
    renderGate({
      status: { state: "terms_required" },
      hasSteppedPastGate: true,
    });

    expect(screen.queryByText("Private vault")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Accept terms and set up Gemma" }),
    ).toBeVisible();
  });

  it("does not re-gate a latched demo visitor on terms, since the demo server refuses the choice", () => {
    renderGate({
      status: { state: "terms_required" },
      hasSteppedPastGate: true,
      demoMode: true,
    });

    expect(screen.getByText("Private vault")).toBeVisible();
  });

  it("never shows a fresh demo visitor the operator model choice, since the demo server refuses it", () => {
    renderGate({
      status: { state: "terms_required" },
      hasSteppedPastGate: false,
      demoMode: true,
    });

    expect(screen.getByText("Private vault")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Accept terms and set up Gemma" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Use Nomic instead" }),
    ).not.toBeInTheDocument();
  });

  it("mounts the vault immediately once the gate has stepped aside, even mid-scan", () => {
    renderGate({
      status: { state: "scanning" },
      hasSteppedPastGate: true,
    });

    expect(screen.getByText("Private vault")).toBeVisible();
  });

  it("mounts the vault immediately once indexing is ready", () => {
    renderGate({ status: { state: "ready" }, hasSteppedPastGate: true });

    expect(screen.getByText("Private vault")).toBeVisible();
  });

  it("mounts the vault immediately on a failed model download rather than blocking on it (#150)", () => {
    renderGate({
      status: { state: "failed", message: "model download failed" },
      hasSteppedPastGate: true,
    });

    expect(screen.getByText("Private vault")).toBeVisible();
  });

  it("explains Gemma terms before any model download and calls onAcceptGemma/onDeclineGemma", () => {
    const onAcceptGemma = vi.fn();
    const onDeclineGemma = vi.fn();
    renderGate({
      status: { state: "terms_required" },
      onAcceptGemma,
      onDeclineGemma,
    });

    expect(
      screen.getByRole("heading", { name: "Set up multilingual search" }),
    ).toBeVisible();
    expect(
      screen.getByText(/does not change ownership of your vault/i),
    ).toBeVisible();
    expect(
      screen.getByRole("link", { name: "Read Gemma Terms" }),
    ).toHaveAttribute("href", "https://ai.google.dev/gemma/terms");

    screen
      .getByRole("button", { name: "Accept terms and set up Gemma" })
      .click();
    expect(onAcceptGemma).toHaveBeenCalledTimes(1);

    screen.getByRole("button", { name: "Use Nomic instead" }).click();
    expect(onDeclineGemma).toHaveBeenCalledTimes(1);
  });

  it("shows model download progress while downloading, before the gate has stepped aside", () => {
    renderGate({
      status: {
        state: "downloading",
        model: "EmbeddingGemma 300M Q4",
        downloaded_bytes: 25,
        total_bytes: 100,
        percent: 25,
      },
    });

    expect(screen.queryByText("Private vault")).not.toBeInTheDocument();
    expect(
      screen.getByText(/Downloading EmbeddingGemma 300M Q4/),
    ).toBeVisible();
    expect(screen.getByRole("progressbar")).toHaveAttribute(
      "aria-valuenow",
      "25",
    );
  });

  it("stays stepped aside on a later downloading answer once it has stepped aside once (a retry never reopens the gate)", () => {
    // #150: the gate is seen at most once per session — a retry after a
    // failed model download must not re-block the workspace even though
    // `state` genuinely goes back to "downloading".
    renderGate({
      status: { state: "downloading", percent: 10 },
      hasSteppedPastGate: true,
    });

    expect(screen.getByText("Private vault")).toBeVisible();
  });

  it("never renders for scanning, indexing, zero Vaults, or any registry condition — those are not states this component ever sees gated", () => {
    const scanning: StartupStatus = { state: "scanning" };
    renderGate({ status: scanning, hasSteppedPastGate: true });
    expect(screen.getByText("Private vault")).toBeVisible();
  });

  it("waits for registry discovery and lets a recovery state override a model gate", () => {
    const modelSetup = { status: { state: "terms_required" as const } };
    renderGate({ ...modelSetup, discoveryLoading: true });
    // Undecided rather than gated or mounted: discovery may yet turn up a
    // recovery state that overrides the model step.
    expect(screen.queryByText("Private vault")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Set up multilingual search" }),
    ).not.toBeInTheDocument();

    cleanup();
    renderGate({ ...modelSetup, hasRegistryRecovery: true });
    expect(screen.getByText("Private vault")).toBeVisible();
  });

  it("lets a resolved zero-Vault workspace override a model gate", () => {
    renderGate({
      status: { state: "terms_required" },
      hasNoVaults: true,
    });

    expect(screen.getByText("Private vault")).toBeVisible();
  });
});

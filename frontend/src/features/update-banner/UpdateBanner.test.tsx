import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi, type Mock } from "vitest";

import { HelpContext, type HelpApi } from "../help/useHelp";
import { UpdateBanner } from "./UpdateBanner";
import { DISMISSED_VERSION_KEY, type UpdateCheckStatus } from "./updateBanner";

const RELEASE_URL =
  "https://github.com/BatterWorks/Hatchdoor/releases/tag/v2.9.0";

function status(overrides: Partial<UpdateCheckStatus> = {}): UpdateCheckStatus {
  return {
    enabled: true,
    checked_at: "2026-10-03T09:00:00Z",
    update_available: { version: "2.9.0", release_url: RELEASE_URL },
    ...overrides,
  };
}

function serve(update_check: UpdateCheckStatus | null, httpStatus = 200): Mock {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(
      async () =>
        new Response(
          update_check ? JSON.stringify({ settings: [], update_check }) : "no",
          { status: httpStatus },
        ),
    ) as unknown as Mock;
}

function renderBanner() {
  const help: HelpApi = {
    openHelp: vi.fn(),
    closeHelp: vi.fn(),
    isOpen: false,
  };
  render(
    <HelpContext.Provider value={help}>
      <UpdateBanner />
    </HelpContext.Provider>,
  );
  return help;
}

async function expectNothingShown(fetchMock: Mock) {
  await waitFor(() => expect(fetchMock).toHaveBeenCalled());
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("Update banner (#425)", () => {
  it("offers a newer release with both links", async () => {
    const fetchMock = serve(status());
    const help = renderBanner();

    expect(
      await screen.findByText(/Hatchdoor 2\.9\.0 is available/),
    ).toBeInTheDocument();
    expect(fetchMock.mock.calls[0][0]).toBe("/api/settings");
    const whatsNew = screen.getByRole("link", { name: "What's new" });
    expect(whatsNew).toHaveAttribute("href", RELEASE_URL);
    expect(whatsNew).toHaveAttribute("target", "_blank");
    expect(whatsNew).toHaveAttribute("rel", "noopener noreferrer");

    fireEvent.click(screen.getByRole("button", { name: "How to upgrade" }));
    expect(help.openHelp).toHaveBeenCalledWith(
      "guides/how-to-upgrade-hatchdoor",
    );
  });

  it("stays dismissed for that version in this browser, and returns for the next", async () => {
    const fetchMock = serve(status());
    renderBanner();
    fireEvent.click(
      await screen.findByRole("button", { name: "Dismiss update notice" }),
    );
    expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
    expect(localStorage.getItem(DISMISSED_VERSION_KEY)).toBe("2.9.0");

    cleanup();
    renderBanner();
    await expectNothingShown(fetchMock);

    cleanup();
    vi.restoreAllMocks();
    serve(
      status({
        update_available: {
          version: "2.10.0",
          release_url: RELEASE_URL.replace("2.9.0", "2.10.0"),
        },
      }),
    );
    renderBanner();
    expect(
      await screen.findByText(/Hatchdoor 2\.10\.0 is available/),
    ).toBeInTheDocument();
  });

  it("shows nothing when nothing newer was found", async () => {
    const fetchMock = serve(status({ update_available: null }));
    renderBanner();
    await expectNothingShown(fetchMock);
  });

  it("shows nothing when the check is off", async () => {
    const fetchMock = serve(status({ enabled: false, update_available: null }));
    renderBanner();
    await expectNothingShown(fetchMock);
  });

  it("shows nothing when the settings cannot be read", async () => {
    const fetchMock = serve(null, 500);
    renderBanner();
    await expectNothingShown(fetchMock);
  });

  it("still dismisses when storage is blocked", async () => {
    serve(status());
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    renderBanner();
    fireEvent.click(
      await screen.findByRole("button", { name: "Dismiss update notice" }),
    );
    expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
  });
});

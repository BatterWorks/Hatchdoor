import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi, type Mock } from "vitest";

import { HelpContext, type HelpApi } from "../help/useHelp";
import { WhatsNew } from "./WhatsNew";
import { SEEN_VERSION_KEY, type WhatsNewResponse } from "./whatsNew";

const INSTALL = {
  label: "Install Hatchdoor with Docker Compose",
  page: "get-started/install-hatchdoor-with-docker-compose",
  heading: "upgrade",
};

const R280 = {
  version: "2.8.0",
  date: "2026-10-20",
  highlights: [
    {
      text: "Installs on 2.4.x or earlier must upgrade to a 2.5.0 to 2.7.x release first.",
      action_needed: true,
      link: INSTALL,
    },
    {
      text: "Help opens **beside** what you are doing.",
      action_needed: false,
      link: {
        label: "Browse and review through the Web UI",
        page: "get-started/browse-and-review-through-the-web-ui",
        heading: null,
      },
    },
    { text: "A setup checklist.", action_needed: false, link: null },
    { text: "An update check.", action_needed: false, link: null },
  ],
};

const R270 = {
  version: "2.7.0",
  date: "2026-10-02",
  highlights: [
    {
      text: "Git-backed Vaults need a token that can push.",
      action_needed: true,
      link: null,
    },
    { text: "Saved queries render live.", action_needed: false, link: null },
    { text: "Titles rank higher.", action_needed: false, link: null },
  ],
};

function upgrade(overrides: Partial<WhatsNewResponse> = {}): WhatsNewResponse {
  return {
    version: "2.8.0",
    previous_version: "2.7.0",
    fresh_install: false,
    releases: [R280],
    ...overrides,
  };
}

function serve(body: WhatsNewResponse | null, status = 200): Mock {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(
      async () =>
        new Response(body ? JSON.stringify(body) : "refused", { status }),
    ) as unknown as Mock;
}

const USAGE_REPORT_LINK = {
  label: "Usage report reference",
  page: "reference/usage-report-reference",
  heading: null,
};

const R281 = {
  version: "2.8.1",
  date: "2026-10-12",
  highlights: [
    {
      text: "The optional usage report.",
      action_needed: false,
      link: USAGE_REPORT_LINK,
    },
    {
      text: "Published images say what they are.",
      action_needed: false,
      link: null,
    },
    {
      text: "The agent shows the web token.",
      action_needed: false,
      link: null,
    },
  ],
};

/** The 2.8.0 line about the usage report, which links to the same page. */
const R280_WITH_USAGE = {
  ...R280,
  highlights: [
    ...R280.highlights,
    {
      text: "An optional usage report.",
      action_needed: false,
      link: USAGE_REPORT_LINK,
    },
  ],
};

type UsageSetting = { value: string; locked?: string };

/** What's new plus the settings the usage report switch reads and saves.
 * `save` answers the PATCH: the saved setting, or a refusal. */
function serveWithSettings(
  body: WhatsNewResponse,
  setting: UsageSetting | null,
  save: UsageSetting | { error: string } = { value: "true" },
): Mock {
  const settings = (usage: UsageSetting) => ({
    settings: [
      { key: "HATCHDOOR_MCP_ENABLED", value: "true", locked: null },
      { key: "HATCHDOOR_USAGE_REPORT_ENABLED", locked: null, ...usage },
    ],
  });
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input, init) => {
      if (input === "/api/v1/whats-new") {
        return new Response(JSON.stringify(body));
      }
      if (init?.method === "PATCH") {
        return "error" in save
          ? new Response(JSON.stringify(save), { status: 400 })
          : new Response(JSON.stringify(settings(save)));
      }
      return setting
        ? new Response(JSON.stringify(settings(setting)))
        : new Response("refused", { status: 500 });
    }) as unknown as Mock;
}

function settingsCalls(fetchMock: Mock, method?: string) {
  return fetchMock.mock.calls.filter(
    ([input, init]) =>
      input === "/api/settings" &&
      (init as RequestInit | undefined)?.method === method,
  );
}

function renderWhatsNew(isOpen = false) {
  const help: HelpApi = {
    openHelp: vi.fn(),
    closeHelp: vi.fn(),
    isOpen,
  };
  const view = render(
    <HelpContext.Provider value={help}>
      <WhatsNew />
    </HelpContext.Provider>,
  );
  return { help, ...view };
}

/** Let the fetch settle, then confirm nothing showed. */
async function expectNothingShown(fetchMock: Mock, fetched = true) {
  if (fetched) {
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
  }
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("What's new pop-up (#418)", () => {
  it("shows an upgrade once per browser, and again in a fresh one", async () => {
    const fetchMock = serve(upgrade());
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog", {
      name: "Hatchdoor 2.8.0",
    });
    expect(fetchMock.mock.calls[0][0]).toBe("/api/v1/whats-new");
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveFocus();
    expect(within(dialog).getByText("beside").tagName).toBe("STRONG");

    fireEvent.click(within(dialog).getByRole("button", { name: "Got it" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(localStorage.getItem(SEEN_VERSION_KEY)).toBe("2.8.0");

    // The same browser, next visit: nothing.
    cleanup();
    renderWhatsNew();
    await expectNothingShown(fetchMock);

    // A fresh browser: shown again.
    cleanup();
    localStorage.clear();
    renderWhatsNew();
    expect(
      await screen.findByRole("dialog", { name: "Hatchdoor 2.8.0" }),
    ).toBeInTheDocument();
  });

  it("dismisses on Escape, remembering the running version's base", async () => {
    serve(upgrade({ version: "2.8.0 (dev abc123)" }));
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog", {
      name: "Hatchdoor 2.8.0",
    });
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(localStorage.getItem(SEEN_VERSION_KEY)).toBe("2.8.0");
  });

  it("stacks skipped releases newest first, action-needed items pinned on top", async () => {
    serve(upgrade({ previous_version: "2.6.0", releases: [R270, R280] }));
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog", {
      name: "Hatchdoor 2.8.0",
    });
    expect(
      within(dialog).getByText(/You skipped 1 release since/),
    ).toBeInTheDocument();

    const action = within(dialog).getByRole("region", {
      name: "Action needed",
    });
    const pinned = within(action).getAllByRole("listitem");
    expect(pinned.map((item) => item.textContent)).toEqual([
      expect.stringMatching(/^v2\.8\.0 Installs on 2\.4\.x/),
      expect.stringMatching(/^v2\.7\.0 Git-backed Vaults/),
    ]);

    const sections = within(dialog)
      .getAllByRole("region")
      .map((region) => region.getAttribute("aria-label"));
    expect(sections).toEqual([
      "Action needed",
      "Version 2.8.0",
      "Version 2.7.0",
    ]);

    // Action-needed lines appear only in the pinned box.
    const v280 = within(dialog).getByRole("region", { name: "Version 2.8.0" });
    expect(within(v280).queryByText(/Installs on 2\.4\.x/)).toBeNull();
    expect(within(v280).getAllByRole("listitem")).toHaveLength(3);
  });

  it("opens Help at a highlight's page and heading, and the changelog page", async () => {
    serve(upgrade());
    const { help } = renderWhatsNew();

    const dialog = await screen.findByRole("dialog", {
      name: "Hatchdoor 2.8.0",
    });
    fireEvent.click(
      within(dialog).getByRole("button", {
        name: "Install Hatchdoor with Docker Compose",
      }),
    );
    expect(help.openHelp).toHaveBeenLastCalledWith(
      "get-started/install-hatchdoor-with-docker-compose",
      "upgrade",
    );

    fireEvent.click(
      within(dialog).getByRole("button", {
        name: "Browse and review through the Web UI",
      }),
    );
    expect(help.openHelp).toHaveBeenLastCalledWith(
      "get-started/browse-and-review-through-the-web-ui",
      undefined,
    );

    fireEvent.click(
      within(dialog).getByRole("button", { name: "Full changelog" }),
    );
    expect(help.openHelp).toHaveBeenLastCalledWith("whats-new");

    // Opening Help leaves the pop-up up and unseen.
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(localStorage.getItem(SEEN_VERSION_KEY)).toBeNull();
  });

  it("moves beside Help while Help is open, where Escape closes Help first", async () => {
    serve(upgrade());
    const { help } = renderWhatsNew(true);

    const dialog = await screen.findByRole("dialog", {
      name: "Hatchdoor 2.8.0",
    });
    expect(dialog.parentElement).toHaveClass("is-beside-help");
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(help.closeHelp).toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(localStorage.getItem(SEEN_VERSION_KEY)).toBeNull();
  });

  it("keeps Tab inside the dialog, unless Help is open beside it", async () => {
    serve(upgrade());
    const outside = document.createElement("button");
    outside.textContent = "Behind the dialog";
    document.body.append(outside);
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog", {
      name: "Hatchdoor 2.8.0",
    });
    const buttons = within(dialog).getAllByRole("button");
    const first = buttons[0];
    const last = within(dialog).getByRole("button", { name: "Got it" });

    last.focus();
    fireEvent.keyDown(last, { key: "Tab" });
    expect(first).toHaveFocus();

    fireEvent.keyDown(first, { key: "Tab", shiftKey: true });
    expect(last).toHaveFocus();

    outside.focus();
    fireEvent.keyDown(outside, { key: "Tab" });
    expect(first).toHaveFocus();

    cleanup();
    serve(upgrade());
    renderWhatsNew(true);
    const besideHelp = await screen.findByRole("dialog", {
      name: "Hatchdoor 2.8.0",
    });
    const gotIt = within(besideHelp).getByRole("button", { name: "Got it" });
    gotIt.focus();
    const event = fireEvent.keyDown(gotIt, { key: "Tab" });
    expect(event).toBe(true); // not prevented: Tab may move on into Help
    outside.remove();
  });

  it("shows nothing on a fresh install", async () => {
    const fetchMock = serve(
      upgrade({ previous_version: null, fresh_install: true, releases: [] }),
    );
    renderWhatsNew();
    await expectNothingShown(fetchMock);
  });

  it("shows nothing when nothing is new", async () => {
    const fetchMock = serve(upgrade({ releases: [] }));
    renderWhatsNew();
    await expectNothingShown(fetchMock);
  });

  it("shows nothing when the server refuses, as it does in demo mode", async () => {
    const fetchMock = serve(null, 403);
    renderWhatsNew();
    await expectNothingShown(fetchMock);
  });

  it("shows nothing and does not ask when storage is blocked", async () => {
    const fetchMock = serve(upgrade());
    for (const method of ["getItem", "setItem"] as const) {
      vi.spyOn(Storage.prototype, method).mockImplementation(() => {
        throw new DOMException("blocked", "SecurityError");
      });
    }
    renderWhatsNew();
    await expectNothingShown(fetchMock, false);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe("the usage report switch in What's new (ADR-48)", () => {
  const to281 = (releases = [R281]) =>
    upgrade({ version: "2.8.1", previous_version: "2.8.0", releases });

  it("sits on the 2.8.1 usage report line, off, and turns the report on with one click", async () => {
    const fetchMock = serveWithSettings(to281(), { value: "false" });
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog", {
      name: "Hatchdoor 2.8.1",
    });
    const toggle = await within(dialog).findByRole("button", {
      name: "Send a usage report",
    });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(toggle).toHaveTextContent("Off");
    // On its own line and no other.
    expect(toggle.closest("li")).toHaveTextContent(
      "The optional usage report.",
    );
    expect(
      within(dialog).getAllByRole("button", { name: "Send a usage report" }),
    ).toHaveLength(1);
    // Looking changes nothing.
    expect(settingsCalls(fetchMock, "PATCH")).toHaveLength(0);

    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "true"));
    expect(toggle).toHaveTextContent("On");
    const saves = settingsCalls(fetchMock, "PATCH");
    expect(saves).toHaveLength(1);
    expect(JSON.parse((saves[0][1] as RequestInit).body as string)).toEqual({
      updates: { HATCHDOOR_USAGE_REPORT_ENABLED: "true" },
      confirm: [],
    });
    // The pop-up stays, and the line still links to the manual page.
    expect(dialog).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Usage report reference" }),
    ).toBeInTheDocument();
  });

  it("shows a report that is already on, and turns it off again", async () => {
    const fetchMock = serveWithSettings(
      to281(),
      { value: "true" },
      {
        value: "false",
      },
    );
    renderWhatsNew();

    const toggle = await screen.findByRole("button", {
      name: "Send a usage report",
    });
    expect(toggle).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(toggle);
    await waitFor(() =>
      expect(toggle).toHaveAttribute("aria-pressed", "false"),
    );
    expect(
      JSON.parse(
        (settingsCalls(fetchMock, "PATCH")[0][1] as RequestInit).body as string,
      ).updates,
    ).toEqual({ HATCHDOOR_USAGE_REPORT_ENABLED: "false" });
  });

  it("only reports a value the configuration file holds", async () => {
    serveWithSettings(to281(), { value: "true", locked: "environment" });
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog");
    expect(
      await within(dialog).findByText(
        "On, set in the server's configuration file",
      ),
    ).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: "Send a usage report" }),
    ).not.toBeInTheDocument();
  });

  it("says so when the save is refused, and stays off", async () => {
    serveWithSettings(
      to281(),
      { value: "false" },
      {
        error: "Settings are read-only here.",
      },
    );
    renderWhatsNew();

    const toggle = await screen.findByRole("button", {
      name: "Send a usage report",
    });
    fireEvent.click(toggle);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Settings are read-only here.",
    );
    expect(toggle).toHaveAttribute("aria-pressed", "false");
  });

  it("shows the line without a switch when the setting cannot be read", async () => {
    const fetchMock = serveWithSettings(to281(), null);
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog");
    await waitFor(() => expect(settingsCalls(fetchMock)).toHaveLength(1));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      within(dialog).getByText("The optional usage report."),
    ).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: "Send a usage report" }),
    ).not.toBeInTheDocument();
  });

  it("is on the 2.8.1 line only when an upgrade also shows the 2.8.0 line", async () => {
    serveWithSettings(
      upgrade({
        version: "2.8.1",
        previous_version: "2.7.0",
        releases: [R281, R280_WITH_USAGE],
      }),
      { value: "false" },
    );
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog");
    const toggles = await within(dialog).findAllByRole("button", {
      name: "Send a usage report",
    });
    expect(toggles).toHaveLength(1);
    expect(toggles[0].closest("section")).toHaveAccessibleName("Version 2.8.1");
  });

  it("is absent, and the settings are never read, for any other release", async () => {
    const fetchMock = serveWithSettings(
      upgrade({ releases: [R280_WITH_USAGE] }),
      { value: "false" },
    );
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog", {
      name: "Hatchdoor 2.8.0",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      within(dialog).queryByRole("button", { name: "Send a usage report" }),
    ).not.toBeInTheDocument();
    expect(settingsCalls(fetchMock)).toHaveLength(0);
  });

  it("stays enabled while it saves, so it keeps its focus", async () => {
    serveWithSettings(to281(), { value: "false" });
    renderWhatsNew();

    const dialog = await screen.findByRole("dialog");
    const toggle = await within(dialog).findByRole("button", {
      name: "Send a usage report",
    });
    fireEvent.click(toggle);
    // Not disabled while it saves, so it keeps the focus it was given.
    expect(toggle).not.toBeDisabled();
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "true"));
  });
});

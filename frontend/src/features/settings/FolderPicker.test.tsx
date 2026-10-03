import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import type { FolderListing, FolderListingEntry } from "../../types";
import { FolderPicker } from "./FolderPicker";
import { mountFolderPath, noteCountLabel } from "./vaultCreation";

vi.mock("../../api/api", () => ({ apiFetch: vi.fn() }));
const mockedApiFetch = vi.mocked(apiFetch);

const json = (body: unknown, init?: ResponseInit) =>
  new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
    ...init,
  });

function folder(
  name: string,
  count: number,
  extra: Partial<FolderListingEntry> = {},
  parent = "",
): FolderListingEntry {
  return {
    name,
    path: parent ? `${parent}/${name}` : name,
    markdown: { count, at_least: false },
    vault: null,
    has_subfolders: false,
    ...extra,
  };
}

function listing(
  path: string,
  folders: FolderListingEntry[],
  extra: Partial<FolderListing> = {},
): FolderListing {
  return {
    root: "/data/vault",
    root_found: true,
    path,
    markdown: {
      count: folders.reduce((sum, entry) => sum + entry.markdown.count, 0),
      at_least: false,
    },
    vault: null,
    folders,
    skipped_invalid_names: 0,
    ...extra,
  };
}

const MOUNT = listing("", [
  folder("Archive", 10000, {
    markdown: { count: 10000, at_least: true },
    has_subfolders: true,
  }),
  folder("Recipes", 86, { vault: { vault_id: "v-1", name: "Kitchen" } }),
  folder("Work", 1280, { has_subfolders: true }),
]);
const WORK = listing("Work", [
  folder("Clients", 310, {}, "Work"),
  folder("Projects", 902, {}, "Work"),
]);

/** Answer each `GET /api/v1/folders` from `answers`, keyed by `path`. */
function serve(answers: Record<string, FolderListing | Response>) {
  mockedApiFetch.mockImplementation(async (input) => {
    const url = new URL(String(input), "http://hatchdoor.test");
    if (url.pathname !== "/api/v1/folders")
      throw new Error(`Unexpected API request: ${url}`);
    const answer = answers[url.searchParams.get("path") ?? ""];
    if (!answer) return json({ code: "folder_not_found" }, { status: 404 });
    return answer instanceof Response ? answer : json(answer);
  });
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("FolderPicker", () => {
  it("lists the mount's folders with their counts and the already-a-Vault marker", async () => {
    serve({ "": MOUNT });
    render(<FolderPicker value="" onPick={() => {}} />);

    const work = await screen.findByRole("button", { name: /^Work/ });
    expect(work).toHaveTextContent("1,280 notes");
    expect(screen.getByRole("button", { name: /^Archive/ })).toHaveTextContent(
      "at least 10,000 notes",
    );
    const recipes = screen.getByRole("button", { name: /^Recipes/ });
    expect(recipes).toHaveTextContent("Already a Vault");
    expect(recipes).toHaveAttribute("aria-disabled", "true");
    expect(
      screen.getByRole("button", { name: "Open Work" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open Recipes" })).toBeNull();
    expect(
      screen.getByText("Pick the folder that holds your notes."),
    ).toBeInTheDocument();
  });

  it("goes into a folder, picks a subfolder as an absolute path, and comes back up", async () => {
    serve({ "": MOUNT, Work: WORK });
    const onPick = vi.fn();
    render(<FolderPicker value="" onPick={onPick} />);

    fireEvent.click(await screen.findByRole("button", { name: "Open Work" }));
    const projects = await screen.findByRole("button", { name: /^Projects/ });
    expect(screen.getByRole("button", { name: /^Use Work/ })).toHaveFocus();

    fireEvent.click(projects);
    expect(onPick).toHaveBeenCalledWith("/data/vault/Work/Projects");

    fireEvent.click(screen.getByRole("button", { name: /^Use Work/ }));
    expect(onPick).toHaveBeenLastCalledWith("/data/vault/Work");

    fireEvent.click(screen.getByRole("button", { name: "vault" }));
    expect(
      await screen.findByRole("button", { name: /^Archive/ }),
    ).toBeInTheDocument();
  });

  it("marks the chosen folder and names its path", async () => {
    serve({ "": MOUNT });
    render(<FolderPicker value="/data/vault/Work" onPick={() => {}} />);

    expect(
      await screen.findByRole("button", { name: /^Work/ }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText("/data/vault/Work")).toBeInTheDocument();
  });

  it("refuses a folder that is already a Vault and says which one", async () => {
    serve({ "": MOUNT });
    const onPick = vi.fn();
    render(<FolderPicker value="" onPick={onPick} />);

    fireEvent.click(await screen.findByRole("button", { name: /^Recipes/ }));

    expect(onPick).not.toHaveBeenCalled();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Recipes is already the Vault Kitchen. Pick another folder.",
    );
  });

  it("says no notes were found when the mount holds no Markdown", async () => {
    serve({ "": listing("", [folder("Scans", 0)]) });
    render(<FolderPicker value="" onPick={() => {}} />);

    expect(await screen.findByText("No notes found yet")).toBeInTheDocument();
    expect(screen.getByText("/data/vault")).toBeInTheDocument();
    expect(screen.getByText(/Or ask your agent to add it/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "How does this work?" }),
    ).toBeInTheDocument();
  });

  it("keeps an empty mount's folders pickable under the message, to start an empty Vault", async () => {
    serve({ "": listing("", [folder("Scans", 0)]) });
    const onPick = vi.fn();
    render(<FolderPicker value="" onPick={onPick} />);

    await screen.findByText("No notes found yet");
    expect(
      screen.getByText(/To start with an empty Vault, pick a folder below/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^Scans/ }));
    expect(onPick).toHaveBeenCalledWith("/data/vault/Scans");
    fireEvent.click(screen.getByRole("button", { name: /^Use vault/ }));
    expect(onPick).toHaveBeenLastCalledWith("/data/vault");
  });

  it("treats a missing mount as having no notes", async () => {
    serve({ "": listing("", [], { root_found: false }) });
    render(<FolderPicker value="" onPick={() => {}} />);

    expect(await screen.findByText("No notes found yet")).toBeInTheDocument();
    expect(
      screen.getByText(/but that folder does not exist/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Use / })).toBeNull();
  });

  it("explains a folder outside the mount, with Help and the agent route", async () => {
    serve({ "": MOUNT });
    render(<FolderPicker value="" onPick={() => {}} />);

    const toggle = await screen.findByRole("button", {
      name: "My folder isn’t here",
    });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(toggle);

    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getByText("Hatchdoor can’t see this folder yet."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "How does this work?" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Or ask your agent to add it."),
    ).toBeInTheDocument();
  });

  it("shows the server's message when the listing fails, and tries again", async () => {
    serve({
      "": json(
        {
          code: "folder_unreadable",
          message: "Hatchdoor does not have permission to read that folder.",
        },
        { status: 422 },
      ),
    });
    render(<FolderPicker value="" onPick={() => {}} />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Hatchdoor does not have permission to read that folder.",
    );

    serve({ "": MOUNT });
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(
      await screen.findByRole("button", { name: /^Work/ }),
    ).toBeInTheDocument();
  });

  it("keeps the trail usable while a folder loads, and drops a slow answer for a folder the reader left", async () => {
    const WORK_DEEP = listing("Work", [
      folder("Projects", 902, { has_subfolders: true }, "Work"),
    ]);
    let releaseProjects: (response: Response) => void = () => {};
    mockedApiFetch.mockImplementation(async (input) => {
      const path = new URL(
        String(input),
        "http://hatchdoor.test",
      ).searchParams.get("path");
      if (path === "Work/Projects")
        return new Promise<Response>((resolve) => {
          releaseProjects = resolve;
        });
      return json(path === "Work" ? WORK_DEEP : MOUNT);
    });
    render(<FolderPicker value="" onPick={() => {}} />);

    fireEvent.click(await screen.findByRole("button", { name: "Open Work" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Open Projects" }),
    );
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Looking for folders…",
    );
    fireEvent.click(screen.getByRole("button", { name: "vault" }));
    await screen.findByRole("button", { name: /^Archive/ });

    releaseProjects(
      json(listing("Work/Projects", [folder("Q3", 4, {}, "Work/Projects")])),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      screen.getByRole("button", { name: /^Archive/ }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Q3/ })).toBeNull();
  });

  it("counts folders whose names cannot be read", async () => {
    serve({
      "": listing("", [folder("Work", 3)], { skipped_invalid_names: 2 }),
    });
    render(<FolderPicker value="" onPick={() => {}} />);

    expect(
      await screen.findByText(
        "2 folders are not shown because their names cannot be read.",
      ),
    ).toBeInTheDocument();
  });
});

describe("folder picker helpers", () => {
  it("joins the mount and a relative path into the path a Vault is created from", () => {
    expect(mountFolderPath("/data/vault", "")).toBe("/data/vault");
    expect(mountFolderPath("/data/vault", "Work/Projects")).toBe(
      "/data/vault/Work/Projects",
    );
    expect(mountFolderPath("/data/vault/", "Work")).toBe("/data/vault/Work");
    expect(mountFolderPath("/", "Work")).toBe("/Work");
  });

  it("words note counts plainly", () => {
    expect(noteCountLabel({ count: 0, at_least: false })).toBe("no notes");
    expect(noteCountLabel({ count: 1, at_least: false })).toBe("1 note");
    expect(noteCountLabel({ count: 1280, at_least: false })).toBe(
      "1,280 notes",
    );
    expect(noteCountLabel({ count: 10000, at_least: true })).toBe(
      "at least 10,000 notes",
    );
    expect(noteCountLabel({ count: 0, at_least: true })).toBe(
      "at least 0 notes",
    );
  });
});

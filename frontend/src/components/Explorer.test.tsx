import { useState } from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it } from "vitest";

import { FolderTree } from "./Explorer";
import type { ExplorerFolder } from "../types";

const VAULT_ID = "vault-1";

function note(slug: string) {
  return { vault_id: VAULT_ID, title: `${slug} note`, slug };
}

// homelab/hosts/rack holds a note three levels down; archive sits beside
// homelab so a test can prove an unrelated folder's entry survives.
const TREE: ExplorerFolder = {
  name: "Vault",
  folders: [
    {
      name: "homelab",
      folders: [
        {
          name: "hosts",
          folders: [{ name: "rack", folders: [], notes: [note("r1")] }],
          notes: [note("h1")],
        },
      ],
      notes: [note("top")],
    },
    { name: "archive", folders: [], notes: [note("old")] },
  ],
  notes: [],
};

/** Renders `FolderTree` over real state, with the React setter as the change
 * callback so the component's updates land exactly as `App.tsx`'s do.
 * `record()` reads the latest committed record; `openNote()` moves the open
 * note the way navigation does, keeping the record and the component state. */
function renderTree({
  initial = {},
  currentPath = "/",
}: {
  initial?: Record<string, boolean>;
  currentPath?: string;
} = {}) {
  let latest = initial;
  function Wrapper({ path }: { path: string }) {
    const [expandedFolders, setExpandedFolders] = useState(initial);
    latest = expandedFolders;
    return (
      <FolderTree
        root={TREE}
        currentPath={path}
        expandedFolders={expandedFolders}
        onExpandedFoldersChange={setExpandedFolders}
        writeEnabled={false}
        onCreateNoteInFolder={() => {}}
      />
    );
  }
  const utils = render(
    <MemoryRouter>
      <Wrapper path={currentPath} />
    </MemoryRouter>,
  );
  const openNote = async (slug: string) => {
    utils.rerender(
      <MemoryRouter>
        <Wrapper path={`/v/${VAULT_ID}/n/${slug}`} />
      </MemoryRouter>,
    );
    await settle();
  };
  return { ...utils, record: () => latest, openNote };
}

/** Lets queued `toggle` events and the renders they cause land. */
async function settle() {
  await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
}

/** A browser fires `toggle` for each <details> that mounts already open;
 * jsdom does not, so a test that mounts with a note open sends them itself. */
async function fireMountToggles() {
  for (const details of document.querySelectorAll("details")) {
    if (details.open) {
      details.dispatchEvent(new Event("toggle"));
    }
  }
  await settle();
}

function folder(path: string): HTMLDetailsElement {
  const details = screen.getByTitle(path).closest("details");
  if (!details) throw new Error(`No folder ${path}`);
  return details as HTMLDetailsElement;
}

/** Opens or closes a folder the way a click on its summary does: the element
 * changes state, then the browser (jsdom too) queues a `toggle` event. */
async function setOpen(path: string, open: boolean) {
  folder(path).open = open;
  await settle();
}

/** Every folder the reader sees open must show its contents. */
function expectNoOpenEmptyFolder() {
  for (const details of document.querySelectorAll("details")) {
    if (details.open) {
      expect(
        details.querySelector(":scope > ul.tree")?.children.length,
        `open folder ${details.querySelector("summary")?.title} is empty`,
      ).toBeGreaterThan(0);
    }
  }
}

describe("FolderTree nested folders (#305)", () => {
  afterEach(cleanup);

  it("opening a folder inside an open folder shows its contents and records both", async () => {
    const { record } = renderTree();

    await setOpen("homelab", true);
    await setOpen("homelab/hosts", true);

    expect(screen.getByText("h1 note")).toBeInTheDocument();
    expect(screen.getByTitle("homelab/hosts/rack")).toBeInTheDocument();
    expect(record()).toEqual({ homelab: true, "homelab/hosts": true });
    expectNoOpenEmptyFolder();
  });

  it("holds three levels deep", async () => {
    const { record } = renderTree();

    await setOpen("homelab", true);
    await setOpen("homelab/hosts", true);
    await setOpen("homelab/hosts/rack", true);

    expect(screen.getByText("r1 note")).toBeInTheDocument();
    expect(record()).toEqual({
      homelab: true,
      "homelab/hosts": true,
      "homelab/hosts/rack": true,
    });
    expectNoOpenEmptyFolder();
  });

  it("closing a nested folder closes only that folder", async () => {
    const { record } = renderTree({
      initial: { homelab: true, "homelab/hosts": true },
    });

    await setOpen("homelab/hosts", false);

    expect(folder("homelab").open).toBe(true);
    expect(screen.getByText("top note")).toBeInTheDocument();
    expect(screen.queryByText("h1 note")).toBeNull();
    expect(record()).toEqual({ homelab: true, "homelab/hosts": false });
  });

  it("showing a deep note leaves other folders' entries alone", async () => {
    const { record } = renderTree({
      initial: { archive: true },
      currentPath: `/v/${VAULT_ID}/n/h1`,
    });
    // Both ancestors mount open, so a browser fires both toggles, back to
    // back as one batch.
    folder("homelab").dispatchEvent(new Event("toggle"));
    folder("homelab/hosts").dispatchEvent(new Event("toggle"));
    await settle();

    expect(record()).toEqual({ archive: true });
    expect(folder("homelab").open).toBe(true);
    expect(folder("homelab/hosts").open).toBe(true);
    expect(folder("archive").open).toBe(true);
    expect(screen.getByText("h1 note")).toBeInTheDocument();
    expectNoOpenEmptyFolder();
  });

  it("restores nested folders from the stored record, as after a reload", async () => {
    const first = renderTree();
    await setOpen("homelab", true);
    await setOpen("homelab/hosts", true);
    const saved = first.record();
    first.unmount();

    renderTree({ initial: saved });

    expect(folder("homelab/hosts").open).toBe(true);
    expect(screen.getByText("h1 note")).toBeInTheDocument();
  });

  it("never shows an open folder empty, even when the record does not follow the toggle", async () => {
    render(
      <MemoryRouter>
        <FolderTree
          root={TREE}
          currentPath="/"
          expandedFolders={{}}
          onExpandedFoldersChange={() => {}}
          writeEnabled={false}
          onCreateNoteInFolder={() => {}}
        />
      </MemoryRouter>,
    );

    await setOpen("homelab", true);

    expect(screen.getByText("top note")).toBeInTheDocument();
    expectNoOpenEmptyFolder();
  });
});

describe("FolderTree folders shown for the open note (#365)", () => {
  afterEach(cleanup);

  const DEEP_FOLDERS = ["homelab", "homelab/hosts", "homelab/hosts/rack"];

  it("shows a deep note's folders without saving them", async () => {
    const { record } = renderTree({ currentPath: `/v/${VAULT_ID}/n/r1` });
    await fireMountToggles();

    for (const path of DEEP_FOLDERS) {
      expect(folder(path).open).toBe(true);
    }
    expect(screen.getByText("r1 note")).toBeInTheDocument();
    expect(record()).toEqual({});
    expectNoOpenEmptyFolder();
  });

  it("closes them again when a note elsewhere opens", async () => {
    const { record, openNote } = renderTree({
      currentPath: `/v/${VAULT_ID}/n/r1`,
    });
    await fireMountToggles();

    await openNote("old");

    expect(folder("homelab").open).toBe(false);
    expect(screen.queryByTitle("homelab/hosts")).toBeNull();
    expect(folder("archive").open).toBe(true);
    expect(record()).toEqual({});
    expectNoOpenEmptyFolder();
  });

  it("keeps a folder the reader opened, while its note-opened children close", async () => {
    const { record, openNote } = renderTree();
    await setOpen("homelab", true);

    await openNote("r1");
    await openNote("old");

    expect(folder("homelab").open).toBe(true);
    expect(folder("homelab/hosts").open).toBe(false);
    expect(screen.queryByTitle("homelab/hosts/rack")).toBeNull();
    expect(record()).toEqual({ homelab: true });
    expectNoOpenEmptyFolder();
  });

  it("respects the reader closing the folder that holds the open note", async () => {
    const { record, openNote } = renderTree({
      currentPath: `/v/${VAULT_ID}/n/h1`,
    });
    await fireMountToggles();

    await setOpen("homelab/hosts", false);
    // The same note again: a re-render must not reopen it.
    await openNote("h1");

    expect(folder("homelab/hosts").open).toBe(false);
    expect(screen.queryByText("h1 note")).toBeNull();
    expect(record()).toEqual({ "homelab/hosts": false });
  });

  it("opens a closed folder again when a note inside it opens", async () => {
    const { record, openNote } = renderTree({
      currentPath: `/v/${VAULT_ID}/n/h1`,
    });
    await fireMountToggles();
    await setOpen("homelab/hosts", false);

    await openNote("r1");

    expect(folder("homelab/hosts").open).toBe(true);
    expect(folder("homelab/hosts/rack").open).toBe(true);
    expect(screen.getByText("r1 note")).toBeInTheDocument();
    expect(record()).toEqual({ "homelab/hosts": false });
    expectNoOpenEmptyFolder();
  });

  it("opens a closed folder again when the reader returns to the note it was closed at", async () => {
    const { openNote } = renderTree({ currentPath: `/v/${VAULT_ID}/n/top` });
    await fireMountToggles();
    await setOpen("homelab", false);

    await openNote("old");
    await openNote("top");

    expect(folder("homelab").open).toBe(true);
    expect(screen.getByText("top note")).toBeInTheDocument();
    expectNoOpenEmptyFolder();
  });

  it("leaves a closed folder closed when a note outside it opens", async () => {
    const { record, openNote } = renderTree({
      currentPath: `/v/${VAULT_ID}/n/top`,
    });
    await fireMountToggles();
    await setOpen("homelab", false);

    await openNote("old");

    expect(folder("homelab").open).toBe(false);
    expect(record()).toEqual({ homelab: false });
  });

  it("saves a folder the reader reopens above the open note, and keeps it open elsewhere", async () => {
    const { record, openNote } = renderTree({
      currentPath: `/v/${VAULT_ID}/n/top`,
    });
    await fireMountToggles();
    await setOpen("homelab", false);
    await setOpen("homelab", true);

    expect(record()).toEqual({ homelab: true });
    await openNote("old");

    expect(folder("homelab").open).toBe(true);
    expect(record()).toEqual({ homelab: true });
    expectNoOpenEmptyFolder();
  });

  it("shows the open note's folders after a reload, whatever the record says, and writes nothing", async () => {
    const saved = { homelab: false, "homelab/hosts": false, archive: true };
    const { record } = renderTree({
      initial: saved,
      currentPath: `/v/${VAULT_ID}/n/h1`,
    });
    await fireMountToggles();

    expect(folder("homelab").open).toBe(true);
    expect(folder("homelab/hosts").open).toBe(true);
    expect(screen.getByText("h1 note")).toBeInTheDocument();
    expect(record()).toEqual(saved);
    expectNoOpenEmptyFolder();
  });
});

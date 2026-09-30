import { useState } from "react";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
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
 * `record()` reads the latest committed record. */
function renderTree({
  initial = {},
  currentPath = "/",
}: {
  initial?: Record<string, boolean>;
  currentPath?: string;
} = {}) {
  let latest = initial;
  function Wrapper() {
    const [expandedFolders, setExpandedFolders] = useState(initial);
    latest = expandedFolders;
    return (
      <FolderTree
        root={TREE}
        currentPath={currentPath}
        expandedFolders={expandedFolders}
        onExpandedFoldersChange={setExpandedFolders}
        writeEnabled={false}
        onCreateNoteInFolder={() => {}}
      />
    );
  }
  const utils = render(
    <MemoryRouter>
      <Wrapper />
    </MemoryRouter>,
  );
  return { ...utils, record: () => latest };
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
  // Let the queued toggle event and the render it causes land.
  await new Promise((resolve) => setTimeout(resolve, 0));
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

  it("opening a deep note opens its ancestors without erasing other folders' entries", async () => {
    const { record } = renderTree({
      initial: { archive: true },
      currentPath: `/v/${VAULT_ID}/n/h1`,
    });
    // A browser fires `toggle` for each <details> that mounts already open;
    // jsdom does not, so both are sent here, back to back as one batch.
    folder("homelab").dispatchEvent(new Event("toggle"));
    folder("homelab/hosts").dispatchEvent(new Event("toggle"));

    await waitFor(() =>
      expect(record()).toEqual({
        archive: true,
        homelab: true,
        "homelab/hosts": true,
      }),
    );
    expect(folder("homelab").open).toBe(true);
    expect(folder("homelab/hosts").open).toBe(true);
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

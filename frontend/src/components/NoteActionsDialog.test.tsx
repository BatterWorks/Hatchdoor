import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NoteActionsDialog } from "./NoteActionsDialog";
import { createDraftKey } from "../lib/writeDrafts";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
});

const VAULTS = [
  { vaultId: "work", name: "Work notes" },
  { vaultId: "personal", name: "Personal" },
];

const FOLDERS_BY_VAULT = {
  work: ["10-topics", "10-topics/Launch"],
  personal: ["Journal"],
};

function renderCreateDialog(
  props: Partial<
    Parameters<typeof NoteActionsDialog>[0] & { onCreate: () => void }
  > = {},
) {
  render(
    <NoteActionsDialog
      kind="create"
      error={null}
      vaults={VAULTS}
      folderPathsByVault={FOLDERS_BY_VAULT}
      initialVaultId="work"
      initialFolder=""
      onClose={() => {}}
      onCreate={() => {}}
      onRename={() => {}}
      onMove={() => {}}
      onArchive={() => {}}
      onDelete={() => {}}
      {...props}
    />,
  );
}

describe("NoteActionsDialog folder picker", () => {
  it("offers only the selected Vault's folders, prefixes included", () => {
    renderCreateDialog();

    // Numeric prefixes are the real folder names and encode a deliberate
    // order, so they are never stripped or prettified.
    const select = screen.getByLabelText("Folder");
    expect(
      within(select).getByRole("option", { name: "Vault root" }),
    ).toBeInTheDocument();
    expect(
      within(select).getByRole("option", { name: "10-topics" }),
    ).toBeInTheDocument();
    expect(
      within(select).getByRole("option", { name: "10-topics/Launch" }),
    ).toBeInTheDocument();
    // Another Vault's folder is never offered here: the folder list and the
    // Vault it belongs to have to agree.
    expect(
      within(select).queryByRole("option", { name: "Journal" }),
    ).not.toBeInTheDocument();
  });

  it("swaps the folder list when the Vault changes", () => {
    renderCreateDialog();

    fireEvent.change(screen.getByLabelText("Folder"), {
      target: { value: "10-topics" },
    });
    fireEvent.change(screen.getByLabelText("Vault"), {
      target: { value: "personal" },
    });

    const select = screen.getByLabelText("Folder");
    expect(
      within(select).getByRole("option", { name: "Journal" }),
    ).toBeInTheDocument();
    expect(
      within(select).queryByRole("option", { name: "10-topics" }),
    ).not.toBeInTheDocument();
    // The previously chosen folder does not survive the switch; it named a
    // path in a Vault that is no longer the destination.
    expect((select as HTMLSelectElement).value).toBe("");
  });

  it("hides the Vault field when there is only one Vault", () => {
    renderCreateDialog({ vaults: [VAULTS[0]] });

    expect(screen.queryByLabelText("Vault")).not.toBeInTheDocument();
  });

  it("reveals a name field when creating a new folder", () => {
    renderCreateDialog();

    expect(screen.queryByLabelText("New folder name")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Folder"), {
      target: { value: "//new-folder" },
    });

    expect(screen.getByLabelText("New folder name")).toBeInTheDocument();
  });

  it("previews the Vault and path that will be created", () => {
    renderCreateDialog();

    fireEvent.change(screen.getByLabelText("Folder"), {
      target: { value: "10-topics" },
    });
    fireEvent.change(screen.getByLabelText("Note name"), {
      target: { value: "Weekly review" },
    });

    expect(
      screen.getByText("Work notes / 10-topics / Weekly review.md"),
    ).toBeInTheDocument();
  });
});

describe("NoteActionsDialog create", () => {
  it("creates in the chosen Vault, not the one the shell would infer", () => {
    const onCreate = vi.fn();
    renderCreateDialog({ onCreate });

    fireEvent.change(screen.getByLabelText("Vault"), {
      target: { value: "personal" },
    });
    fireEvent.change(screen.getByLabelText("Folder"), {
      target: { value: "Journal" },
    });
    fireEvent.change(screen.getByLabelText("Note name"), {
      target: { value: "Monday" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create and open" }));

    expect(onCreate).toHaveBeenCalledWith("personal", "Journal/Monday");
  });

  it("collects no body: the note is written after it opens", () => {
    renderCreateDialog();

    expect(screen.queryByLabelText("Markdown content")).not.toBeInTheDocument();
  });
});

describe("NoteActionsDialog create draft persistence", () => {
  it("persists the destination so an auto-update reload does not lose it", () => {
    renderCreateDialog();

    fireEvent.change(screen.getByLabelText("Vault"), {
      target: { value: "personal" },
    });
    fireEvent.change(screen.getByLabelText("Note name"), {
      target: { value: "Half typed" },
    });

    // A service-worker autoUpdate reload wipes the DOM. Where the note was
    // going must survive in localStorage under the create-draft key.
    const persisted = window.localStorage.getItem(createDraftKey());
    expect(persisted).not.toBeNull();
    expect(persisted).toContain("Half typed");
    expect(persisted).toContain("personal");
  });
});

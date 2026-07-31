import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { NoteActionsDialog } from "./NoteActionsDialog";
import { createDraftKey } from "../lib/writeDrafts";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
});

function renderCreateDialog() {
  render(
    <NoteActionsDialog
      kind="create"
      error={null}
      folderPaths={[]}
      initialFolder=""
      onClose={() => {}}
      onCreate={() => {}}
      onRename={() => {}}
      onMove={() => {}}
      onArchive={() => {}}
      onDelete={() => {}}
    />,
  );
}

describe("NoteActionsDialog folder picker", () => {
  function renderWithFolders() {
    render(
      <NoteActionsDialog
        kind="create"
        error={null}
        folderPaths={["10-topics", "10-topics/Launch"]}
        initialFolder=""
        onClose={() => {}}
        onCreate={() => {}}
        onRename={() => {}}
        onMove={() => {}}
        onArchive={() => {}}
        onDelete={() => {}}
      />,
    );
  }

  it("offers existing folders verbatim, prefixes included", () => {
    renderWithFolders();

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
  });

  it("reveals a name field when creating a new folder", () => {
    renderWithFolders();

    expect(screen.queryByLabelText("New folder name")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Folder"), {
      target: { value: "//new-folder" },
    });

    expect(screen.getByLabelText("New folder name")).toBeInTheDocument();
  });

  it("previews the path that will be created", () => {
    renderWithFolders();

    fireEvent.change(screen.getByLabelText("Folder"), {
      target: { value: "10-topics" },
    });
    fireEvent.change(screen.getByLabelText("Note name"), {
      target: { value: "Weekly review" },
    });

    expect(
      screen.getByText("10-topics / Weekly review.md"),
    ).toBeInTheDocument();
  });
});

describe("NoteActionsDialog create draft persistence", () => {
  it("persists typed create-note content so an auto-update reload does not lose it", () => {
    renderCreateDialog();

    fireEvent.change(screen.getByLabelText("Markdown content"), {
      target: { value: "My unsaved draft body" },
    });

    // A service-worker autoUpdate reload wipes the DOM. The typed content must
    // survive in localStorage under the create-draft key so it can be restored.
    const persisted = window.localStorage.getItem(createDraftKey());
    expect(persisted).not.toBeNull();
    expect(persisted).toContain("My unsaved draft body");
  });
});

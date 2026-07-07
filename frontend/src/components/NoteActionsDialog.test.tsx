import { cleanup, fireEvent, render, screen } from "@testing-library/react";
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

describe("NoteActionsDialog folder suggestions", () => {
  it("uses a custom folder suggestion list instead of datalist", () => {
    render(
      <NoteActionsDialog
        kind="create"
        error={null}
        folderPaths={["Projects", "Projects/Launch"]}
        initialFolder=""
        onClose={() => {}}
        onCreate={() => {}}
        onRename={() => {}}
        onMove={() => {}}
        onArchive={() => {}}
        onDelete={() => {}}
      />,
    );

    expect(document.querySelector("datalist")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Projects" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Projects/Launch" }),
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

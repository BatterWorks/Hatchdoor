import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { HeldDraft } from "../../lib/writeDrafts";
import type { VaultSummary } from "../../types";
import { UnsavedDrafts } from "./UnsavedDrafts";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function vault(id: string, name: string): VaultSummary {
  return {
    vault_id: id,
    name,
    enabled: true,
    exclude_patterns: [],
    credential_configured: false,
    activation: "active",
    local_content: "read_write",
    search: "ready",
    git: "disabled",
    watcher: "running",
    capabilities: {} as VaultSummary["capabilities"],
  } as VaultSummary;
}

const noteDraft: HeldDraft = {
  id: "note:orphaned",
  kind: "note",
  slug: "orphaned",
  content: "some unsaved text",
  baseContentHash: "abc",
  savedAt: Date.now() - 60_000,
};

const createDraft: HeldDraft = {
  id: "create",
  kind: "create",
  folder: "10-topics",
  name: "In Progress",
  content: "half-written note",
  savedAt: Date.now() - 60_000,
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.localStorage.clear();
});

describe("UnsavedDrafts (#151)", () => {
  it("restores a note draft into the destination Vault and discards the held copy", async () => {
    const v = vault("vault-1", "Alpha");
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      jsonResponse({
        vault_id: v.vault_id,
        note: { content_hash: "fresh-hash" },
      }),
    );
    const onDiscard = vi.fn();

    render(
      <MemoryRouter>
        <UnsavedDrafts
          drafts={[noteDraft]}
          vaults={[v]}
          onDiscard={onDiscard}
        />
      </MemoryRouter>,
    );

    // A single Vault pre-fills the destination.
    expect(screen.getByLabelText("Destination Vault")).toHaveValue(
      "vault-1",
    );

    fireEvent.click(screen.getByRole("button", { name: "Restore" }));

    await waitFor(() => expect(onDiscard).toHaveBeenCalledWith("note:orphaned"));
    const stored = window.localStorage.getItem(
      "hatchdoor:draft:note:vault-1:orphaned",
    );
    expect(stored).toContain("some unsaved text");
    // The draft's own baseContentHash survives restore — never the
    // destination note's freshly fetched hash — so the existing stale-draft
    // comparison in NotePage still fires correctly (#151 spec review).
    expect(stored).toContain('"baseContentHash":"abc"');
  });

  it("offers a different Vault or a new note when the destination has no such note", async () => {
    const v = vault("vault-1", "Alpha");
    vi.spyOn(globalThis, "fetch").mockResolvedValue(jsonResponse({}, 404));
    const onOpenCreateDraft = vi.fn();
    const onDiscard = vi.fn();

    render(
      <MemoryRouter>
        <UnsavedDrafts
          drafts={[noteDraft]}
          vaults={[v]}
          onOpenCreateDraft={onOpenCreateDraft}
          onDiscard={onDiscard}
        />
      </MemoryRouter>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Restore" }));

    await screen.findByText(/has no note at/);
    fireEvent.click(
      screen.getByRole("button", { name: "restore it as a new note here" }),
    );

    expect(onOpenCreateDraft).toHaveBeenCalledWith(
      "vault-1",
      "",
      "orphaned",
      "some unsaved text",
    );
    expect(onDiscard).toHaveBeenCalledWith("note:orphaned");
  });

  it("restores a create draft directly without checking for an existing note", async () => {
    const v = vault("vault-1", "Alpha");
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    const onOpenCreateDraft = vi.fn();
    const onDiscard = vi.fn();

    render(
      <MemoryRouter>
        <UnsavedDrafts
          drafts={[createDraft]}
          vaults={[v]}
          onOpenCreateDraft={onOpenCreateDraft}
          onDiscard={onDiscard}
        />
      </MemoryRouter>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Restore" }));

    expect(fetchSpy).not.toHaveBeenCalled();
    expect(onOpenCreateDraft).toHaveBeenCalledWith(
      "vault-1",
      "10-topics",
      "In Progress",
      "half-written note",
    );
    expect(onDiscard).toHaveBeenCalledWith("create");
  });

  it("discards only after confirmation", () => {
    const v = vault("vault-1", "Alpha");
    const onDiscard = vi.fn();
    vi.spyOn(window, "confirm").mockReturnValueOnce(false);

    render(
      <MemoryRouter>
        <UnsavedDrafts
          drafts={[noteDraft]}
          vaults={[v]}
          onDiscard={onDiscard}
        />
      </MemoryRouter>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    expect(onDiscard).not.toHaveBeenCalled();

    vi.spyOn(window, "confirm").mockReturnValueOnce(true);
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    expect(onDiscard).toHaveBeenCalledWith("note:orphaned");
  });

  it("does not pre-fill the destination when more than one Vault is enabled", () => {
    render(
      <MemoryRouter>
        <UnsavedDrafts
          drafts={[noteDraft]}
          vaults={[vault("vault-1", "Alpha"), vault("vault-2", "Beta")]}
          onDiscard={vi.fn()}
        />
      </MemoryRouter>,
    );

    expect(screen.getByLabelText("Destination Vault")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Restore" })).toBeDisabled();
  });
});

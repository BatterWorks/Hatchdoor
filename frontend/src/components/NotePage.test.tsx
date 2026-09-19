import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { act } from "react";
import { EditorView } from "@codemirror/view";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NOTE_PROPERTIES_COLLAPSED_KEY } from "../app/constants";
import { isAppReloadHeld, resetAppReloadHolds } from "../lib/reloadGuard";
import { loadNoteDraft, saveNoteDraft } from "../lib/writeDrafts";
import { staleVault, syncStoppedVault } from "../test/fixtures/vaults";
import { NotePage } from "./NotePage";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function renderNote(
  vaultId: string,
  overrides: Partial<Parameters<typeof NotePage>[0]> = {},
) {
  const props = {
    onActiveNoteChange: vi.fn(),
    onTagSelect: vi.fn(),
    propertiesCollapsedStorageKey: NOTE_PROPERTIES_COLLAPSED_KEY,
    vaultRevision: null,
    writeEnabled: true,
    editRequestId: 0,
    vaults: [],
    ...overrides,
  };

  return render(
    <MemoryRouter initialEntries={[`/v/${vaultId}/n/home`]}>
      <Routes>
        <Route path="/v/:vaultId/n/:slug" element={<NotePage {...props} />} />
      </Routes>
    </MemoryRouter>,
  );
}

/** Like `renderNote`, with the Vault revision under the test's control. */
function renderNoteAtRevision(
  vaultId: string,
  vaults: Parameters<typeof NotePage>[0]["vaults"],
  revision: number,
) {
  const tree = (vaultRevision: number) => (
    <MemoryRouter initialEntries={[`/v/${vaultId}/n/home`]}>
      <Routes>
        <Route
          path="/v/:vaultId/n/:slug"
          element={
            <NotePage
              onActiveNoteChange={vi.fn()}
              onTagSelect={vi.fn()}
              propertiesCollapsedStorageKey={NOTE_PROPERTIES_COLLAPSED_KEY}
              vaultRevision={vaultRevision}
              writeEnabled={true}
              editRequestId={0}
              vaults={vaults}
            />
          }
        />
      </Routes>
    </MemoryRouter>
  );
  const view = render(tree(revision));
  return {
    view,
    setRevision: (next: number) => view.rerender(tree(next)),
  };
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.localStorage.clear();
  resetAppReloadHolds();
});

describe("NotePage write escalation (#141)", () => {
  it("shows Not saving and the full-bleed notice for a stopped Vault, before any save is attempted", async () => {
    const vault = syncStoppedVault("Beta");
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: vault.vault_id,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "Body",
              content_hash: "hash",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: vault.vault_id, results: [] });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    renderNote(vault.vault_id, { vaults: [vault] });

    await waitFor(() => {
      expect(screen.getByText("Not saving")).toBeInTheDocument();
    });
    expect(
      screen.getByText(/Local edits in this Vault halted Git integration\./),
    ).toBeInTheDocument();
  });

  it("shows Not saving and the notice for a conflicted Vault, with the Vault's own message", async () => {
    const vault = {
      ...staleVault("Ignored"),
      vault_id: "conflict-vault",
      git: "unavailable" as const,
      git_error: {
        code: "git_content_conflict",
        message: "A content conflict is blocking Git integration.",
        retryable: false,
      },
    };
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: vault.vault_id,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "Body",
              content_hash: "hash",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: vault.vault_id, results: [] });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    renderNote(vault.vault_id, { vaults: [vault] });

    await waitFor(() => {
      expect(screen.getByText("Not saving")).toBeInTheDocument();
    });
    expect(
      screen.getByText(/A content conflict is blocking Git integration\./),
    ).toBeInTheDocument();
  });

  it("shows the instruction-free fallback sentence, not the Vault's own operator diagnostic, in demo mode (#152)", async () => {
    const vault = {
      ...staleVault("Ignored"),
      vault_id: "conflict-vault-demo",
      git: "unavailable" as const,
      git_error: {
        code: "git_content_conflict",
        message: "Run `hatchdoor vault repair` on the operator console.",
        retryable: false,
      },
    };
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: vault.vault_id,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "Body",
              content_hash: "hash",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: vault.vault_id, results: [] });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    renderNote(vault.vault_id, {
      vaults: [vault],
      writeEnabled: false,
      demoMode: true,
    });

    await waitFor(() => {
      expect(
        screen.getByText(
          /A content conflict is blocking Git sync for this Vault\./,
        ),
      ).toBeInTheDocument();
    });
    expect(screen.queryByText(/operator console/)).not.toBeInTheDocument();
  });

  it("raises nothing beyond the sidebar slot for a non-blocking condition (stale)", async () => {
    const vault = staleVault("Gamma");
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: vault.vault_id,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "Body",
              content_hash: "hash",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: vault.vault_id, results: [] });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    renderNote(vault.vault_id, { vaults: [vault] });

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Edit" })).toBeInTheDocument();
    });
    expect(screen.queryByText("Not saving")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/The last index build for this Vault failed\./),
    ).not.toBeInTheDocument();
  });
});

describe("NotePage tag taps hand over this note's own Vault (#144)", () => {
  it("calls onTagSelect with the tag and the open note's Vault", async () => {
    const vaultId = "vault-work";
    const onTagSelect = vi.fn();
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: vaultId,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "---\ntitle: Home\ntags:\n  - orchard\n---\n# Body\n",
              content_hash: "hash",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: vaultId, results: [] });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    // The properties grid starts collapsed; expand it so the tag chip is
    // actually reachable.
    window.localStorage.setItem(NOTE_PROPERTIES_COLLAPSED_KEY, "0");
    // writeEnabled: false so the tag chip selects rather than entering
    // frontmatter edit mode (sections.tsx routes a tag click to onTagSelect
    // only when the property grid is not itself editable).
    renderNote(vaultId, { onTagSelect, vaults: [], writeEnabled: false });

    const tagChip = await screen.findByRole("button", { name: "#orchard" });
    tagChip.click();

    expect(onTagSelect).toHaveBeenCalledExactlyOnceWith("orchard", vaultId);
  });
});

describe("NotePage read escalation (#141)", () => {
  it("renders the documented (red) error block when a note cannot be read", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      jsonResponse({ code: "vault_unavailable", message: "boom" }, 503),
    );

    const { container } = renderNote("vault-1", { vaults: [] });

    await waitFor(() => {
      expect(screen.getByText("Note Unavailable")).toBeInTheDocument();
    });
    expect(container.querySelector(".state-block.error")).not.toBeNull();
  });
});

describe("NotePage held-draft recovery (#151)", () => {
  it("shows a dismissible notice above the note body when a held draft exists", async () => {
    window.localStorage.setItem(
      "hatchdoor:heldDraft:note:orphaned",
      JSON.stringify({
        id: "note:orphaned",
        kind: "note",
        slug: "orphaned",
        content: "unsaved",
        baseContentHash: "abc",
        savedAt: Date.now(),
      }),
    );
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: "vault-1",
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "Body",
              content_hash: "hash",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: "vault-1", results: [] });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    renderNote("vault-1", { vaults: [] });

    const notice = await screen.findByText(/find them in Settings/);
    expect(notice.closest("a")).toHaveAttribute("href", "/settings");

    fireEvent.click(screen.getByRole("button", { name: "Dismiss notice" }));
    expect(screen.queryByText(/find them in Settings/)).not.toBeInTheDocument();
  });

  it("never shows the held-drafts notice in demo mode, even with a held draft present (#152)", async () => {
    window.localStorage.setItem(
      "hatchdoor:heldDraft:note:orphaned",
      JSON.stringify({
        id: "note:orphaned",
        kind: "note",
        slug: "orphaned",
        content: "unsaved",
        baseContentHash: "abc",
        savedAt: Date.now(),
      }),
    );
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: "vault-1",
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "Body",
              content_hash: "hash",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: "vault-1", results: [] });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    renderNote("vault-1", { vaults: [], demoMode: true });

    await waitFor(() => {
      expect(screen.getByText("Body")).toBeInTheDocument();
    });
    expect(screen.queryByText(/find them in Settings/)).not.toBeInTheDocument();
  });

  it("opens the editor with a recovered draft on ?restoreEdit=1 and strips the marker", async () => {
    const vaultId = "vault-1";
    saveNoteDraft(vaultId, "home", {
      vaultId,
      slug: "home",
      content: "recovered draft text",
      baseContentHash: "hash",
      savedAt: Date.now(),
    });
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: vaultId,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "Body on disk",
              content_hash: "hash",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: vaultId, results: [] });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${vaultId}/n/home?restoreEdit=1`]}>
        <Routes>
          <Route
            path="/v/:vaultId/n/:slug"
            element={
              <NotePage
                onActiveNoteChange={vi.fn()}
                onTagSelect={vi.fn()}
                propertiesCollapsedStorageKey={NOTE_PROPERTIES_COLLAPSED_KEY}
                vaultRevision={0}
                writeEnabled={true}
                editRequestId={0}
                vaults={[]}
              />
            }
          />
        </Routes>
      </MemoryRouter>,
    );

    expect(
      await screen.findByDisplayValue("recovered draft text"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Body on disk")).not.toBeInTheDocument();
  });
});

describe("NotePage crash-safe inline editing (#330)", () => {
  type Sent = { url: string; init: RequestInit };

  /**
   * The note read, the wikilink resolve, and a recording PUT handler — the
   * three requests the note page makes while an edit is in flight.
   */
  function mockVault(
    content: string,
    hash = "hash",
    vaultId = "vault-1",
  ): Sent[] {
    const sent: Sent[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (init?.method === "PUT") {
          sent.push({ url, init });
          return jsonResponse({
            vault_id: vaultId,
            ok: true,
            slug: "home",
            relative_path: "Home.md",
            content_hash: "hash-2",
            quality_warnings: [],
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: null,
            layer: null,
          });
        }
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: vaultId, results: [] });
        }
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: vaultId,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home.md",
              content,
              content_hash: hash,
              layer: null,
            },
          });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );
    return sent;
  }

  /** The open block is a CodeMirror editor, so its text lives in editor state
   * rather than in a DOM value. */
  function typeInOpenBlock(text: string): void {
    const view = EditorView.findFromDOM(screen.getByRole("textbox"));
    if (!view) {
      throw new Error("no CodeMirror view is mounted on the active block");
    }
    act(() => {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: text },
      });
    });
  }

  it("restores an unsaved inline edit from its draft after a reload", async () => {
    saveNoteDraft("vault-1", "home", {
      vaultId: "vault-1",
      slug: "home",
      content: "Body with the sentence that never saved.",
      baseContentHash: "hash",
      savedAt: Date.now(),
    });
    const sent = mockVault("Body on disk.\n");

    renderNote("vault-1", { vaults: [] });

    expect(
      await screen.findByText("Body with the sentence that never saved."),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Restored an edit that had not reached the vault yet/),
    ).toBeInTheDocument();
    // The interrupted write is finished rather than left sitting in the draft.
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(JSON.parse(String(sent[0].init.body)).content).toBe(
      "Body with the sentence that never saved.",
    );
  });

  it("leaves the body alone and points at source mode when the note moved under the draft", async () => {
    saveNoteDraft("vault-1", "home", {
      vaultId: "vault-1",
      slug: "home",
      content: "An edit based on an older version.",
      baseContentHash: "older-hash",
      savedAt: Date.now(),
    });
    const sent = mockVault("Body on disk.\n");

    renderNote("vault-1", { vaults: [] });

    expect(await screen.findByText("Body on disk.")).toBeInTheDocument();
    expect(
      screen.getByText(/the note has changed since. Use Edit to review it/),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("An edit based on an older version."),
    ).not.toBeInTheDocument();
    expect(sent).toHaveLength(0);
  });

  it("persists text typed into an open block and delivers it with keepalive when the tab closes", async () => {
    const sent = mockVault("First paragraph.\n");

    renderNote("vault-1", { vaults: [] });

    fireEvent.click(await screen.findByText("First paragraph."));
    typeInOpenBlock("First paragraph, still being typed.");

    // Inside the idle-flush window: nothing has gone out yet, which is the
    // window a closing tab falls into.
    expect(sent).toHaveLength(0);

    const draftAtPageHide = (() => {
      let seen: string | null = null;
      act(() => {
        window.dispatchEvent(new Event("pagehide"));
        seen = loadNoteDraft("vault-1", "home")?.content ?? null;
      });
      return seen as string | null;
    })();

    // Written synchronously inside the handler, before the send goes out:
    // nothing the send reports can reach a page that is already gone.
    expect(draftAtPageHide).toContain("First paragraph, still being typed.");

    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0].init.keepalive).toBe(true);
    expect(JSON.parse(String(sent[0].init.body)).content).toContain(
      "First paragraph, still being typed.",
    );
    // This page happens to survive its own pagehide, and the send is booked
    // like any other save, so the draft it no longer needs is cleared.
    await waitFor(() => expect(loadNoteDraft("vault-1", "home")).toBeNull());
  });

  it("does not write a draft per keystroke, and flushes the pending one on the way out", async () => {
    mockVault("Body on disk.\n");

    renderNote("vault-1", { vaults: [] });

    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    const textarea = await screen.findByRole("textbox");
    fireEvent.change(textarea, { target: { value: "Body being typed." } });

    // The write is debounced off the typing path rather than running a
    // JSON.stringify plus a synchronous setItem on every keystroke.
    expect(loadNoteDraft("vault-1", "home")).toBeNull();

    act(() => {
      window.dispatchEvent(new Event("pagehide"));
    });

    expect(loadNoteDraft("vault-1", "home")?.content).toBe("Body being typed.");
  });

  it("says so when the browser refuses to store the draft", async () => {
    mockVault("Body on disk.\n");

    renderNote("vault-1", { vaults: [] });

    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    const textarea = await screen.findByRole("textbox");
    fireEvent.change(textarea, { target: { value: "Body being typed." } });

    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("quota exceeded", "QuotaExceededError");
    });
    act(() => {
      window.dispatchEvent(new Event("pagehide"));
    });

    expect(
      await screen.findByText(
        /Your browser isn’t storing drafts, so saving is the only way to keep this edit\./,
      ),
    ).toBeInTheDocument();
  });

  // Undo is the third way the document changes, and it was the one that did
  // not write a draft. On a Vault that refuses the commit, the draft left on
  // disk held the pre-undo text, so the page going away restored the edit the
  // user had just taken back.
  it("writes the draft for an undo the Vault will not take", async () => {
    const vault = syncStoppedVault("Beta");
    const sent = mockVault("First paragraph.\n", "hash", vault.vault_id);

    renderNote(vault.vault_id, { vaults: [vault] });

    fireEvent.click(await screen.findByText("First paragraph."));
    typeInOpenBlock("First paragraph, edited.");
    fireEvent.blur(screen.getByRole("textbox"));
    await screen.findByText("First paragraph, edited.");

    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    await screen.findByText("First paragraph.");

    act(() => {
      window.dispatchEvent(new Event("pagehide"));
    });

    // Nothing was ever sent, so the draft is the only copy of what the user is
    // looking at.
    expect(sent).toHaveLength(0);
    const draft = loadNoteDraft(vault.vault_id, "home");
    expect(draft?.content).toContain("First paragraph.");
    expect(draft?.content).not.toContain("edited");
  });

  it("does not put the draft back after the save that cleared it", async () => {
    const sent = mockVault("First paragraph.\n");

    renderNote("vault-1", { vaults: [] });

    fireEvent.click(await screen.findByText("First paragraph."));
    typeInOpenBlock("First paragraph, edited.");
    fireEvent.blur(screen.getByRole("textbox"));

    await waitFor(() => expect(sent).toHaveLength(1));
    await waitFor(() => expect(loadNoteDraft("vault-1", "home")).toBeNull());

    // The debounced write scheduled by that edit fires after the save landed.
    // Recreating the draft there leaves text the vault already holds sitting
    // under a hash that has moved on, and the next visit reports a held edit
    // that was in fact saved.
    await new Promise((resolve) => setTimeout(resolve, 900));
    expect(loadNoteDraft("vault-1", "home")).toBeNull();
  });

  it("notices the note changed on disk even while a stopped Vault holds the edit", async () => {
    const vault = syncStoppedVault("Beta");
    mockVault("First paragraph.\n", "hash", vault.vault_id);
    const { setRevision } = renderNoteAtRevision(vault.vault_id, [vault], 1);

    fireEvent.click(await screen.findByText("First paragraph."));
    typeInOpenBlock("First paragraph, edited.");
    fireEvent.blur(screen.getByRole("textbox"));
    await screen.findByText("First paragraph, edited.");

    // `inlineDirty` never clears on a Vault that refuses the write, so without
    // this the page would ignore every later revision for the rest of the
    // session. The edit itself is not written over: the change is flagged.
    setRevision(2);

    expect(
      await screen.findByText(
        /This note changed on disk while your edit was waiting to save/,
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("First paragraph, edited.")).toBeInTheDocument();
  });

  it("holds off the service-worker reload until the edit is saved", async () => {
    const sent = mockVault("First paragraph.\n");

    renderNote("vault-1", { vaults: [] });

    await screen.findByText("First paragraph.");
    expect(isAppReloadHeld()).toBe(false);

    fireEvent.click(screen.getByText("First paragraph."));
    typeInOpenBlock("First paragraph, edited.");
    expect(isAppReloadHeld()).toBe(true);

    fireEvent.blur(screen.getByRole("textbox"));
    await waitFor(() => expect(sent).toHaveLength(1));
    await waitFor(() => expect(isAppReloadHeld()).toBe(false));
  });
});

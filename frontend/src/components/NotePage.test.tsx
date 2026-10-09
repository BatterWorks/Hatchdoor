import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { act } from "react";
import { EditorView } from "@codemirror/view";
import { Link, MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NOTE_PROPERTIES_COLLAPSED_KEY } from "../app/constants";
import { isAppReloadHeld, resetAppReloadHolds } from "../lib/reloadGuard";
import { loadNoteDraft, saveNoteDraft } from "../lib/writeDrafts";
import {
  conflictVault,
  staleVault,
  syncStoppedVault,
} from "../test/fixtures/vaults";
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

describe("NotePage saves through every Vault condition (#372)", () => {
  /** Serves the note and records every save, which the server accepts. */
  function serveWritable(vaultId: string): string[] {
    const saved: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (init?.method === "PUT") {
          saved.push(String(init.body));
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
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: vaultId,
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
          return jsonResponse({ vault_id: vaultId, results: [] });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );
    return saved;
  }

  async function saveFromSourceMode(saved: string[]): Promise<void> {
    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
    const textarea = await screen.findByRole("textbox");
    fireEvent.change(textarea, { target: { value: "Body, edited." } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toContain("Body, edited.");
  }

  it("saves a note in a Vault whose sync stopped on a conflict, and warns on the note the conflict lists", async () => {
    const vault = conflictVault("Beta", ["Home.md"]);
    const saved = serveWritable(vault.vault_id);

    renderNote(vault.vault_id, { vaults: [vault] });

    expect(
      await screen.findByText(/This note is part of a sync conflict/),
    ).toBeInTheDocument();
    expect(screen.queryByText("Not saving")).not.toBeInTheDocument();
    expect(screen.queryByText(/Edits aren.t saving/)).not.toBeInTheDocument();
    await saveFromSourceMode(saved);
  });

  it("saves a note in a Vault whose sync stopped on files changed by hand, with no notice", async () => {
    const vault = syncStoppedVault("Beta");
    const saved = serveWritable(vault.vault_id);

    renderNote(vault.vault_id, { vaults: [vault] });

    await screen.findByRole("button", { name: "Source" });
    expect(screen.queryByText("Not saving")).not.toBeInTheDocument();
    expect(screen.queryByText(/Edits aren.t saving/)).not.toBeInTheDocument();
    expect(
      screen.queryByText(/unsupported local work/),
    ).not.toBeInTheDocument();
    await saveFromSourceMode(saved);
  });

  it("raises nothing beyond the sidebar slot for a stale Vault", async () => {
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
      expect(
        screen.getByRole("button", { name: "Source" }),
      ).toBeInTheDocument();
    });
    expect(screen.queryByText("Not saving")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/The last index build for this Vault failed\./),
    ).not.toBeInTheDocument();
  });
});

describe("NotePage sync conflict notice (ADR-30)", () => {
  function conflictedVault(paths: string[]) {
    return {
      ...staleVault("Conflicted"),
      search: "ready" as const,
      search_error: undefined,
      git: "unavailable" as const,
      git_error: {
        code: "managed_git_conflict",
        message: "managed checkout merge conflict",
        retryable: false,
        detail: {
          kind: "affected_paths" as const,
          paths,
          total: paths.length,
        },
      },
    };
  }

  function serveHome(vaultId: string) {
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
              content: "Body",
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
  }

  it("warns on a note the conflict lists, and leaves it editable", async () => {
    const vault = conflictedVault(["Home.md"]);
    serveHome(vault.vault_id);
    renderNote(vault.vault_id, { vaults: [vault] });
    expect(
      await screen.findByText(/This note is part of a sync conflict/),
    ).toBeInTheDocument();
    expect(screen.queryByText("Not saving")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Source" })).toBeInTheDocument();
  });

  it("says nothing on a note the conflict does not list", async () => {
    const vault = conflictedVault(["Other.md"]);
    serveHome(vault.vault_id);
    renderNote(vault.vault_id, { vaults: [vault] });
    await screen.findByRole("button", { name: "Source" });
    expect(
      screen.queryByText(/This note is part of a sync conflict/),
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
    { refuseWrites = false }: { refuseWrites?: boolean } = {},
  ): Sent[] {
    const sent: Sent[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (init?.method === "PUT") {
          sent.push({ url, init });
          if (refuseWrites) {
            return jsonResponse(
              { code: "vault_unavailable", message: "Vault is unavailable" },
              503,
            );
          }
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

    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
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

    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
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
  it("autosaves an inline edit in a Vault whose sync stopped (#372)", async () => {
    const vault = syncStoppedVault("Beta");
    const sent = mockVault("First paragraph.\n", "hash", vault.vault_id);

    renderNote(vault.vault_id, { vaults: [vault] });

    fireEvent.click(await screen.findByText("First paragraph."));
    typeInOpenBlock("First paragraph, edited.");
    fireEvent.blur(screen.getByRole("textbox"));

    await waitFor(() => expect(sent).toHaveLength(1));
    expect(String(sent[0].init.body)).toContain("First paragraph, edited.");
    expect(screen.queryByText("Not saving")).not.toBeInTheDocument();
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

  it("notices the note changed on disk even while a refused save holds the edit", async () => {
    mockVault("First paragraph.\n", "hash", "vault-1", { refuseWrites: true });
    const { setRevision } = renderNoteAtRevision("vault-1", [], 1);

    fireEvent.click(await screen.findByText("First paragraph."));
    typeInOpenBlock("First paragraph, edited.");
    fireEvent.blur(screen.getByRole("textbox"));
    await screen.findByText("First paragraph, edited.");
    await screen.findByText(/Hatchdoor could not reach the vault/);

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

  // The full source editor keeps its text in a debounced draft, so a reload
  // between a keystroke and that write loses it just as surely (#332).
  it("holds off the service-worker reload while the source editor is open", async () => {
    const sent = mockVault("Body on disk.\n");

    renderNote("vault-1", { vaults: [] });

    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
    const textarea = await screen.findByRole("textbox");
    fireEvent.change(textarea, { target: { value: "Body being typed." } });
    expect(isAppReloadHeld()).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(sent).toHaveLength(1));
    await waitFor(() => expect(isAppReloadHeld()).toBe(false));
  });
});

describe("NotePage conflict review and editing correctness (#331)", () => {
  type Sent = { url: string; init: RequestInit };
  type DiskNote = { content: string; hash: string };

  /**
   * A Vault holding `home` and `other`. Reads return whatever `disk` holds at
   * the time, so a test can move a note on disk mid-edit; a PUT answers 409
   * whenever its expected hash is not the one on disk.
   */
  function mockTwoNotes(disk: Record<string, DiskNote>): Sent[] {
    const sent: Sent[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: "vault-1", results: [] });
        }
        if (url.includes("/attachments") && init?.method === "POST") {
          sent.push({ url, init });
          return jsonResponse({
            vault_id: "vault-1",
            attachment: { relative_path: "Attachments/scan.pdf" },
          });
        }
        const slug = Object.keys(disk).find((name) =>
          url.includes(`/notes/${name}`),
        );
        if (!slug) {
          return jsonResponse({ error: "not found" }, 404);
        }
        if (url.endsWith("/links")) {
          return jsonResponse({
            vault_id: "vault-1",
            outgoing: [],
            backlinks: [],
          });
        }
        if (init?.method === "PUT") {
          sent.push({ url, init });
          const body = JSON.parse(String(init.body)) as {
            content: string;
            expected_content_hash?: string;
          };
          if (
            body.expected_content_hash &&
            body.expected_content_hash !== disk[slug].hash
          ) {
            return jsonResponse({ error: "changed on disk" }, 409);
          }
          disk[slug] = { content: body.content, hash: `${slug}-saved` };
          return jsonResponse({
            vault_id: "vault-1",
            ok: true,
            slug,
            relative_path: `${slug}.md`,
            content_hash: disk[slug].hash,
            quality_warnings: [],
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: null,
            layer: null,
          });
        }
        return jsonResponse({
          vault_id: "vault-1",
          note: {
            title: slug,
            slug,
            relative_path: `${slug}.md`,
            content: disk[slug].content,
            content_hash: disk[slug].hash,
            layer: null,
          },
        });
      },
    );
    return sent;
  }

  function renderWithNav(extra?: React.ReactNode) {
    return render(
      <MemoryRouter initialEntries={["/v/vault-1/n/home"]}>
        <Link to="/v/vault-1/n/other">Go to other</Link>
        {extra}
        <Routes>
          <Route
            path="/v/:vaultId/n/:slug"
            element={
              <NotePage
                onActiveNoteChange={vi.fn()}
                onTagSelect={vi.fn()}
                propertiesCollapsedStorageKey={NOTE_PROPERTIES_COLLAPSED_KEY}
                vaultRevision={null}
                writeEnabled={true}
                editRequestId={0}
                vaults={[]}
              />
            }
          />
        </Routes>
      </MemoryRouter>,
    );
  }

  function openBlock(): HTMLElement {
    const block = document.querySelector<HTMLElement>(
      ".live-editor .cm-content",
    );
    if (!block) {
      throw new Error("no live editor is mounted");
    }
    return block;
  }

  function typeInOpenBlock(text: string): void {
    const view = EditorView.findFromDOM(openBlock());
    if (!view) {
      throw new Error("no CodeMirror view is mounted on the active block");
    }
    act(() => {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: text },
      });
    });
  }

  it("drops the conflict review when the user navigates to another note", async () => {
    const disk = {
      home: { content: "Home body.\n", hash: "home-1" },
      other: { content: "Other body.\n", hash: "other-1" },
    };
    const sent = mockTwoNotes(disk);
    renderWithNav();

    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
    fireEvent.change(await screen.findByRole("textbox"), {
      target: { value: "Home body, edited here.\n" },
    });
    // Someone else writes the note while the editor is open.
    disk.home = { content: "Home body, edited elsewhere.\n", hash: "home-2" };
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByRole("region", { name: "Conflict review" }),
    ).toBeInTheDocument();

    // Leave without resolving it. The editor for the other note must not
    // inherit a review of home's disk version against other's text.
    fireEvent.click(screen.getByRole("link", { name: "Go to other" }));
    expect(await screen.findByText("Other body.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Source" }));
    await screen.findByRole("textbox");
    expect(
      screen.queryByRole("region", { name: "Conflict review" }),
    ).not.toBeInTheDocument();

    // And nothing ever wrote home's slug with other's text.
    const puts = sent.filter((entry) => entry.init.method === "PUT");
    expect(puts).toHaveLength(1);
    expect(disk.home.content).toBe("Home body, edited elsewhere.\n");
  });

  it("drops a conflict fetch that lands after the user has left the note", async () => {
    const disk = {
      home: { content: "Home body.\n", hash: "home-1" },
      other: { content: "Other body.\n", hash: "other-1" },
    };
    mockTwoNotes(disk);
    const fetchMock = vi.mocked(globalThis.fetch);
    const realImpl = fetchMock.getMockImplementation()!;
    let releaseConflictRead: () => void = () => {};
    let putSeen = false;
    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      if (init?.method === "PUT") {
        putSeen = true;
      } else if (putSeen && url.endsWith("/notes/home")) {
        // Hold the disk read that follows the 409 until the user has moved on.
        putSeen = false;
        await new Promise<void>((resolve) => {
          releaseConflictRead = resolve;
        });
      }
      return realImpl(input, init);
    });
    renderWithNav();

    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
    fireEvent.change(await screen.findByRole("textbox"), {
      target: { value: "Home body, edited here.\n" },
    });
    disk.home = { content: "Home body, edited elsewhere.\n", hash: "home-2" };
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putSeen).toBe(false));

    fireEvent.click(screen.getByRole("link", { name: "Go to other" }));
    expect(await screen.findByText("Other body.")).toBeInTheDocument();
    await act(async () => {
      releaseConflictRead();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    fireEvent.click(screen.getByRole("button", { name: "Source" }));
    await screen.findByRole("textbox");
    expect(
      screen.queryByRole("region", { name: "Conflict review" }),
    ).not.toBeInTheDocument();
  });

  it("never restores a draft older than the last inline save into source mode", async () => {
    // A source-mode draft left behind against a version that has since moved.
    saveNoteDraft("vault-1", "home", {
      vaultId: "vault-1",
      slug: "home",
      content: "An old source-mode draft.",
      baseContentHash: "home-0",
      savedAt: Date.now() - 60_000,
    });
    const disk = {
      home: { content: "First paragraph.\n", hash: "home-1" },
      other: { content: "Other body.\n", hash: "other-1" },
    };
    const sent = mockTwoNotes(disk);
    renderWithNav();

    fireEvent.click(await screen.findByText("First paragraph."));
    typeInOpenBlock("First paragraph, saved inline.");
    fireEvent.blur(screen.getByRole("textbox"));
    await waitFor(() => expect(sent).toHaveLength(1));
    await waitFor(() =>
      expect(disk.home.content).toBe("First paragraph, saved inline."),
    );

    fireEvent.click(screen.getByRole("button", { name: "Source" }));
    const textarea = (await screen.findByRole(
      "textbox",
    )) as HTMLTextAreaElement;
    expect(textarea.value).toBe("First paragraph, saved inline.");
    expect(screen.queryByText(/Restored an earlier draft/)).toBeNull();
  });

  // Found in the live pass for #331: the history for the note just opened was
  // seeded while the page still held the previous note, so undoing the first
  // edit there wrote the previous note's whole text over this one.
});

describe("NotePage body before links (#361)", () => {
  function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((done) => {
      resolve = done;
    });
    return { promise, resolve };
  }

  function mockHeldLinks(links: Promise<Response>) {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/notes/home/links")) {
          return links;
        }
        if (url.endsWith("/notes/home")) {
          return jsonResponse({
            vault_id: "vault-1",
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "The body arrives first.",
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
  }

  it("renders the note body while its links read is still open, then fills in the links", async () => {
    const links = deferred<Response>();
    mockHeldLinks(links.promise);

    renderNote("vault-1");

    expect(
      await screen.findByText("The body arrives first."),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Note links")).not.toBeInTheDocument();

    links.resolve(
      jsonResponse({
        vault_id: "vault-1",
        outgoing: [],
        backlinks: [
          {
            vault_id: "vault-1",
            link: {
              title: "Elsewhere",
              slug: "elsewhere",
              relative_path: "Elsewhere",
              layer: null,
            },
          },
        ],
      }),
    );

    expect(await screen.findByLabelText("Note links")).toBeInTheDocument();
    expect(screen.getByText("Elsewhere")).toBeInTheDocument();
  });

  it("keeps the body on screen when the links read fails", async () => {
    const links = deferred<Response>();
    mockHeldLinks(links.promise);

    renderNote("vault-1");
    expect(
      await screen.findByText("The body arrives first."),
    ).toBeInTheDocument();

    await act(async () => {
      links.resolve(jsonResponse({ error: "boom" }, 500));
      await links.promise;
    });

    expect(screen.getByText("The body arrives first.")).toBeInTheDocument();
    expect(screen.queryByLabelText("Note links")).not.toBeInTheDocument();
  });
});

describe("NotePage reading chrome (#530)", () => {
  async function atPhoneWidth(run: () => Promise<void>) {
    const original = window.matchMedia;
    window.matchMedia = ((query: string) => ({
      matches: query.includes("920"),
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    })) as unknown as typeof window.matchMedia;
    try {
      await run();
    } finally {
      window.matchMedia = original;
    }
  }

  function mockNote(vaultId: string, resolveBatch: () => Promise<Response>) {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: vaultId,
            outgoing: [],
            backlinks: [],
          });
        }
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: vaultId,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "---\ntags: [a]\n---\n\n# Home\n\nBody\n\n## Role\n",
              content_hash: "hash",
              layer: null,
            },
          });
        }
        if (url.includes("/resolve-batch")) {
          return resolveBatch();
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );
  }

  it("never flashes the line-mapping notice while wikilinks are still resolving", async () => {
    const vaultId = "vault-1";
    mockNote(vaultId, () => new Promise(() => {}));
    renderNote(vaultId);
    await screen.findByRole("heading", { level: 2, name: "Home" });
    expect(
      screen.queryByText(/source and rendered lines don.t line up/),
    ).not.toBeInTheDocument();
  });

  it("draws the title once and lists only the headings under it", async () => {
    const vaultId = "vault-1";
    mockNote(vaultId, async () =>
      jsonResponse({ vault_id: vaultId, results: [] }),
    );
    renderNote(vaultId);
    await screen.findByRole("heading", { level: 2, name: "Home" });
    // The rendered page is the reading view; the editor is the default.
    fireEvent.click(await screen.findByRole("button", { name: "Reading" }));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { level: 2, name: "Role" }),
      ).toBeVisible(),
    );
    const bodyTitle = document.querySelector(".note-body h1");
    expect(bodyTitle).toHaveAttribute("hidden");
    const toc = screen.getByRole("navigation", { name: "Table of contents" });
    expect(toc).toHaveTextContent("Role");
    expect(toc).not.toHaveTextContent("Home");
  });

  it("remembers the phone's Properties fold under its own key", async () => {
    const vaultId = "vault-1";
    await atPhoneWidth(async () => {
      mockNote(vaultId, async () =>
        jsonResponse({ vault_id: vaultId, results: [] }),
      );
      renderNote(vaultId);
      await screen.findByRole("heading", { level: 2, name: "Home" });
      fireEvent.click(screen.getByRole("button", { name: "Properties" }));
      expect(
        window.localStorage.getItem(`${NOTE_PROPERTIES_COLLAPSED_KEY}.mobile`),
      ).toBe("0");
      expect(
        window.localStorage.getItem(NOTE_PROPERTIES_COLLAPSED_KEY),
      ).toBeNull();
    });
  });

  it("shows the Source button's shortcut hint on desktop", async () => {
    const vaultId = "vault-1";
    mockNote(vaultId, async () =>
      jsonResponse({ vault_id: vaultId, results: [] }),
    );

    renderNote(vaultId);

    const edit = await screen.findByRole("button", { name: "Source" });
    expect(edit.querySelector(".shortcut-hint")).toHaveTextContent("E");
  });

  it("leaves the shortcut hint off the Source button on a phone", async () => {
    const vaultId = "vault-1";
    await atPhoneWidth(async () => {
      mockNote(vaultId, async () =>
        jsonResponse({ vault_id: vaultId, results: [] }),
      );

      renderNote(vaultId);

      const edit = await screen.findByRole("button", { name: "Source" });
      expect(edit.querySelector(".shortcut-hint")).toBeNull();
      expect(edit).toHaveTextContent(/^Source$/);
    });
  });
});

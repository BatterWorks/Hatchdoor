import {
  cleanup,
  createEvent,
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
      ".block-input .cm-content",
    );
    if (!block) {
      throw new Error("no block is open");
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

    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
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
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
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

    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
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

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await screen.findByRole("textbox");
    expect(
      screen.queryByRole("region", { name: "Conflict review" }),
    ).not.toBeInTheDocument();
  });

  it("does not run document undo for Ctrl+Z typed into another text field", async () => {
    const disk = {
      home: { content: "First paragraph.\n", hash: "home-1" },
      other: { content: "Other body.\n", hash: "other-1" },
    };
    const sent = mockTwoNotes(disk);
    renderWithNav(
      <>
        <input aria-label="Search box" />
        <textarea aria-label="Property box" />
      </>,
    );

    fireEvent.click(await screen.findByText("First paragraph."));
    typeInOpenBlock("First paragraph, edited.");
    fireEvent.blur(openBlock());
    await screen.findByText("First paragraph, edited.");
    await waitFor(() => expect(sent).toHaveLength(1));

    const input = screen.getByLabelText("Search box");
    const inputUndo = fireEvent.keyDown(input, { key: "z", ctrlKey: true });
    const textareaUndo = fireEvent.keyDown(
      screen.getByLabelText("Property box"),
      { key: "z", metaKey: true },
    );

    // The field keeps its own native undo, and the note is left alone.
    expect(inputUndo).toBe(true);
    expect(textareaUndo).toBe(true);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.getByText("First paragraph, edited.")).toBeInTheDocument();
    expect(sent).toHaveLength(1);

    // Outside any field, document undo still answers.
    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    expect(await screen.findByText("First paragraph.")).toBeInTheDocument();
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
      expect(disk.home.content).toBe("First paragraph, saved inline.\n"),
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const textarea = (await screen.findByRole(
      "textbox",
    )) as HTMLTextAreaElement;
    expect(textarea.value).toBe("First paragraph, saved inline.\n");
    expect(screen.queryByText(/Restored an earlier draft/)).toBeNull();
  });

  it("places a dropped attachment after the block aimed at when the open block gained lines", async () => {
    const disk = {
      home: {
        content: "First paragraph.\n\nSecond paragraph.\n",
        hash: "home-1",
      },
      other: { content: "Other body.\n", hash: "other-1" },
    };
    const sent = mockTwoNotes(disk);
    // jsdom lays nothing out, so each block is given a band by its first line.
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        const start = Number(this.dataset.startLine);
        const top = Number.isFinite(start) ? start * 100 : 0;
        return {
          top,
          bottom: top + 50,
          left: 0,
          right: 100,
          width: 100,
          height: 50,
          x: 0,
          y: top,
          toJSON: () => ({}),
        } as DOMRect;
      },
    );
    const { container } = renderWithNav();

    fireEvent.click(await screen.findByText("First paragraph."));
    // Still open when the file lands: the drop's own blur commits it, and the
    // commit turns one line into two.
    typeInOpenBlock("First line.\nAn added line.");

    const file = new File(["%PDF-1.4"], "scan.pdf", {
      type: "application/pdf",
    });
    const dropZone = container.querySelector(".note-body-drop")!;
    // jsdom has no DragEvent, so the coordinates are put on by hand.
    const drop = createEvent.drop(dropZone);
    Object.defineProperty(drop, "clientY", { value: 320 });
    Object.defineProperty(drop, "dataTransfer", {
      value: { files: [file], types: ["Files"] },
    });
    fireEvent(dropZone, drop);

    await waitFor(() =>
      expect(
        sent.filter((entry) => entry.init.method === "PUT").length,
      ).toBeGreaterThan(0),
    );
    await waitFor(() =>
      expect(disk.home.content).toContain("![[Attachments/scan.pdf]]"),
    );
    expect(disk.home.content).toBe(
      "First line.\nAn added line.\n\nSecond paragraph.\n\n![[Attachments/scan.pdf]]\n",
    );
  });

  // Found in the live pass for #331: the history for the note just opened was
  // seeded while the page still held the previous note, so undoing the first
  // edit there wrote the previous note's whole text over this one.
  it("never undoes into the text of the note that was open before", async () => {
    const disk = {
      home: { content: "Home body.\n", hash: "home-1" },
      other: { content: "Other body.\n", hash: "other-1" },
    };
    const sent = mockTwoNotes(disk);
    renderWithNav();

    await screen.findByText("Home body.");
    fireEvent.click(screen.getByRole("link", { name: "Go to other" }));
    fireEvent.click(await screen.findByText("Other body."));
    typeInOpenBlock("Other body, edited.");
    fireEvent.blur(openBlock());
    await waitFor(() =>
      expect(disk.other.content).toBe("Other body, edited.\n"),
    );

    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    await waitFor(() => expect(sent).toHaveLength(2));
    await waitFor(() => expect(disk.other.content).toBe("Other body.\n"));
    expect(disk.home.content).toBe("Home body.\n");
  });
});

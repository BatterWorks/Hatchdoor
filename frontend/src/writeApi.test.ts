import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "./api";
import {
  archiveNote,
  createNote,
  deleteNote,
  describeWriteOutcome,
  getWriteCapabilities,
  moveNote,
  renameNote,
  updateNote,
  uploadAttachment,
} from "./writeApi";
import type { WriteOutcome } from "./types";

function outcome(overrides: Partial<WriteOutcome> = {}): WriteOutcome {
  return {
    ok: true,
    slug: "home",
    relative_path: "Home.md",
    content_hash: "hash",
    quality_warnings: [],
    git_sync_warning: null,
    rewritten_notes: 0,
    moved_assets: 0,
    trashed_path: null,
    ...overrides,
  };
}

vi.mock("./api", () => ({
  apiFetch: vi.fn(),
}));

const mockedApiFetch = vi.mocked(apiFetch);

afterEach(() => {
  vi.clearAllMocks();
});

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "content-type": "application/json",
    },
  });
}

function expectJsonCall(
  callIndex: number,
  expectedUrl: string,
  expectedMethod: string,
  expectedBody: unknown,
): void {
  const [url, init] = mockedApiFetch.mock.calls[callIndex] ?? [];
  expect(url).toBe(expectedUrl);
  expect(init?.method).toBe(expectedMethod);
  expect(init?.headers).toMatchObject({ "content-type": "application/json" });
  expect(JSON.parse(String(init?.body))).toEqual(expectedBody);
}

describe("writeApi", () => {
  it("loads write capabilities", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({ enabled: true, warnings: ["read-only mode off"] }),
    );

    await expect(getWriteCapabilities()).resolves.toEqual({
      enabled: true,
      warnings: ["read-only mode off"],
    });

    expect(mockedApiFetch).toHaveBeenCalledWith("/api/write-capabilities");
  });

  it("sends the expected create/update/rename/move/archive/delete write requests", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        ok: true,
        slug: "home",
        relative_path: "Home.md",
        content_hash: "hash-create",
        git_sync_warning: null,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: null,
      }),
    );
    await createNote("Notes/Home.md", "# Home");
    expectJsonCall(0, "/api/note", "POST", {
      relative_path: "Notes/Home.md",
      content: "# Home",
    });

    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        ok: true,
        slug: "folder/alpha beta",
        relative_path: "Folder/Alpha Beta.md",
        content_hash: "hash-update",
        git_sync_warning: null,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: null,
      }),
    );
    await updateNote("folder/alpha beta", "# Updated", "hash-1");
    expectJsonCall(1, "/api/note/folder%2Falpha%20beta", "PUT", {
      content: "# Updated",
      expected_content_hash: "hash-1",
    });

    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        ok: true,
        slug: "folder/alpha beta",
        relative_path: "Folder/Renamed.md",
        content_hash: "hash-rename",
        git_sync_warning: null,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: null,
      }),
    );
    await renameNote("folder/alpha beta", "Renamed Note", "hash-2");
    expectJsonCall(2, "/api/note/folder%2Falpha%20beta/rename", "PATCH", {
      new_title: "Renamed Note",
      expected_content_hash: "hash-2",
    });

    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        ok: true,
        slug: "folder/alpha beta",
        relative_path: "Archive/Renamed.md",
        content_hash: "hash-move",
        git_sync_warning: null,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: null,
      }),
    );
    await moveNote("folder/alpha beta", "Archive/2026", "hash-3");
    expectJsonCall(3, "/api/note/folder%2Falpha%20beta/move", "PATCH", {
      target_folder: "Archive/2026",
      expected_content_hash: "hash-3",
    });

    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        ok: true,
        slug: "folder/alpha beta",
        relative_path: "90-archive/Renamed.md",
        content_hash: "hash-archive",
        git_sync_warning: null,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: null,
      }),
    );
    await archiveNote("folder/alpha beta", "hash-4");
    expectJsonCall(4, "/api/note/folder%2Falpha%20beta/archive", "PATCH", {
      expected_content_hash: "hash-4",
    });

    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        ok: true,
        slug: "folder/alpha beta",
        relative_path: null,
        content_hash: null,
        git_sync_warning: null,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: "90-archive/Renamed.md",
      }),
    );
    await deleteNote("folder/alpha beta", "hash-5");
    expectJsonCall(5, "/api/note/folder%2Falpha%20beta", "DELETE", {
      expected_content_hash: "hash-5",
    });
  });

  it("uploads an attachment as multipart form data", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        ok: true,
        attachment: {
          relative_path: "Attachments/pasted.png",
          size_bytes: 9,
          content_hash: "fnv1a64:test",
        },
        git_sync_warning: null,
        rewritten_notes: 0,
        trashed_path: null,
        cleanup_warning: null,
      }),
    );

    const file = new File(["png-bytes"], "pasted.png", { type: "image/png" });
    await uploadAttachment(file, "Attachments/pasted.png");

    const [url, init] = mockedApiFetch.mock.calls[0] ?? [];
    expect(url).toBe("/api/attachment");
    expect(init?.method).toBe("POST");
    expect(init?.body).toBeInstanceOf(FormData);
    expect((init?.body as FormData).get("target_relative_path")).toBe(
      "Attachments/pasted.png",
    );
    expect((init?.body as FormData).get("file")).toBe(file);
  });

  it("summarizes write outcome side effects", () => {
    expect(describeWriteOutcome(outcome())).toBeNull();
    expect(
      describeWriteOutcome(outcome({ git_sync_warning: "push failed" })),
    ).toBe("Git sync warning: push failed");
    expect(
      describeWriteOutcome(
        outcome({ quality_warnings: ["added final newline"] }),
      ),
    ).toBe("Write quality: added final newline");
    expect(describeWriteOutcome(outcome({ rewritten_notes: 1 }))).toBe(
      "Updated 1 linking note.",
    );
    expect(
      describeWriteOutcome(outcome({ rewritten_notes: 3, moved_assets: 2 })),
    ).toBe("Updated 3 linking notes. Moved 2 assets.");
    expect(
      describeWriteOutcome(outcome({ trashed_path: "90-archive/Home.md" })),
    ).toBe("Moved to trash: 90-archive/Home.md.");
  });

  it("maps conflict and write errors to named exceptions", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({ error: "changed" }, 409),
    );
    await expect(createNote("Home", "# Home")).rejects.toMatchObject({
      name: "ConflictError",
      message: "changed",
    });

    mockedApiFetch.mockResolvedValueOnce(jsonResponse({ error: "boom" }, 500));
    await expect(deleteNote("home", "hash-1")).rejects.toMatchObject({
      name: "WriteApiError",
      message: "boom",
    });
  });
});

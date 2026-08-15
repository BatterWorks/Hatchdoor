import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "./api";
import {
  archiveNote,
  createNote,
  deleteNote,
  describeWriteOutcome,
  getWriteCapabilities,
  isDemoReadOnlyError,
  moveNote,
  renameNote,
  MUTATION_FETCH_TIMEOUT_MS,
  updateNote,
  uploadAttachment,
  UPLOAD_FETCH_TIMEOUT_MS,
} from "./writeApi";
import type { WriteOutcome } from "../types";

const VAULT_ID = "00000000-0000-4000-8000-000000000001";

function outcome(overrides: Partial<WriteOutcome> = {}): WriteOutcome {
  return {
    vault_id: VAULT_ID,
    ok: true,
    slug: "home",
    relative_path: "Home.md",
    content_hash: "hash",
    quality_warnings: [],
    rewritten_notes: 0,
    moved_assets: 0,
    trashed_path: null,
    layer: null,
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
  it("loads write capabilities for one Vault", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        vault_id: VAULT_ID,
        enabled: true,
        warnings: ["read-only mode off"],
      }),
    );

    await expect(getWriteCapabilities(VAULT_ID)).resolves.toEqual({
      vault_id: VAULT_ID,
      enabled: true,
      warnings: ["read-only mode off"],
    });

    expect(mockedApiFetch).toHaveBeenCalledWith(
      `/api/v1/vaults/${VAULT_ID}/write-capabilities`,
    );
  });

  it("sends the expected create/update/rename/move/archive/delete write requests", async () => {
    mockedApiFetch.mockResolvedValueOnce(jsonResponse(outcome()));
    await createNote(VAULT_ID, "Notes/Home.md", "# Home");
    expectJsonCall(0, `/api/v1/vaults/${VAULT_ID}/notes`, "POST", {
      relative_path: "Notes/Home.md",
      content: "# Home",
    });

    mockedApiFetch.mockResolvedValueOnce(jsonResponse(outcome()));
    await updateNote(VAULT_ID, "folder/alpha beta", "# Updated", "hash-1");
    expectJsonCall(
      1,
      `/api/v1/vaults/${VAULT_ID}/notes/folder%2Falpha%20beta`,
      "PUT",
      { content: "# Updated", expected_content_hash: "hash-1" },
    );

    mockedApiFetch.mockResolvedValueOnce(jsonResponse(outcome()));
    await renameNote(VAULT_ID, "folder/alpha beta", "Renamed Note", "hash-2");
    expectJsonCall(
      2,
      `/api/v1/vaults/${VAULT_ID}/notes/folder%2Falpha%20beta/rename`,
      "PATCH",
      { new_title: "Renamed Note", expected_content_hash: "hash-2" },
    );

    mockedApiFetch.mockResolvedValueOnce(jsonResponse(outcome()));
    await moveNote(VAULT_ID, "folder/alpha beta", "Archive/2026", "hash-3");
    expectJsonCall(
      3,
      `/api/v1/vaults/${VAULT_ID}/notes/folder%2Falpha%20beta/move`,
      "PATCH",
      { target_folder: "Archive/2026", expected_content_hash: "hash-3" },
    );

    mockedApiFetch.mockResolvedValueOnce(jsonResponse(outcome()));
    await archiveNote(VAULT_ID, "folder/alpha beta", "hash-4");
    expectJsonCall(
      4,
      `/api/v1/vaults/${VAULT_ID}/notes/folder%2Falpha%20beta/archive`,
      "PATCH",
      { expected_content_hash: "hash-4" },
    );

    mockedApiFetch.mockResolvedValueOnce(jsonResponse(outcome()));
    await deleteNote(VAULT_ID, "folder/alpha beta", "hash-5");
    expectJsonCall(
      5,
      `/api/v1/vaults/${VAULT_ID}/notes/folder%2Falpha%20beta`,
      "DELETE",
      { expected_content_hash: "hash-5" },
    );
  });

  it("uploads an attachment to one Vault as multipart form data", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        vault_id: VAULT_ID,
        ok: true,
        attachment: {
          relative_path: "Attachments/pasted.png",
          size_bytes: 9,
          content_hash: "fnv1a64:test",
          layer: null,
        },
        rewritten_notes: 0,
        trashed_path: null,
        cleanup_warning: null,
      }),
    );

    const file = new File(["png-bytes"], "pasted.png", { type: "image/png" });
    await uploadAttachment(VAULT_ID, file, "Attachments/pasted.png");

    const [url, init] = mockedApiFetch.mock.calls[0] ?? [];
    expect(url).toBe(`/api/v1/vaults/${VAULT_ID}/attachments`);
    expect(init?.method).toBe("POST");
    expect(init?.body).toBeInstanceOf(FormData);
    expect((init?.body as FormData).get("target_relative_path")).toBe(
      "Attachments/pasted.png",
    );
    expect((init?.body as FormData).get("file")).toBe(file);
  });

  it("gives attachment uploads a timeout large enough for big files on slow links", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse({
        vault_id: VAULT_ID,
        ok: true,
        attachment: {
          relative_path: "Attachments/manual.pdf",
          size_bytes: 9,
          content_hash: "fnv1a64:test",
          layer: null,
        },
        rewritten_notes: 0,
        trashed_path: null,
        cleanup_warning: null,
      }),
    );

    const file = new File(["pdf-bytes"], "manual.pdf", {
      type: "application/pdf",
    });
    await uploadAttachment(VAULT_ID, file, "Attachments/manual.pdf");

    const [, init] = mockedApiFetch.mock.calls[0] ?? [];
    expect(init?.timeoutMs).toBe(UPLOAD_FETCH_TIMEOUT_MS);
    expect(UPLOAD_FETCH_TIMEOUT_MS).toBeGreaterThanOrEqual(120_000);
  });

  it("gives note mutations a longer timeout than reads", async () => {
    mockedApiFetch.mockResolvedValueOnce(jsonResponse(outcome()));
    await updateNote(VAULT_ID, "home", "# Updated", "hash-1");

    const [, init] = mockedApiFetch.mock.calls[0] ?? [];
    expect(init?.timeoutMs).toBe(MUTATION_FETCH_TIMEOUT_MS);
    expect(MUTATION_FETCH_TIMEOUT_MS).toBeGreaterThanOrEqual(60_000);
  });

  it("summarizes write outcome side effects", () => {
    expect(describeWriteOutcome(outcome())).toBeNull();
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
      jsonResponse(
        { code: "write_conflict", message: "changed", retryable: true },
        409,
      ),
    );
    await expect(createNote(VAULT_ID, "Home", "# Home")).rejects.toMatchObject({
      name: "ConflictError",
      message: "changed",
    });

    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse(
        { code: "internal_error", message: "boom", retryable: false },
        500,
      ),
    );
    await expect(deleteNote(VAULT_ID, "home", "hash-1")).rejects.toMatchObject({
      name: "WriteApiError",
      message: "boom",
    });
  });

  it("carries the demo_read_only code on the thrown error so callers can detect it (#152)", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      jsonResponse(
        {
          code: "demo_read_only",
          message:
            "This is a public read-only demo instance; mutations and Vault-control operations are disabled.",
          retryable: false,
        },
        403,
      ),
    );

    let caught: unknown;
    try {
      await createNote(VAULT_ID, "Home", "# Home");
    } catch (error) {
      caught = error;
    }

    expect(isDemoReadOnlyError(caught)).toBe(true);
    expect(isDemoReadOnlyError(new Error("unrelated"))).toBe(false);
  });

  it("explains common non-JSON infrastructure errors without relying on statusText", async () => {
    mockedApiFetch.mockResolvedValueOnce(
      new Response("<html>bad gateway</html>", {
        status: 502,
        statusText: "",
        headers: { "content-type": "text/html" },
      }),
    );

    await expect(createNote(VAULT_ID, "Home", "# Home")).rejects.toMatchObject({
      name: "WriteApiError",
      message: "502 Bad Gateway",
    });

    mockedApiFetch.mockResolvedValueOnce(
      new Response("too large", {
        status: 413,
        statusText: "",
        headers: { "content-type": "text/plain" },
      }),
    );

    await expect(
      uploadAttachment(VAULT_ID, new File(["x"], "x.png"), "x.png"),
    ).rejects.toMatchObject({
      name: "WriteApiError",
      message: "413 Payload Too Large",
    });
  });
});

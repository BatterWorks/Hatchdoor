import { apiFetch } from "./api";
import type {
  AttachmentOutcome,
  VaultId,
  WriteCapabilities,
  WriteOutcome,
} from "../types";

/** Uploads can carry files up to the server's 10 MB cap; on a slow uplink that
 * legitimately takes minutes, so they must not inherit the short read timeout. */
export const UPLOAD_FETCH_TIMEOUT_MS = 300_000;
/** Mutations rebuild the vault index server-side under the write lock, which on
 * a large vault can exceed the read timeout. Aborting early is worse than
 * waiting: the server still applies the write, so a retry hits a conflict. */
export const MUTATION_FETCH_TIMEOUT_MS = 60_000;

/**
 * Build a human-readable summary of the side effects reported by a write so
 * they can be surfaced to the user. Returns null when a write completed
 * cleanly with nothing worth announcing.
 */
export function describeWriteOutcome(outcome: WriteOutcome): string | null {
  const parts: string[] = [];

  const qualityWarnings = outcome.quality_warnings ?? [];
  if (qualityWarnings.length > 0) {
    parts.push(`Write quality: ${qualityWarnings.join(" ")}`);
  }
  if (outcome.rewritten_notes > 0) {
    parts.push(
      `Updated ${outcome.rewritten_notes} linking note${
        outcome.rewritten_notes === 1 ? "" : "s"
      }.`,
    );
  }
  if (outcome.moved_assets > 0) {
    parts.push(
      `Moved ${outcome.moved_assets} asset${
        outcome.moved_assets === 1 ? "" : "s"
      }.`,
    );
  }
  if (outcome.trashed_path) {
    parts.push(`Moved to trash: ${outcome.trashed_path}.`);
  }

  return parts.length > 0 ? parts.join(" ") : null;
}

async function parseError(res: Response): Promise<string> {
  try {
    const json = (await res.json()) as { message?: unknown };
    if (typeof json.message === "string" && json.message.trim().length > 0) {
      return json.message;
    }
  } catch {
    // Fall back to status text below.
  }
  return `${res.status} ${statusFallbackText(res)}`.trim();
}

function statusFallbackText(res: Response): string {
  if (res.statusText.trim()) {
    return res.statusText.trim();
  }
  switch (res.status) {
    case 400:
      return "Bad Request";
    case 401:
      return "Unauthorized";
    case 403:
      return "Forbidden";
    case 404:
      return "Not Found";
    case 409:
      return "Conflict";
    case 413:
      return "Payload Too Large";
    case 500:
      return "Internal Server Error";
    case 502:
      return "Bad Gateway";
    case 503:
      return "Service Unavailable";
    case 504:
      return "Gateway Timeout";
    default:
      return "HTTP Error";
  }
}

function makeWriteError(message: string, status: number): Error {
  const error = new Error(message);
  error.name = status === 409 ? "ConflictError" : "WriteApiError";
  return error;
}

async function requestJson<T>(url: string, init: RequestInit): Promise<T> {
  const res = await apiFetch(url, {
    ...init,
    timeoutMs: MUTATION_FETCH_TIMEOUT_MS,
    headers: {
      "content-type": "application/json",
      ...((init.headers as Record<string, string>) ?? {}),
    },
  });
  if (!res.ok) {
    throw makeWriteError(await parseError(res), res.status);
  }
  return (await res.json()) as T;
}

function vaultNotesUrl(vaultId: VaultId): string {
  return `/api/v1/vaults/${encodeURIComponent(vaultId)}/notes`;
}

function vaultNoteUrl(vaultId: VaultId, slug: string): string {
  return `${vaultNotesUrl(vaultId)}/${encodeURIComponent(slug)}`;
}

export async function getWriteCapabilities(
  vaultId: VaultId,
): Promise<WriteCapabilities> {
  const res = await apiFetch(
    `/api/v1/vaults/${encodeURIComponent(vaultId)}/write-capabilities`,
  );
  if (!res.ok) {
    throw makeWriteError(await parseError(res), res.status);
  }
  return (await res.json()) as WriteCapabilities;
}

export function createNote(
  vaultId: VaultId,
  relativePath: string,
  content: string,
): Promise<WriteOutcome> {
  return requestJson(vaultNotesUrl(vaultId), {
    method: "POST",
    body: JSON.stringify({ relative_path: relativePath, content }),
  });
}

export async function uploadAttachment(
  vaultId: VaultId,
  file: File,
  targetRelativePath: string,
): Promise<AttachmentOutcome> {
  const form = new FormData();
  form.set("target_relative_path", targetRelativePath);
  form.set("file", file);
  const res = await apiFetch(
    `/api/v1/vaults/${encodeURIComponent(vaultId)}/attachments`,
    {
      method: "POST",
      body: form,
      timeoutMs: UPLOAD_FETCH_TIMEOUT_MS,
    },
  );
  if (!res.ok) {
    throw makeWriteError(await parseError(res), res.status);
  }
  return (await res.json()) as AttachmentOutcome;
}

export function updateNote(
  vaultId: VaultId,
  slug: string,
  content: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(vaultNoteUrl(vaultId, slug), {
    method: "PUT",
    body: JSON.stringify({
      content,
      expected_content_hash: expectedContentHash,
    }),
  });
}

export function renameNote(
  vaultId: VaultId,
  slug: string,
  newTitle: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`${vaultNoteUrl(vaultId, slug)}/rename`, {
    method: "PATCH",
    body: JSON.stringify({
      new_title: newTitle,
      expected_content_hash: expectedContentHash,
    }),
  });
}

export function moveNote(
  vaultId: VaultId,
  slug: string,
  targetFolder: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`${vaultNoteUrl(vaultId, slug)}/move`, {
    method: "PATCH",
    body: JSON.stringify({
      target_folder: targetFolder,
      expected_content_hash: expectedContentHash,
    }),
  });
}

export function archiveNote(
  vaultId: VaultId,
  slug: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`${vaultNoteUrl(vaultId, slug)}/archive`, {
    method: "PATCH",
    body: JSON.stringify({ expected_content_hash: expectedContentHash }),
  });
}

export function deleteNote(
  vaultId: VaultId,
  slug: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(vaultNoteUrl(vaultId, slug), {
    method: "DELETE",
    body: JSON.stringify({ expected_content_hash: expectedContentHash }),
  });
}

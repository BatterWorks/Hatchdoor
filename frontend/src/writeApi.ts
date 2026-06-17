import { apiFetch } from "./api";
import type { WriteCapabilities, WriteOutcome } from "./types";

async function parseError(res: Response): Promise<string> {
  try {
    const json = (await res.json()) as { error?: unknown };
    if (typeof json.error === "string") {
      return json.error;
    }
  } catch {
    // Fall back to status text below.
  }
  return `${res.status} ${res.statusText}`.trim();
}

function makeWriteError(message: string, status: number): Error {
  const error = new Error(message);
  error.name = status === 409 ? "ConflictError" : "WriteApiError";
  return error;
}

async function requestJson<T>(url: string, init: RequestInit): Promise<T> {
  const res = await apiFetch(url, {
    ...init,
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

export async function getWriteCapabilities(): Promise<WriteCapabilities> {
  const res = await apiFetch("/api/write-capabilities");
  if (!res.ok) {
    throw makeWriteError(await parseError(res), res.status);
  }
  return (await res.json()) as WriteCapabilities;
}

export function createNote(
  relativePath: string,
  content: string,
): Promise<WriteOutcome> {
  return requestJson("/api/note", {
    method: "POST",
    body: JSON.stringify({ relative_path: relativePath, content }),
  });
}

export function updateNote(
  slug: string,
  content: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`/api/note/${encodeURIComponent(slug)}`, {
    method: "PUT",
    body: JSON.stringify({
      content,
      expected_content_hash: expectedContentHash,
    }),
  });
}

export function renameNote(
  slug: string,
  newTitle: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`/api/note/${encodeURIComponent(slug)}/rename`, {
    method: "PATCH",
    body: JSON.stringify({
      new_title: newTitle,
      expected_content_hash: expectedContentHash,
    }),
  });
}

export function moveNote(
  slug: string,
  targetFolder: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`/api/note/${encodeURIComponent(slug)}/move`, {
    method: "PATCH",
    body: JSON.stringify({
      target_folder: targetFolder,
      expected_content_hash: expectedContentHash,
    }),
  });
}

export function deleteNote(
  slug: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`/api/note/${encodeURIComponent(slug)}`, {
    method: "DELETE",
    body: JSON.stringify({ expected_content_hash: expectedContentHash }),
  });
}

/**
 * Extract the server's structured `VaultApiError` `message` from a failed
 * response (`{code, message, vault_id?, retryable}`), falling back to a
 * status-based message when the body is absent or not the expected shape.
 * Mirrors writeApi's `parseError` so the read path surfaces the same backend
 * diagnostics (e.g. "Note not found: <slug>") as the write path instead of a
 * bare status code.
 */
export async function readErrorMessage(
  res: Response,
  fallback: string,
): Promise<string> {
  try {
    const json = (await res.json()) as { message?: unknown };
    if (typeof json.message === "string" && json.message.trim().length > 0) {
      return json.message;
    }
  } catch {
    // Body was not JSON — fall through to the status-based message.
  }
  return `${fallback}: ${res.status}`;
}

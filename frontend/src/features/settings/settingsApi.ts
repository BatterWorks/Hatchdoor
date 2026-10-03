// The settings requests Settings sends, shared with the first-run checklist
// (#419) so both send exactly the same thing.

import { apiFetch } from "../../api/api";

/** `PATCH /api/settings` with the changed keys and any accepted consequence. */
export function patchSettings(
  updates: Record<string, string>,
  confirm: string[] = [],
): Promise<Response> {
  return apiFetch("/api/settings", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ updates, confirm }),
  });
}

/** A new MCP password candidate, not in use until it is saved. */
export function generateMcpTokenCandidate(): Promise<Response> {
  return apiFetch("/api/settings/mcp-token/generate", { method: "POST" });
}

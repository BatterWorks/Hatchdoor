// The settings requests Settings sends, shared with the first-run checklist
// (#419) so both send exactly the same thing.

import { apiFetch } from "../../api/api";

/** Sent on `window` after a save the server accepted, so what reads a
 * setting outside Settings, such as the sidebar's lists (#684), reads it
 * again without a reload. */
export const SETTINGS_SAVED_EVENT = "hatchdoor:settings-saved";

/** `PATCH /api/settings` with the changed keys and any accepted consequence. */
export async function patchSettings(
  updates: Record<string, string>,
  confirm: string[] = [],
): Promise<Response> {
  const response = await apiFetch("/api/settings", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ updates, confirm }),
  });
  if (response.ok) {
    window.dispatchEvent(new Event(SETTINGS_SAVED_EVENT));
  }
  return response;
}

/** A new MCP password candidate, not in use until it is saved. */
export function generateMcpTokenCandidate(): Promise<Response> {
  return apiFetch("/api/settings/mcp-token/generate", { method: "POST" });
}

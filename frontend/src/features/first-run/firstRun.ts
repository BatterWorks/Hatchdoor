// What the first-run checklist (#419) knows and does: whether to show it, what
// this browser remembers, and the existing settings requests its steps reuse.

import { useSyncExternalStore } from "react";

import { apiFetch } from "../../api/api";
import { generateMcpTokenCandidate, patchSettings } from "../settings";
import { safeGetItem, safeRemoveItem, safeSetItem } from "../../lib/storage";
import type { LastAgentConnection } from "../../types";

/** The public demo step 1 offers to anyone who wants example Vaults. */
export const DEMO_URL = "https://hatchdoor.battercloud.cc";

/** Set once this browser closed the checklist. */
export const DISMISSED_KEY = "hatchdoor_first_run_dismissed";
/** The search that proved the notes are indexed, as JSON. */
export const SEARCH_KEY = "hatchdoor_first_run_search";

/** A search in this browser that found something. */
export type SearchProof = { query: string; count: number };

type FirstRunState = {
  dismissed: boolean;
  /** Reopened from Help in this visit, which shows it on any install. */
  reopened: boolean;
  search: SearchProof | null;
};

function readSearch(): SearchProof | null {
  const raw = safeGetItem(SEARCH_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<SearchProof>;
    return typeof parsed.query === "string" && typeof parsed.count === "number"
      ? { query: parsed.query, count: parsed.count }
      : null;
  } catch {
    return null;
  }
}

function initialState(): FirstRunState {
  return {
    dismissed: safeGetItem(DISMISSED_KEY) === "1",
    reopened: false,
    search: readSearch(),
  };
}

// The browser's storage is read once and then mirrored here, so a storage
// failure still lets the checklist close for this visit; it simply shows
// again on the next load.
let state: FirstRunState = initialState();
const listeners = new Set<() => void>();

function update(next: Partial<FirstRunState>): void {
  state = { ...state, ...next };
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useFirstRunState(): FirstRunState {
  return useSyncExternalStore(subscribe, () => state);
}

/** Close the checklist in this browser. */
export function dismissFirstRun(): void {
  safeSetItem(DISMISSED_KEY, "1");
  update({ dismissed: true, reopened: false });
}

/** Help's "Setup checklist" entry. */
export function reopenFirstRun(): void {
  safeRemoveItem(DISMISSED_KEY);
  update({ dismissed: false, reopened: true });
}

/** A search ran in this browser and found something (step 4). The first
 * proof is kept; later searches change nothing. */
export function recordSearchResults(query: string, count: number): void {
  const trimmed = query.trim();
  if (state.search || !trimmed || count <= 0) return;
  const search = { query: trimmed, count };
  safeSetItem(SEARCH_KEY, JSON.stringify(search));
  update({ search });
}

/** Re-read storage, for tests. */
export function resetFirstRunForTests(): void {
  state = initialState();
  listeners.forEach((listener) => listener());
}

/** Whether the checklist takes the place of the workspace's empty screen.
 * Never in demo mode; on its own only on a fresh install; otherwise only
 * when reopened from Help. */
export function shouldShowFirstRun({
  demoMode,
  freshInstall,
  dismissed,
  reopened,
}: {
  demoMode: boolean;
  freshInstall: boolean;
  dismissed: boolean;
  reopened: boolean;
}): boolean {
  if (demoMode || dismissed) return false;
  return freshInstall || reopened;
}

/** `fresh_install` from `GET /api/v1/whats-new` (#424), or `false` when it
 * cannot be read: a checklist shown to an upgraded install by mistake is
 * worse than one a fresh install has to open from Help. */
export async function fetchFreshInstall(
  signal?: AbortSignal,
): Promise<boolean> {
  try {
    const response = await apiFetch("/api/v1/whats-new", { signal });
    if (!response.ok) return false;
    const payload = (await response.json()) as { fresh_install?: unknown };
    return payload.fresh_install === true;
  } catch {
    return false;
  }
}

/** One setting as `GET /api/settings` reports it. */
type SettingRecord = {
  key: string;
  value: string | null;
  configured?: boolean;
  locked: string | null;
};

/** What the connect step and the update-check row read from the settings. */
export type AgentSetup = {
  mcpEnabled: boolean;
  mcpLocked: boolean;
  writesEnabled: boolean;
  writesLocked: boolean;
  /** Set in the server's configuration file, so it cannot be made here. */
  tokenLocked: boolean;
  publicUrl: string;
  updateCheckEnabled: boolean;
  updateCheckLocked: boolean;
  lastAgent: LastAgentConnection | null;
};

const MCP_ENABLED = "HATCHDOOR_MCP_ENABLED";
const MCP_WRITE_ENABLED = "HATCHDOOR_MCP_WRITE_ENABLED";
const MCP_TOKEN = "HATCHDOOR_MCP_BEARER_TOKEN";
const PUBLIC_URL = "HATCHDOOR_PUBLIC_URL";
const UPDATE_CHECK = "HATCHDOOR_UPDATE_CHECK_ENABLED";

/** The server's own truthy spellings (`is_truthy`), since a value from the
 * configuration file arrives as written there. */
function isOn(value: string | null | undefined): boolean {
  return ["1", "true", "yes", "on"].includes(
    (value ?? "").trim().toLowerCase(),
  );
}

function toSetup(payload: {
  settings?: SettingRecord[];
  last_agent?: LastAgentConnection | null;
}): AgentSetup {
  const byKey = new Map(
    (payload.settings ?? []).map((item) => [item.key, item]),
  );
  const get = (key: string) => byKey.get(key);
  return {
    mcpEnabled: isOn(get(MCP_ENABLED)?.value),
    mcpLocked: Boolean(get(MCP_ENABLED)?.locked),
    writesEnabled: isOn(get(MCP_WRITE_ENABLED)?.value),
    writesLocked: Boolean(get(MCP_WRITE_ENABLED)?.locked),
    tokenLocked: Boolean(get(MCP_TOKEN)?.locked),
    publicUrl: (get(PUBLIC_URL)?.value ?? "").trim(),
    updateCheckEnabled: isOn(get(UPDATE_CHECK)?.value),
    updateCheckLocked: Boolean(get(UPDATE_CHECK)?.locked),
    lastAgent: payload.last_agent ?? null,
  };
}

export type SetupResult<T> =
  { ok: true; value: T } | { ok: false; message: string };

export async function fetchAgentSetup(): Promise<SetupResult<AgentSetup>> {
  try {
    const response = await apiFetch("/api/settings");
    if (!response.ok)
      return { ok: false, message: "Settings could not be loaded." };
    return { ok: true, value: toSetup(await response.json()) };
  } catch {
    return { ok: false, message: "Settings could not be loaded." };
  }
}

async function saveSettings(
  updates: Record<string, string>,
): Promise<SetupResult<AgentSetup>> {
  try {
    const response = await patchSettings(updates);
    const payload = (await response.json()) as {
      settings?: SettingRecord[];
      last_agent?: LastAgentConnection | null;
      error?: string;
      fields?: { message: string }[];
    };
    if (!response.ok) {
      const message =
        payload.fields?.map((field) => field.message).join(" ") ||
        payload.error ||
        "Nothing was saved.";
      return { ok: false, message };
    }
    return { ok: true, value: toSetup(payload) };
  } catch {
    return { ok: false, message: "Settings could not be saved. Try again." };
  }
}

type Connected = { setup: AgentSetup; token: string | null };

/** Save `updates` with a new password, which comes back to show once. */
async function saveWithNewToken(
  updates: Record<string, string>,
): Promise<SetupResult<Connected>> {
  let token: string | null;
  try {
    const response = await generateMcpTokenCandidate();
    const payload = (await response.json()) as { value?: string };
    token = response.ok && payload.value ? payload.value : null;
  } catch {
    token = null;
  }
  if (!token) {
    return {
      ok: false,
      message: "The agent's password could not be made. Try again.",
    };
  }
  const saved = await saveSettings({ ...updates, [MCP_TOKEN]: token });
  return saved.ok ? { ok: true, value: { setup: saved.value, token } } : saved;
}

/** The one-click connect: MCP on, writes off, and a new password unless the
 * server's configuration file holds it. Never turns writes on, and refuses
 * when the configuration file holds them on. */
export async function connectAgent(
  setup: AgentSetup,
): Promise<SetupResult<Connected>> {
  if (setup.mcpLocked && !setup.mcpEnabled) {
    return {
      ok: false,
      message:
        "Agent access is switched off in the server's configuration file, so it cannot be turned on here.",
    };
  }
  if (setup.writesLocked && setup.writesEnabled) {
    return {
      ok: false,
      message:
        "The server's configuration file lets agents change notes, so connecting here would not be read-only. Set HATCHDOOR_MCP_WRITE_ENABLED=false there first, or turn agent access on in Settings.",
    };
  }
  const updates: Record<string, string> = {};
  if (!setup.mcpLocked) updates[MCP_ENABLED] = "true";
  if (!setup.writesLocked) updates[MCP_WRITE_ENABLED] = "false";
  if (setup.tokenLocked) {
    const saved = await saveSettings(updates);
    return saved.ok
      ? { ok: true, value: { setup: saved.value, token: null } }
      : saved;
  }
  return saveWithNewToken(updates);
}

/** Replace the password. Agents holding the old one stop getting in. */
export function replaceToken(): Promise<SetupResult<Connected>> {
  return saveWithNewToken({});
}

export async function setUpdateCheck(
  on: boolean,
): Promise<SetupResult<AgentSetup>> {
  return saveSettings({ [UPDATE_CHECK]: on ? "true" : "false" });
}

/** Where an agent reaches MCP: the public address when one is set, else the
 * address this page was opened at. */
export function mcpAddress(publicUrl: string, origin: string): string {
  return `${(publicUrl || origin).replace(/\/+$/, "")}/mcp`;
}

export const TOKEN_PLACEHOLDER = "<your MCP password>";

export type ClientId = "claude" | "codex" | "openclaw" | "hermes" | "other";

export type ClientConfig = {
  id: ClientId;
  label: string;
  /** Where the text goes, as plain sentences. */
  where: string;
  text: string;
};

/** A ready-made config per client, with the address and password filled in. */
export function clientConfigs(
  address: string,
  token: string | null,
): ClientConfig[] {
  const secret = token ?? TOKEN_PLACEHOLDER;
  return [
    {
      id: "claude",
      label: "Claude Code",
      where:
        "Run this in a terminal, then type /mcp in Claude Code to check the connection.",
      text: `claude mcp add --transport http --scope user hatchdoor ${address} --header "Authorization: Bearer ${secret}"`,
    },
    {
      id: "codex",
      label: "Codex",
      where:
        "Add this to ~/.codex/config.toml, then run codex mcp list to check it.",
      text: [
        "[mcp_servers.hatchdoor]",
        `url = "${address}"`,
        `http_headers = { Authorization = "Bearer ${secret}" }`,
      ].join("\n"),
    },
    {
      id: "openclaw",
      label: "OpenClaw",
      where:
        "Add this to ~/.openclaw/openclaw.json, then run openclaw mcp doctor hatchdoor --probe to check it.",
      text: JSON.stringify(
        {
          mcp: {
            servers: {
              hatchdoor: {
                url: address,
                transport: "streamable-http",
                headers: { Authorization: `Bearer ${secret}` },
              },
            },
          },
        },
        null,
        2,
      ),
    },
    {
      id: "hermes",
      label: "Hermes",
      where: "Add this to ~/.hermes/config.yaml.",
      text: [
        "mcp_servers:",
        "  hatchdoor:",
        `    url: "${address}"`,
        "    headers:",
        `      Authorization: "Bearer ${secret}"`,
      ].join("\n"),
    },
    {
      id: "other",
      label: "Other",
      where:
        "Any MCP client that speaks Streamable HTTP takes these two values.",
      text: `Address        ${address}\nAuthorization  Bearer ${secret}`,
    },
  ];
}

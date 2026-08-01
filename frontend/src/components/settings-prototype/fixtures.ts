/**
 * PROTOTYPE — throwaway. Fixture data for the Settings page variants.
 *
 * Shapes mirror the wire contract decided in "Decide the settings API
 * contract" (#55): the server sends key/value/source/locked/class/kind and
 * nothing else; every label, sentence and grouping below is the page's own.
 */

export type SettingKind = "switch" | "number" | "text" | "secret" | "mode";

/** Live-applicable classes only. There is no applies-after-restart class (#60). */
export type SettingClass = "instant" | "reindex";

export type LockReason = "environment" | "never";

export type Setting = {
  key: string;
  kind: SettingKind;
  cls: SettingClass;
  /** For secrets this is null; `configured` says whether one is set. */
  value: string | null;
  configured?: boolean;
  source: "environment" | "stored" | "default";
  locked: LockReason | null;
};

export type GitLifecycle = "disabled" | "starting" | "running" | "stopping";

export type GitStatus = {
  lifecycle: GitLifecycle;
  mode: "off" | "local" | "remote";
  /** Remote mode only; absent in local mode rather than zeroed (#61). */
  unpushed: number | null;
  lastCommit: string | null;
  lastError: string | null;
};

export type IndexStatus = {
  state: "idle" | "rebuilding";
  percent: number;
  etaSeconds: number | null;
  /** Stored settings and the index disagree — the resting drift state (#57). */
  drift: boolean;
  lastFailure: string | null;
};

export type Scenario = "mixed" | "fresh" | "pinned" | "local" | "rebuilding";

export const SCENARIOS: { id: Scenario; label: string; blurb: string }[] = [
  {
    id: "mixed",
    label: "Typical",
    blurb: "A few values pinned in .env, git syncing to a remote.",
  },
  {
    id: "fresh",
    label: "Fresh install",
    blurb: "Nothing pinned but the web token. Everything editable here.",
  },
  {
    id: "pinned",
    label: "Fully pinned",
    blurb: "What an existing user sees after upgrading: every field locked.",
  },
  {
    id: "local",
    label: "Local versioning",
    blurb: "Git sync on, no remote. Remote-only fields absent.",
  },
  {
    id: "rebuilding",
    label: "Rebuilding search",
    blurb: "A reindex-triggering setting was just saved.",
  },
];

// ── Page-owned copy ─────────────────────────────────────────────────────────
// Section order mirrors .env.example (#59) so one vocabulary spans the file,
// the docs and this page.

export type SectionId = "vault" | "agents" | "uploads" | "versioning";

export const SECTIONS: {
  id: SectionId;
  num: string;
  title: string;
  blurb: string;
}[] = [
  {
    id: "vault",
    num: "01",
    title: "Vault",
    blurb:
      "What Hatchdoor treats as archived, and what it leaves out of search.",
  },
  {
    id: "agents",
    num: "02",
    title: "Agent access",
    blurb: "Whether AI assistants can reach this vault, and what they may do.",
  },
  {
    id: "uploads",
    num: "03",
    title: "Uploads",
    blurb: "How large a file may be attached to a note.",
  },
  {
    id: "versioning",
    num: "04",
    title: "Versioning",
    blurb:
      "Keeping a history of every change, and optionally sending it elsewhere.",
  },
];

export type Copy = {
  section: SectionId;
  label: string;
  help: string;
  /** Shown under a field only when it needs a unit or an example. */
  hint?: string;
};

export const COPY: Record<string, Copy> = {
  HATCHDOOR_ARCHIVE_PREFIX: {
    section: "vault",
    label: "Archive folder",
    help: "Notes under this folder are treated as archived: still searchable, but ranked below everything else.",
    hint: "A folder name ending in /",
  },
  HATCHDOOR_EXCLUDE: {
    section: "vault",
    label: "Ignore these files",
    help: "Files matching these patterns are left out of search entirely. Same syntax as a .gitignore file, separated by commas.",
    hint: "Changing this rebuilds the search index",
  },
  HATCHDOOR_EMBED_LAYERS: {
    section: "vault",
    label: "Deep search for older notes",
    help: "Spends more disk and startup time so that notes further down the pile still match on meaning, not just on words.",
    hint: "Changing this rebuilds the search index",
  },
  HATCHDOOR_MCP_ENABLED: {
    section: "agents",
    label: "Let assistants connect",
    help: "Opens a second door into this vault for AI assistants such as Claude. Off means the door does not exist.",
  },
  HATCHDOOR_MCP_WRITE_ENABLED: {
    section: "agents",
    label: "Let assistants change notes",
    help: "Assistants can create, edit, move and delete notes and attachments. Off means they can only read.",
  },
  HATCHDOOR_MCP_BEARER_TOKEN: {
    section: "agents",
    label: "Assistant password",
    help: "The password an assistant must send to get in. Required whenever assistants are allowed to connect.",
  },
  HATCHDOOR_MCP_ALLOWED_ORIGINS: {
    section: "agents",
    label: "Websites allowed to connect",
    help: "Assistants running inside a browser must come from one of these addresses. Separated by commas.",
  },
  HATCHDOOR_MAX_ATTACHMENT_BYTES: {
    section: "uploads",
    label: "Largest file from this app",
    help: "The biggest file you can drop into a note from your browser.",
    hint: "In megabytes",
  },
  HATCHDOOR_MCP_MAX_BASE64_BYTES: {
    section: "uploads",
    label: "Largest file from an assistant",
    help: "The biggest file an assistant can send inline. Assistants that can make a normal upload are not limited by this.",
    hint: "In megabytes",
  },
  HATCHDOOR_GIT_SYNC_ENABLED: {
    section: "versioning",
    label: "Keep a history of changes",
    help: "Off keeps no history. On this machine records every change locally. Send elsewhere also pushes it to a server you already set up.",
  },
  HATCHDOOR_GIT_REMOTE: {
    section: "versioning",
    label: "Server nickname",
    help: "The short name your vault folder already uses for the server it sends to. Almost always origin.",
  },
  HATCHDOOR_GIT_HTTPS_USERNAME: {
    section: "versioning",
    label: "Username",
    help: "The username that goes with the access token below.",
  },
  HATCHDOOR_GIT_HTTPS_TOKEN: {
    section: "versioning",
    label: "Access token",
    help: "The token that lets Hatchdoor send changes to the server. Required when sending elsewhere.",
  },
  HATCHDOOR_GIT_DEBOUNCE_SECONDS: {
    section: "versioning",
    label: "Wait before recording",
    help: "How long Hatchdoor waits after you stop typing before recording a batch of changes.",
    hint: "In seconds",
  },
  HATCHDOOR_GIT_AUTHOR_NAME: {
    section: "versioning",
    label: "Recorded as (name)",
    help: "The name attached to every recorded change.",
  },
  HATCHDOOR_GIT_AUTHOR_EMAIL: {
    section: "versioning",
    label: "Recorded as (email)",
    help: "The email attached to every recorded change.",
  },
  HATCHDOOR_GIT_BRANCH: {
    section: "versioning",
    label: "Branch",
    help: "Which line of history changes are recorded on. Hatchdoor always uses whichever one your vault folder is already on.",
  },
};

/** Remote-only fields: hidden, not locked, when the mode is not remote (#61). */
export const REMOTE_ONLY = [
  "HATCHDOOR_GIT_REMOTE",
  "HATCHDOOR_GIT_HTTPS_USERNAME",
  "HATCHDOOR_GIT_HTTPS_TOKEN",
];

/** Shown only while versioning is on at all. */
export const VERSIONING_DETAIL = [
  ...REMOTE_ONLY,
  "HATCHDOOR_GIT_DEBOUNCE_SECONDS",
  "HATCHDOOR_GIT_AUTHOR_NAME",
  "HATCHDOOR_GIT_AUTHOR_EMAIL",
  "HATCHDOOR_GIT_BRANCH",
];

// ── The payloads ────────────────────────────────────────────────────────────

function base(): Setting[] {
  return [
    {
      key: "HATCHDOOR_ARCHIVE_PREFIX",
      kind: "text",
      cls: "instant",
      value: "90-archive/",
      source: "default",
      locked: null,
    },
    {
      key: "HATCHDOOR_EXCLUDE",
      kind: "text",
      cls: "reindex",
      value: "*.excalidraw.md, .obsidian/**",
      source: "stored",
      locked: null,
    },
    {
      key: "HATCHDOOR_EMBED_LAYERS",
      kind: "switch",
      cls: "reindex",
      value: "false",
      source: "default",
      locked: null,
    },
    {
      key: "HATCHDOOR_MCP_ENABLED",
      kind: "switch",
      cls: "instant",
      value: "true",
      source: "stored",
      locked: null,
    },
    {
      key: "HATCHDOOR_MCP_WRITE_ENABLED",
      kind: "switch",
      cls: "instant",
      value: "true",
      source: "stored",
      locked: null,
    },
    {
      key: "HATCHDOOR_MCP_BEARER_TOKEN",
      kind: "secret",
      cls: "instant",
      value: null,
      configured: true,
      source: "stored",
      locked: null,
    },
    {
      key: "HATCHDOOR_MCP_ALLOWED_ORIGINS",
      kind: "text",
      cls: "instant",
      value: "http://127.0.0.1, http://localhost",
      source: "default",
      locked: null,
    },
    {
      key: "HATCHDOOR_MAX_ATTACHMENT_BYTES",
      kind: "number",
      cls: "instant",
      value: "10485760",
      source: "default",
      locked: null,
    },
    {
      key: "HATCHDOOR_MCP_MAX_BASE64_BYTES",
      kind: "number",
      cls: "instant",
      value: "5242880",
      source: "default",
      locked: null,
    },
    {
      key: "HATCHDOOR_GIT_SYNC_ENABLED",
      kind: "mode",
      cls: "instant",
      value: "remote",
      source: "stored",
      locked: null,
    },
    {
      key: "HATCHDOOR_GIT_REMOTE",
      kind: "text",
      cls: "instant",
      value: "origin",
      source: "default",
      locked: null,
    },
    {
      key: "HATCHDOOR_GIT_HTTPS_USERNAME",
      kind: "text",
      cls: "instant",
      value: "hatchdoor",
      source: "default",
      locked: null,
    },
    {
      key: "HATCHDOOR_GIT_HTTPS_TOKEN",
      kind: "secret",
      cls: "instant",
      value: null,
      configured: true,
      source: "stored",
      locked: null,
    },
    {
      key: "HATCHDOOR_GIT_DEBOUNCE_SECONDS",
      kind: "number",
      cls: "instant",
      value: "30",
      source: "default",
      locked: null,
    },
    {
      key: "HATCHDOOR_GIT_AUTHOR_NAME",
      kind: "text",
      cls: "instant",
      value: "Hatchdoor",
      source: "default",
      locked: null,
    },
    {
      key: "HATCHDOOR_GIT_AUTHOR_EMAIL",
      kind: "text",
      cls: "instant",
      value: "hatchdoor@localhost",
      source: "default",
      locked: null,
    },
    {
      // Never editable here, for a different reason than an .env pin (#56).
      key: "HATCHDOOR_GIT_BRANCH",
      kind: "text",
      cls: "instant",
      value: "main",
      source: "environment",
      locked: "never",
    },
  ];
}

function pin(settings: Setting[], keys: string[]): Setting[] {
  return settings.map((s) =>
    keys.includes(s.key) && s.locked !== "never"
      ? { ...s, source: "environment" as const, locked: "environment" as const }
      : s,
  );
}

function set(settings: Setting[], key: string, patch: Partial<Setting>) {
  return settings.map((s) => (s.key === key ? { ...s, ...patch } : s));
}

export type Payload = {
  settings: Setting[];
  git: GitStatus;
  index: IndexStatus;
};

export function payloadFor(scenario: Scenario): Payload {
  let settings = base();
  let git: GitStatus = {
    lifecycle: "running",
    mode: "remote",
    unpushed: 0,
    lastCommit: "12 minutes ago",
    lastError: null,
  };
  let index: IndexStatus = {
    state: "idle",
    percent: 0,
    etaSeconds: null,
    drift: false,
    lastFailure: null,
  };

  if (scenario === "mixed") {
    settings = pin(settings, [
      "HATCHDOOR_MCP_BEARER_TOKEN",
      "HATCHDOOR_MCP_ALLOWED_ORIGINS",
      "HATCHDOOR_GIT_HTTPS_TOKEN",
    ]);
  }

  if (scenario === "fresh") {
    settings = set(settings, "HATCHDOOR_MCP_ENABLED", {
      value: "false",
      source: "default",
    });
    settings = set(settings, "HATCHDOOR_MCP_WRITE_ENABLED", {
      value: "false",
      source: "default",
    });
    settings = set(settings, "HATCHDOOR_MCP_BEARER_TOKEN", {
      configured: false,
      source: "default",
    });
    settings = set(settings, "HATCHDOOR_EXCLUDE", {
      value: "",
      source: "default",
    });
    settings = set(settings, "HATCHDOOR_GIT_SYNC_ENABLED", {
      value: "off",
      source: "default",
    });
    settings = set(settings, "HATCHDOOR_GIT_HTTPS_TOKEN", {
      configured: false,
      source: "default",
    });
    git = {
      lifecycle: "disabled",
      mode: "off",
      unpushed: null,
      lastCommit: null,
      lastError: null,
    };
  }

  if (scenario === "pinned") {
    settings = pin(
      settings,
      base().map((s) => s.key),
    );
  }

  if (scenario === "local") {
    settings = set(settings, "HATCHDOOR_GIT_SYNC_ENABLED", { value: "local" });
    git = {
      lifecycle: "running",
      mode: "local",
      unpushed: null,
      lastCommit: "3 minutes ago",
      lastError: null,
    };
  }

  if (scenario === "rebuilding") {
    settings = set(settings, "HATCHDOOR_EMBED_LAYERS", {
      value: "true",
      source: "stored",
    });
    index = {
      state: "rebuilding",
      percent: 38,
      etaSeconds: 260,
      drift: true,
      lastFailure: null,
    };
  }

  return { settings, git, index };
}

// ── Helpers the variants share (formatting, not layout) ─────────────────────

export const MAX_UPLOAD_BYTES = 512 * 1024 * 1024;

export function toMb(bytes: string | null): string {
  if (!bytes) return "";
  const n = Number(bytes);
  if (!Number.isFinite(n)) return bytes;
  return String(Math.round((n / (1024 * 1024)) * 10) / 10);
}

export function fromMb(mb: string): string {
  const n = Number(mb);
  if (!Number.isFinite(n)) return mb;
  return String(Math.round(n * 1024 * 1024));
}

export function fmtEta(seconds: number | null): string {
  if (seconds == null) return "estimating";
  if (seconds < 90) return `about ${seconds} seconds left`;
  return `about ${Math.round(seconds / 60)} minutes left`;
}

export const LOCK_COPY: Record<LockReason, { chip: string; why: string }> = {
  environment: {
    chip: "Set in .env",
    why: "This value comes from your .env file, which always wins. To change it, edit that file and restart Hatchdoor.",
  },
  never: {
    chip: "Not editable here",
    why: "Hatchdoor always follows whichever branch your vault folder is on, so there is nothing to choose.",
  },
};

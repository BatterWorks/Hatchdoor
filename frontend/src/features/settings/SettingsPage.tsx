/**
 * Settings — the structure settled by the Variant B prototype (issue #58):
 * an index of sections on the left, one section at a time on the right, and
 * the save action belonging to the section rather than the page. A locked
 * setting is not a disabled control: it leaves the form entirely and becomes a
 * record in the "Managed outside this page" plaque below it.
 */

import { useEffect, useMemo, useState } from "react";

import { apiFetch } from "../../api/api";

type SettingKind = "switch" | "number" | "text" | "secret" | "mode";
type Setting = {
  key: string;
  value: string | null;
  configured?: boolean;
  source: "environment" | "stored" | "default";
  locked: "environment" | "never" | null;
  class: "instant" | "reindex";
  kind: SettingKind;
};

type IndexStatus = {
  state: "up_to_date" | "rebuilding" | "failed";
  stale: boolean;
  drift: boolean;
  notes_completed?: number;
  notes_total?: number;
  chunks_completed?: number;
  chunks_total?: number;
  tokens_completed?: number;
  tokens_total?: number;
  percent?: number;
  eta_seconds?: number;
  last_failure?: string | null;
};

type GitStatus = {
  state: "disabled" | "starting" | "running" | "stopping";
  mode: "off" | "local" | "remote";
  last_sync_at?: string | null;
  last_ok?: boolean;
  last_error?: string | null;
  pending?: number;
  unpushed?: number;
};

type Consequence = "reindex" | "git_init" | "git_downgrade";
type Confirmation = {
  consequence: Consequence;
  message: string;
  updates: Record<string, string>;
};

const STATUS_POLL_MS = 2_000;

/** Remote-only fields: absent, not locked, when the mode is not remote (#61). */
const REMOTE_ONLY = [
  "HATCHDOOR_GIT_HTTPS_USERNAME",
  "HATCHDOOR_GIT_HTTPS_TOKEN",
];

/** Shown only while versioning is on at all. */
const VERSIONING_DETAIL = [
  ...REMOTE_ONLY,
  "HATCHDOOR_GIT_DEBOUNCE_SECONDS",
  "HATCHDOOR_GIT_AUTHOR_NAME",
  "HATCHDOOR_GIT_AUTHOR_EMAIL",
  "HATCHDOOR_GIT_BRANCH",
];

/** Section order mirrors .env.example (#59), so one vocabulary spans both. */
const SECTIONS = [
  {
    id: "vault",
    number: "01",
    title: "Vault",
    blurb:
      "What Hatchdoor treats as archived, and what it leaves out of search.",
  },
  {
    id: "agents",
    number: "02",
    title: "Agent access (MCP)",
    blurb: "Whether AI assistants can reach this vault, and what they may do.",
  },
  {
    id: "uploads",
    number: "03",
    title: "Uploads",
    blurb: "How large a file may be attached to a note.",
  },
  {
    id: "versioning",
    number: "04",
    title: "Versioning",
    blurb:
      "Keeping a history of every change, and optionally sending it elsewhere.",
  },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

const COPY: Record<
  string,
  {
    section: SectionId;
    label: string;
    help: string;
    unit?: string;
    /** Shown in the empty box, so the shape of a valid answer is visible. */
    example?: string;
    /** A quiet line under the help, for a fact the help sentence cannot hold. */
    note?: string;
  }
> = {
  HATCHDOOR_ARCHIVE_PREFIX: {
    section: "vault",
    label: "Archive folder",
    help: "Notes under this folder are treated as archived: still searchable, but ranked below everything else.",
  },
  HATCHDOOR_EXCLUDE: {
    section: "vault",
    label: "Ignore these files and folders",
    help: "Anything matching these patterns is left out of search entirely. Same syntax as a .gitignore file, separated by commas. End a pattern with / to ignore a whole folder and everything in it.",
    example: "drafts/, *.excalidraw.md, 99-scratch/",
    note: "Ignored before your list is read: .obsidian/, .trash/, .hatchdoor-trash/, .DS_Store, *.tmp, *.sync-conflict-*. Start a pattern with ! to bring one of those back.",
  },
  HATCHDOOR_EMBED_LAYERS: {
    section: "vault",
    label: "Meaning search in demoted layers",
    help: "Folders marked with a .hatchdoor-layer file stay out of the browser and out of normal search; assistants can still ask for them by name. On, those notes can also be found by meaning. Off, only by exact words, which saves disk space and indexing time.",
  },
  HATCHDOOR_MCP_ENABLED: {
    section: "agents",
    label: "Let assistants connect (MCP)",
    help: "Opens a second door into this vault for AI assistants such as Claude. Off means the door does not exist.",
  },
  HATCHDOOR_MCP_WRITE_ENABLED: {
    section: "agents",
    label: "Let assistants change notes",
    help: "Assistants can create, edit, move and delete notes and attachments. Off means they can only read.",
  },
  HATCHDOOR_MCP_BEARER_TOKEN: {
    section: "agents",
    label: "MCP password",
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
    unit: "in megabytes",
  },
  HATCHDOOR_MCP_MAX_BASE64_BYTES: {
    section: "uploads",
    label: "Largest file from an assistant",
    help: "The biggest file an assistant can send inline. Assistants that can make a normal upload are not limited by this.",
    unit: "in megabytes",
  },
  HATCHDOOR_GIT_SYNC_ENABLED: {
    section: "versioning",
    label: "Keep a history of changes",
    help: "Off keeps no history. This machine records every change locally. Send elsewhere also pushes it to a server you already set up.",
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
    unit: "in seconds",
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

const MODES = [
  { id: "off", label: "Off" },
  { id: "local", label: "This machine" },
  { id: "remote", label: "Send elsewhere" },
];

const REINDEX_CONFIRMATION =
  "Saving this rebuilds the search index. The setting takes effect right away and search keeps working the whole time — it just keeps answering from the old setting until the rebuild finishes.";

const BUSY_MESSAGE =
  "Still finishing the last batch of changes. Try again in a few seconds — nothing was lost.";

/** Both lock reasons render the same way and say different things (#47, #56). */
const LOCK_WHY: Record<NonNullable<Setting["locked"]>, string> = {
  environment:
    "This value comes from your .env file, which always wins. To change it, edit that file and restart Hatchdoor.",
  never:
    "Hatchdoor always follows whichever branch your vault folder is on, so there is nothing to choose.",
};

/**
 * The wire still carries the boolean spellings versioning had before it grew a
 * third position, so the segmented control normalises what it is given rather
 * than showing nothing selected for a value it does not recognise.
 */
function normalizeMode(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "local") return "local";
  if (["remote", "true", "1", "yes", "on"].includes(normalized))
    return "remote";
  return "off";
}

function toMb(bytes: string | null): string {
  if (!bytes) return "";
  const value = Number(bytes);
  if (!Number.isFinite(value)) return bytes;
  return String(Math.round((value / (1024 * 1024)) * 10) / 10);
}

function fromMb(mb: string): string {
  const value = Number(mb);
  if (!Number.isFinite(value)) return mb;
  return String(Math.round(value * 1024 * 1024));
}

/**
 * A locked setting is a record of the same thing the control would have shown,
 * so it is spelled in the same words: On rather than true, a megabyte count
 * rather than a byte count, the segment's name rather than the wire's alias.
 */
function plaqueValue(setting: Setting): string {
  if (setting.kind === "secret") return setting.configured ? "set" : "not set";
  const value = setting.value ?? "";
  if (setting.kind === "switch") return value === "true" ? "On" : "Off";
  if (setting.kind === "mode") {
    const selected = normalizeMode(value);
    return MODES.find((item) => item.id === selected)!.label;
  }
  if (setting.key.includes("BYTES")) return `${toMb(value)} MB`;
  return value || "empty";
}

function formatEta(seconds: number | undefined): string {
  if (seconds === undefined) return "estimating";
  if (seconds < 90) return `about ${seconds} seconds left`;
  return `about ${Math.round(seconds / 60)} minutes left`;
}

function formatWhen(timestamp: string | null | undefined): string | null {
  if (!timestamp) return null;
  const then = Date.parse(timestamp);
  if (!Number.isFinite(then)) return null;
  const minutes = Math.round((Date.now() - then) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes === 1) return "1 minute ago";
  if (minutes < 60) return `${minutes} minutes ago`;
  const hours = Math.round(minutes / 60);
  if (hours === 1) return "1 hour ago";
  if (hours < 24) return `${hours} hours ago`;
  const days = Math.round(hours / 24);
  return days === 1 ? "1 day ago" : `${days} days ago`;
}

export function SettingsPage() {
  const [settings, setSettings] = useState<Setting[]>([]);
  const [active, setActive] = useState<SectionId>("vault");
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [banner, setBanner] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [webToken, setWebToken] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<Record<string, string>>({});
  const [replacing, setReplacing] = useState<Record<string, boolean>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [indexStatus, setIndexStatus] = useState<IndexStatus | null>(null);
  const [gitStatus, setGitStatus] = useState<GitStatus | null>(null);

  const load = async () => {
    const response = await apiFetch("/api/settings");
    if (!response.ok) throw new Error("Settings could not be loaded.");
    const payload = (await response.json()) as { settings: Setting[] };
    setSettings(payload.settings);
    setDrafts({});
    setErrors({});
    setRevealed({});
    setReplacing({});
  };

  useEffect(() => {
    void load()
      .catch((error: unknown) =>
        setBanner(
          error instanceof Error
            ? error.message
            : "Settings could not be loaded.",
        ),
      )
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    let live = true;
    const poll = async () => {
      try {
        const response = await apiFetch("/api/git-status");
        if (!response.ok) throw new Error();
        if (live) setGitStatus((await response.json()) as GitStatus);
      } catch {
        if (live) setGitStatus(null);
      }
    };
    void poll();
    const interval = window.setInterval(() => void poll(), STATUS_POLL_MS);
    return () => {
      live = false;
      window.clearInterval(interval);
    };
  }, []);

  useEffect(() => {
    let live = true;
    const poll = async () => {
      try {
        const response = await apiFetch("/api/index-status");
        if (!response.ok) throw new Error();
        if (live) setIndexStatus((await response.json()) as IndexStatus);
      } catch {
        if (live) setIndexStatus(null);
      }
    };
    void poll();
    const interval = window.setInterval(() => void poll(), STATUS_POLL_MS);
    return () => {
      live = false;
      window.clearInterval(interval);
    };
  }, []);

  const section = useMemo(
    () => SECTIONS.find((item) => item.id === active)!,
    [active],
  );

  const effective = (setting: Setting) =>
    drafts[setting.key] ?? setting.value ?? "";
  const mode = normalizeMode(
    drafts.HATCHDOOR_GIT_SYNC_ENABLED ??
      settings.find((item) => item.key === "HATCHDOOR_GIT_SYNC_ENABLED")
        ?.value ??
      "off",
  );

  const visible = (setting: Setting) => {
    if (!COPY[setting.key]) return false;
    if (mode === "off" && VERSIONING_DETAIL.includes(setting.key)) return false;
    if (mode === "local" && REMOTE_ONLY.includes(setting.key)) return false;
    return true;
  };
  const inSection = (id: SectionId) =>
    settings.filter((item) => COPY[item.key]?.section === id && visible(item));

  const fields = inSection(active);
  const editable = fields.filter((item) => !item.locked);
  const locked = fields.filter((item) => item.locked);
  const dirtyKeys = editable
    .filter((item) => drafts[item.key] !== undefined)
    .map((item) => item.key);

  const editableCount = settings.filter(
    (item) => COPY[item.key] && !item.locked,
  ).length;
  const pinnedCount = settings.filter(
    (item) => COPY[item.key] && item.locked === "environment",
  ).length;

  const edit = (key: string, value: string) => {
    setSaved(null);
    setDrafts((old) => ({ ...old, [key]: value }));
  };

  const discard = () => {
    setDrafts({});
    setErrors({});
    setBanner(null);
    setBusy(null);
    setSaved(null);
    setReplacing({});
  };

  const send = async (
    updates: Record<string, string>,
    accepted?: Consequence,
  ) => {
    setSaving(true);
    setErrors({});
    setBanner(null);
    setBusy(null);
    setSaved(null);
    try {
      const response = await apiFetch("/api/settings", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          updates,
          ...(accepted === "reindex" ? { confirm_reindex: true } : {}),
          ...(accepted === "git_init" ? { confirm_git_init: true } : {}),
          ...(accepted === "git_downgrade"
            ? { confirm_git_downgrade: true }
            : {}),
        }),
      });
      const payload = (await response.json()) as {
        settings?: Setting[];
        error?: string;
        fields?: { key: string | null; message: string }[];
        confirmation_required?: Consequence;
      };
      if (response.status === 409 && payload.confirmation_required) {
        setConfirmation({
          consequence: payload.confirmation_required,
          message: payload.error ?? "This change has a consequence.",
          updates,
        });
        return;
      }
      if (response.status === 409) {
        setBusy(BUSY_MESSAGE);
        return;
      }
      if (!response.ok) {
        setErrors(
          Object.fromEntries(
            (payload.fields ?? [])
              .filter((field) => field.key)
              .map((field) => [field.key!, field.message]),
          ),
        );
        setBanner(payload.error ?? "Nothing was saved.");
        return;
      }
      setSettings(payload.settings ?? settings);
      setDrafts({});
      setRevealed({});
      setReplacing({});
      setSaved("Saved");
    } catch {
      setBanner("Settings could not be saved. Try again.");
    } finally {
      setSaving(false);
    }
  };

  const save = () => {
    const updates = Object.fromEntries(
      editable
        .filter((item) => drafts[item.key] !== undefined)
        .map((item) => [
          item.key,
          item.key.includes("BYTES")
            ? fromMb(drafts[item.key])
            : drafts[item.key],
        ]),
    );
    if (Object.keys(updates).length === 0) return;
    const rebuilds = editable.some(
      (item) => drafts[item.key] !== undefined && item.class === "reindex",
    );
    if (rebuilds) {
      setConfirmation({
        consequence: "reindex",
        message: REINDEX_CONFIRMATION,
        updates,
      });
      return;
    }
    void send(updates);
  };

  const generateMcpToken = async () => {
    setBanner(null);
    try {
      const response = await apiFetch("/api/settings/mcp-token/generate", {
        method: "POST",
      });
      const payload = (await response.json()) as { value?: string };
      if (!response.ok || !payload.value) throw new Error();
      setRevealed((old) => ({
        ...old,
        HATCHDOOR_MCP_BEARER_TOKEN: payload.value!,
      }));
      edit("HATCHDOOR_MCP_BEARER_TOKEN", payload.value);
      setSaved("A new password is ready. It is not in use until you save.");
    } catch {
      setBanner("The MCP password could not be generated.");
    }
  };

  const revealMcpToken = async () => {
    setBanner(null);
    try {
      const response = await apiFetch("/api/settings/mcp-token/reveal", {
        method: "POST",
      });
      const payload = (await response.json()) as { value?: string };
      if (!response.ok || !payload.value) {
        setBanner("This MCP password cannot be shown to this session.");
        return;
      }
      setRevealed((old) => ({
        ...old,
        HATCHDOOR_MCP_BEARER_TOKEN: payload.value!,
      }));
    } catch {
      setBanner("The MCP password could not be shown.");
    }
  };

  const revealWebToken = async () => {
    setBanner(null);
    try {
      const response = await apiFetch("/api/settings/web-token/reveal", {
        method: "POST",
      });
      if (!response.ok) {
        setBanner("No web access token is set for this server.");
        return;
      }
      const payload = (await response.json()) as { value?: string };
      setWebToken(payload.value ?? null);
    } catch {
      setBanner("The web access token could not be shown.");
    }
  };

  const control = (setting: Setting) => {
    const copy = COPY[setting.key];
    const value = effective(setting);

    if (setting.kind === "switch") {
      const on = value === "true";
      return (
        <button
          type="button"
          className="settings-toggle"
          aria-label={copy.label}
          aria-pressed={on}
          onClick={() => edit(setting.key, on ? "false" : "true")}
        >
          <span className="settings-toggle-track">
            <span className="settings-toggle-knob" />
          </span>
          <span>{on ? "On" : "Off"}</span>
        </button>
      );
    }

    if (setting.kind === "mode") {
      const selected = normalizeMode(value);
      return (
        <div
          className="settings-segmented"
          role="group"
          aria-label={copy.label}
        >
          {MODES.map((item) => (
            <button
              key={item.id}
              type="button"
              aria-pressed={selected === item.id}
              onClick={() => edit(setting.key, item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
      );
    }

    if (setting.kind === "number") {
      const shown = setting.key.includes("BYTES") ? toMb(value) : value;
      return (
        <div className="settings-inline">
          <input
            className="settings-input settings-input-short"
            type="number"
            aria-label={copy.label}
            value={shown}
            onChange={(event) => edit(setting.key, event.target.value)}
          />
          {copy.unit ? (
            <span className="settings-unit">{copy.unit}</span>
          ) : null}
        </div>
      );
    }

    if (setting.kind === "secret") {
      const isMcp = setting.key === "HATCHDOOR_MCP_BEARER_TOKEN";
      const draft = drafts[setting.key];
      const shown = revealed[setting.key];
      const masked =
        draft === undefined && shown === undefined && setting.configured;
      const editing = !masked || replacing[setting.key];
      return (
        <div className="settings-inline">
          <input
            className="settings-input"
            type="text"
            aria-label={copy.label}
            placeholder="not set"
            value={draft ?? shown ?? (masked ? "••••••••••••••••" : "")}
            readOnly={masked && !replacing[setting.key]}
            onChange={(event) => edit(setting.key, event.target.value)}
          />
          {isMcp && setting.configured && shown === undefined ? (
            <button
              type="button"
              className="settings-mini"
              onClick={() => void revealMcpToken()}
            >
              Show
            </button>
          ) : null}
          {shown !== undefined ? (
            <button
              type="button"
              className="settings-mini"
              onClick={() =>
                setRevealed((old) => {
                  const next = { ...old };
                  delete next[setting.key];
                  return next;
                })
              }
            >
              Hide
            </button>
          ) : null}
          {!editing ? (
            <button
              type="button"
              className="settings-mini"
              onClick={() => {
                setReplacing((old) => ({ ...old, [setting.key]: true }));
                edit(setting.key, "");
              }}
            >
              Replace
            </button>
          ) : null}
          {isMcp ? (
            <button
              type="button"
              className="settings-mini"
              onClick={() => void generateMcpToken()}
            >
              Generate
            </button>
          ) : null}
        </div>
      );
    }

    return (
      <input
        className="settings-input"
        type="text"
        aria-label={copy.label}
        placeholder={copy.example}
        value={value}
        onChange={(event) => edit(setting.key, event.target.value)}
      />
    );
  };

  if (loading) {
    return (
      <div className="settings-page">
        <p className="settings-muted">Loading settings…</p>
      </div>
    );
  }

  return (
    <div className="settings-page">
      <div className="settings-layout">
        <aside className="settings-index">
          <p className="settings-eyebrow">Server settings</p>
          <nav aria-label="Settings sections">
            {SECTIONS.map((item) => {
              const rows = inSection(item.id);
              const dirty = rows.some(
                (row) => drafts[row.key] !== undefined && !row.locked,
              );
              return (
                <button
                  key={item.id}
                  type="button"
                  className="settings-index-item"
                  data-active={item.id === active}
                  onClick={() => setActive(item.id)}
                >
                  <span className="settings-index-num">{item.number}</span>
                  <span className="settings-index-title">{item.title}</span>
                  {dirty ? (
                    <span className="settings-dirty" aria-label="unsaved" />
                  ) : null}
                  <span className="settings-index-count">
                    {rows.filter((row) => !row.locked).length}/{rows.length}
                  </span>
                </button>
              );
            })}
          </nav>
          <div className="settings-index-foot">
            {editableCount === 0 ? (
              <p>
                Every setting is set in <code>.env</code>. Nothing on this page
                can be changed from here.
              </p>
            ) : (
              <p>
                {editableCount} editable here, {pinnedCount} set in{" "}
                <code>.env</code>.
              </p>
            )}
            <button
              type="button"
              className="settings-link"
              onClick={() => {
                if (webToken) setWebToken(null);
                else void revealWebToken();
              }}
            >
              {webToken ? "Hide web access token" : "Show web access token"}
            </button>
            {webToken ? <code>{webToken}</code> : null}
          </div>
        </aside>

        <div className="settings-main">
          <div className="settings-console">
            <div className="settings-console-cell">
              <span className="settings-console-lbl">Search index</span>
              {indexStatus?.state === "rebuilding" ? (
                <>
                  <span className="settings-console-val">
                    Rebuilding {indexStatus.percent ?? 0}%
                  </span>
                  <div className="settings-mini-bar">
                    <span style={{ width: `${indexStatus.percent ?? 0}%` }} />
                  </div>
                  <span className="settings-muted">
                    Still answering from the old setting ·{" "}
                    {formatEta(indexStatus.eta_seconds)}
                  </span>
                </>
              ) : indexStatus?.drift || indexStatus?.state === "failed" ? (
                <span className="settings-console-val settings-warn">
                  Behind your settings
                </span>
              ) : (
                <span className="settings-console-val">Up to date</span>
              )}
            </div>
            <div className="settings-console-cell">
              <span className="settings-console-lbl">Versioning</span>
              <span className="settings-console-val">
                <span
                  className={`settings-dot is-${gitStatus?.state ?? "disabled"}`}
                />
                {!gitStatus || gitStatus.state === "disabled"
                  ? "Off"
                  : gitStatus.state === "starting"
                    ? "Starting…"
                    : gitStatus.state === "stopping"
                      ? "Finishing…"
                      : gitStatus.mode === "local"
                        ? "On, this machine"
                        : "On, sending"}
              </span>
              {formatWhen(gitStatus?.last_sync_at) ? (
                <span className="settings-muted">
                  last recorded {formatWhen(gitStatus?.last_sync_at)}
                </span>
              ) : null}
              {gitStatus?.mode === "remote" && gitStatus.unpushed != null ? (
                <span className="settings-muted">
                  {gitStatus.unpushed} waiting to send
                </span>
              ) : null}
            </div>
          </div>

          <div className="settings-sec-head">
            <div>
              <h2 className="settings-sec-title">
                <span className="settings-sec-num">{section.number}</span>{" "}
                {section.title}
              </h2>
              <p className="settings-sec-blurb">{section.blurb}</p>
            </div>
            {/* A section with nothing to edit is a record, not a form: no dead
                save button above a plaque holding all its content. */}
            {editable.length === 0 ? null : (
              <div className="settings-sec-actions">
                {saved ? <span className="settings-ok">{saved}</span> : null}
                <button
                  type="button"
                  className="settings-btn"
                  onClick={discard}
                  disabled={dirtyKeys.length === 0 || saving}
                >
                  Discard
                </button>
                <button
                  type="button"
                  className="settings-btn settings-btn-hot"
                  onClick={save}
                  disabled={dirtyKeys.length === 0 || saving}
                >
                  {saving ? "Saving…" : `Save ${section.title.toLowerCase()}`}
                </button>
              </div>
            )}
          </div>

          {banner ? (
            <div className="settings-notice settings-notice-err" role="alert">
              {banner}
            </div>
          ) : null}
          {busy ? (
            <div className="settings-notice settings-notice-warn" role="alert">
              {busy}
            </div>
          ) : null}

          <div className="settings-rows" data-empty={editable.length === 0}>
            {editable.map((setting) => {
              const copy = COPY[setting.key];
              const error = errors[setting.key];
              return (
                <div
                  className={`settings-row${error ? " has-error" : ""}`}
                  key={setting.key}
                >
                  <div>
                    <div className="settings-row-label">
                      {copy.label}
                      {drafts[setting.key] !== undefined ? (
                        <span className="settings-dirty" aria-label="unsaved" />
                      ) : null}
                    </div>
                    <p className="settings-row-help">{copy.help}</p>
                    {copy.note ? (
                      <p className="settings-row-note">{copy.note}</p>
                    ) : null}
                    {setting.class === "reindex" ? (
                      <p className="settings-row-class">
                        Saving this rebuilds the search index.
                      </p>
                    ) : null}
                    {setting.key === "HATCHDOOR_MCP_BEARER_TOKEN" ? (
                      <p className="settings-row-class">
                        This password also controls who can upload files, not
                        only who can talk to assistants.
                      </p>
                    ) : null}
                    {error ? <p className="settings-error">{error}</p> : null}
                  </div>
                  <div>{control(setting)}</div>
                </div>
              );
            })}
          </div>

          {locked.length ? (
            <div className="settings-plaque">
              <p className="settings-plaque-head">Managed outside this page</p>
              <dl>
                {locked.map((setting) => (
                  <div className="settings-plaque-row" key={setting.key}>
                    <dt>
                      {COPY[setting.key].label}
                      <code>{setting.key}</code>
                    </dt>
                    <dd>{plaqueValue(setting)}</dd>
                  </div>
                ))}
              </dl>
              {[...new Set(locked.map((setting) => setting.locked!))].map(
                (reason) => (
                  <p className="settings-plaque-why" key={reason}>
                    {LOCK_WHY[reason]}
                  </p>
                ),
              )}
            </div>
          ) : null}
        </div>
      </div>

      {confirmation ? (
        <div className="settings-modal-back">
          <div
            className="settings-modal"
            role="dialog"
            aria-modal="true"
            aria-label="Before this is saved"
          >
            <h3>Before this is saved</h3>
            <p>{confirmation.message}</p>
            <div className="settings-modal-actions">
              <button
                type="button"
                className="settings-btn"
                onClick={() => setConfirmation(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="settings-btn settings-btn-hot"
                onClick={() => {
                  const pending = confirmation;
                  setConfirmation(null);
                  void send(pending.updates, pending.consequence);
                }}
              >
                Go ahead
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

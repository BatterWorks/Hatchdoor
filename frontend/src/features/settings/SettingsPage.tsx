/**
 * Settings — the structure settled by the Variant B prototype (issue #58):
 * an index of sections on the left, one section at a time on the right, and
 * the save action belonging to the section rather than the page. A locked
 * setting is not a disabled control: it leaves the form entirely and becomes a
 * record in the "Managed outside this page" plaque below it.
 */

import { useEffect, useMemo, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";

import { apiFetch } from "../../api/api";
import {
  CONTEXTUAL_HELP,
  ContextualHelpLink,
  type ContextualHelp,
} from "../help";
import type { LastAgentConnection, VaultSummary } from "../../types";
import {
  discardHeldDraft,
  listHeldDrafts,
  type HeldDraft,
} from "../../lib/writeDrafts";
import { formatWhen } from "./relativeTime";
import { generateMcpTokenCandidate, patchSettings } from "./settingsApi";
import { SettingsModal } from "./SettingsModal";
import { UnsavedDrafts, type RestoreCreateDraft } from "./UnsavedDrafts";
import { VaultSettingsDetail, VaultSettingsIndex } from "./VaultSettingsIndex";

type SettingKind = "switch" | "number" | "text" | "secret" | "mode";
type Setting = {
  key: string;
  value: string | null;
  configured?: boolean;
  source: "environment" | "stored" | "default";
  locked: "environment" | "never" | "demo" | null;
  class: "instant" | "reindex";
  kind: SettingKind;
};

/** The opt-in usage report, from the settings response (ADR-45). `report` is
 * the exact report the next send would carry, already indented by the server
 * so the page shows it as text and never rebuilds it. `install_id` is `null`
 * while the report is off. */
type UsageReportStatus = {
  enabled: boolean;
  install_id: string | null;
  report: string;
  /** When the collector last accepted a report, while the report is on. */
  last_sent_at: string | null;
};

type Consequence = "reindex";
type Confirmation = {
  consequence: Consequence;
  updates: Record<string, string>;
  /** Every consequence already accepted for this same save, so accepting a
   * second one (e.g. a downgrade onto a vault that also needs fresh local
   * history) does not forget the first (issue #57). */
  confirm: Consequence[];
};

/** These legacy instance settings describe one Vault. Their controls move to
 * that Vault's page; #149 supplies the detailed Git behaviour and sign-in UI.
 * The two author keys are not among them: they are the server-wide commit
 * identity a Vault without its own falls back to, so they are always shown,
 * whatever the retired `HATCHDOOR_GIT_SYNC_ENABLED` reads (#340). */
const PER_VAULT_SETTING_KEYS = new Set([
  "HATCHDOOR_ARCHIVE_PREFIX",
  "HATCHDOOR_EXCLUDE",
  "HATCHDOOR_GIT_SYNC_ENABLED",
  "HATCHDOOR_GIT_HTTPS_USERNAME",
  "HATCHDOOR_GIT_HTTPS_TOKEN",
  "HATCHDOOR_GIT_DEBOUNCE_SECONDS",
  "HATCHDOOR_GIT_BRANCH",
]);

/** Section order mirrors .env.example (#59), so one vocabulary spans both. */
const SECTIONS = [
  {
    id: "notes",
    number: "01",
    title: "Notes handling",
    blurb: "How this server indexes the notes its Vaults provide.",
    manual: CONTEXTUAL_HELP.notesSettings,
  },
  {
    id: "agents",
    number: "02",
    title: "Agent access (MCP)",
    blurb: "Whether AI assistants can reach this vault, and what they may do.",
    manual: CONTEXTUAL_HELP.agentSettings,
  },
  {
    id: "uploads",
    number: "03",
    title: "Uploads",
    blurb: "How large a file may be attached to a note.",
    manual: CONTEXTUAL_HELP.uploadSettings,
  },
  {
    id: "updates",
    number: "04",
    title: "Updates",
    blurb: "Whether Hatchdoor tells you when a newer version is out.",
    manual: CONTEXTUAL_HELP.upgrade,
  },
  {
    id: "usage",
    number: "05",
    title: "Usage report",
    blurb:
      "Whether this server tells Hatchdoor's maintainer how it is set up. This is telemetry, and it is off unless you turn it on.",
    manual: CONTEXTUAL_HELP.usageReport,
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
    /** The manual page that explains this setting on its own (#423). */
    manual?: ContextualHelp;
  }
> = {
  HATCHDOOR_ARCHIVE_PREFIX: {
    section: "notes",
    label: "Archive folder",
    help: "Notes under this folder are treated as archived: still searchable, but ranked below everything else.",
  },
  HATCHDOOR_EXCLUDE: {
    section: "notes",
    label: "Ignore these files and folders",
    help: "Anything matching these patterns is left out of search entirely. Same syntax as a .gitignore file, separated by commas. End a pattern with / to ignore a whole folder and everything in it.",
    example: "drafts/, *.excalidraw.md, 99-scratch/",
    note: "Ignored before your list is read: .obsidian/, .trash/, .hatchdoor-trash/, .DS_Store, *.tmp, *.sync-conflict-*. Start a pattern with ! to bring one of those back.",
  },
  HATCHDOOR_EMBED_LAYERS: {
    section: "notes",
    label: "Meaning search in demoted layers",
    help: "Folders marked with a .hatchdoor-layer file stay out of normal search but still show in the sidebar; assistants can ask for them by name. On, those notes can also be found by meaning. Off, only by exact words, which saves disk space and indexing time.",
  },
  HATCHDOOR_DEMO_MODE: {
    section: "notes",
    label: "Public demo mode",
    help: "Read-only public browsing with every write surface disabled. Set for this deployment; there is nothing to change from here.",
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
    manual: CONTEXTUAL_HELP.agentWrites,
  },
  HATCHDOOR_MCP_RATE_LIMITS_ENABLED: {
    section: "agents",
    label: "Limit how fast assistants work",
    help: "Caps assistant activity on the MCP door: at most 120 tool calls per minute per assistant, eight running at once, two of them expensive searches. Over the limit, requests are told to retry shortly. Off removes the caps entirely.",
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
  HATCHDOOR_PUBLIC_URL: {
    section: "agents",
    label: "Public address",
    help: "The address people and assistants use to reach this server. Assistants download and upload files through short-lived links built on it. Left empty, links use the address the assistant connected to, as reported by a proxy's forwarded headers (Forwarded, or X-Forwarded-Proto and X-Forwarded-Host). Set it when the proxy sends none of those or serves Hatchdoor under a path; when set, it always wins.",
    example: "https://notes.example.com",
  },
  HATCHDOOR_MAX_ATTACHMENT_BYTES: {
    section: "uploads",
    label: "Largest file from this app",
    help: "The biggest file you can drop into a note from your browser, or an assistant can upload through a link.",
    unit: "in megabytes",
  },
  HATCHDOOR_MCP_MAX_BASE64_BYTES: {
    section: "uploads",
    label: "Largest file from an assistant",
    help: "The biggest file an assistant can send inline, and the biggest it can download. An assistant uploading through a link is held to the limit above instead.",
    unit: "in megabytes",
  },
  HATCHDOOR_UPDATE_CHECK_ENABLED: {
    section: "updates",
    label: "Tell me about new releases",
    help: "Once a day, Hatchdoor sends one request to GitHub's public list of Hatchdoor releases, carrying this server's IP address and the user-agent Hatchdoor, nothing else.",
    manual: CONTEXTUAL_HELP.updateCheck,
  },
  HATCHDOOR_USAGE_REPORT_ENABLED: {
    section: "usage",
    label: "Send a usage report",
    help: "Once a day, Hatchdoor sends the report shown below to telemetry-hatchdoor.battercloud.cc, which Hatchdoor's maintainer runs, to decide which platforms to test and which parts of Hatchdoor people rely on.",
  },
  HATCHDOOR_GIT_SYNC_ENABLED: {
    section: "notes",
    label: "Keep a history of changes",
    help: "Off keeps no history. This machine records every change locally. Send elsewhere also pushes it to a server you already set up.",
  },
  HATCHDOOR_GIT_HTTPS_USERNAME: {
    section: "notes",
    label: "Username",
    help: "The username that goes with the access token below.",
  },
  HATCHDOOR_GIT_HTTPS_TOKEN: {
    section: "notes",
    label: "Access token",
    help: "The token that lets Hatchdoor send changes to the server. Required when sending elsewhere.",
  },
  HATCHDOOR_GIT_DEBOUNCE_SECONDS: {
    section: "notes",
    label: "Wait before recording",
    help: "How long Hatchdoor waits after you stop typing before recording a batch of changes.",
    unit: "in seconds",
  },
  HATCHDOOR_GIT_AUTHOR_NAME: {
    section: "notes",
    label: "Recorded as (name)",
    help: "The name attached to changes recorded in a Vault that has no commit identity of its own.",
  },
  HATCHDOOR_GIT_AUTHOR_EMAIL: {
    section: "notes",
    label: "Recorded as (email)",
    help: "The email attached to changes recorded in a Vault that has no commit identity of its own.",
  },
  HATCHDOOR_GIT_BRANCH: {
    section: "notes",
    label: "Branch",
    help: "Which line of history changes are recorded on. Hatchdoor always uses whichever one your vault folder is already on.",
  },
};

const REINDEX_CONFIRMATION =
  "Saving this rebuilds the search index. The setting takes effect right away and search keeps working the whole time — it just keeps answering from the old setting until the rebuild finishes.";

/** The page's own words for each consequence a save may need consent for
 * (issue #55, #58): the server sends only the machine-readable identifier. */
const CONSEQUENCE_COPY: Record<Consequence, string> = {
  reindex: REINDEX_CONFIRMATION,
};

const BUSY_MESSAGE =
  "Still finishing the last batch of changes. Try again in a few seconds — nothing was lost.";

/** Each lock reason renders the same way and says something different (#47, #56, #55). */
const LOCK_WHY: Record<NonNullable<Setting["locked"]>, string> = {
  environment:
    "This value comes from your .env file, which always wins. To change it, edit that file and restart Hatchdoor.",
  never:
    "Hatchdoor always follows whichever branch your vault folder is on, so there is nothing to choose.",
  demo: "This is fixed for the public demo deployment and cannot be changed from here.",
};

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
 * rather than a byte count.
 */
function plaqueValue(setting: Setting): string {
  if (setting.kind === "secret") return setting.configured ? "set" : "not set";
  const value = setting.value ?? "";
  if (setting.kind === "switch") return value === "true" ? "On" : "Off";
  if (setting.key.includes("BYTES")) return `${toMb(value)} MB`;
  return value || "empty";
}

export function SettingsPage({
  vaults = [],
  onRestoreCreateDraft,
}: {
  /** Enabled Vaults, for the held-draft destination picker (#151). */
  vaults?: VaultSummary[];
  /** Held drafts that need a new note stay unrecoverable without it. */
  onRestoreCreateDraft?: RestoreCreateDraft;
} = {}) {
  const location = useLocation();
  const navigate = useNavigate();
  // Set once by the zero-Vault workspace state's `Add a Vault` button
  // (#150) navigating here; consumed on this first render only so a later
  // back/forward visit to `/settings` does not reopen the flow on its own.
  const [autoOpenCreation] = useState(() =>
    Boolean(
      (location.state as { openVaultCreation?: boolean } | null)
        ?.openVaultCreation,
    ),
  );
  useEffect(() => {
    if (autoOpenCreation) {
      navigate(location.pathname, { replace: true, state: null });
    }
    // Only ever runs once, right after the initial render consumed the
    // navigation state above — location/navigate are stable across
    // re-renders for this purpose.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const [settings, setSettings] = useState<Setting[]>([]);
  const [lastAgent, setLastAgent] = useState<LastAgentConnection | null>(null);
  const [usageReport, setUsageReport] = useState<UsageReportStatus | null>(
    null,
  );
  const [active, setActive] = useState<SectionId>("notes");
  const [showDrafts, setShowDrafts] = useState(false);
  const [heldDrafts, setHeldDrafts] = useState<HeldDraft[]>(() =>
    listHeldDrafts(),
  );
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
  const [selectedVaultId, setSelectedVaultId] = useState<string | null>(null);

  const handleDiscardHeldDraft = (id: string) => {
    discardHeldDraft(id);
    const next = listHeldDrafts();
    setHeldDrafts(next);
    // Recovery is a migration artefact, not a standing feature: once the
    // last held draft is dealt with, the section withdraws for good.
    if (next.length === 0) {
      setShowDrafts(false);
    }
  };

  const load = async () => {
    const response = await apiFetch("/api/settings");
    if (!response.ok) throw new Error("Settings could not be loaded.");
    const payload = (await response.json()) as {
      settings: Setting[];
      last_agent?: LastAgentConnection | null;
      usage_report?: UsageReportStatus;
    };
    setSettings(payload.settings);
    setLastAgent(payload.last_agent ?? null);
    setUsageReport(payload.usage_report ?? null);
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

  const section = useMemo(
    () => SECTIONS.find((item) => item.id === active)!,
    [active],
  );

  const effective = (setting: Setting) =>
    drafts[setting.key] ?? setting.value ?? "";

  const visible = (setting: Setting) =>
    Boolean(COPY[setting.key]) && !PER_VAULT_SETTING_KEYS.has(setting.key);
  const inSection = (id: SectionId) =>
    settings.filter((item) => COPY[item.key]?.section === id && visible(item));

  const fields = inSection(active);
  const editable = fields.filter((item) => !item.locked);
  const locked = fields.filter((item) => item.locked);
  const dirtyKeys = editable
    .filter((item) => drafts[item.key] !== undefined)
    .map((item) => item.key);

  // The footer counts with the same test the sections render with, so it
  // cannot claim a row no section shows (#340).
  const editableCount = settings.filter(
    (item) => visible(item) && !item.locked,
  ).length;
  const pinnedCount = settings.filter(
    (item) => visible(item) && item.locked === "environment",
  ).length;

  const edit = (key: string, value: string) => {
    setSaved(null);
    setDrafts((old) => ({ ...old, [key]: value }));
  };

  // Discard and a successful save are scoped to the keys they were about, not
  // the whole page: `drafts` spans every section, and the buttons that act on
  // it name one (#338). Discard drops every key the active section owns, shown
  // or not.
  const withoutKeys = <T,>(record: Record<string, T>, keys: Set<string>) =>
    Object.fromEntries(
      Object.entries(record).filter(([key]) => !keys.has(key)),
    );

  const discard = () => {
    const sectionKeys = new Set(
      Object.keys(drafts).filter((key) => COPY[key]?.section === active),
    );
    setDrafts((old) => withoutKeys(old, sectionKeys));
    setErrors((old) => withoutKeys(old, sectionKeys));
    setBanner(null);
    setBusy(null);
    setSaved(null);
    setReplacing((old) => withoutKeys(old, sectionKeys));
  };

  const send = async (
    updates: Record<string, string>,
    confirm: Consequence[] = [],
  ) => {
    setSaving(true);
    setErrors({});
    setBanner(null);
    setBusy(null);
    setSaved(null);
    try {
      const response = await patchSettings(updates, confirm);
      const payload = (await response.json()) as {
        settings?: Setting[];
        last_agent?: LastAgentConnection | null;
        usage_report?: UsageReportStatus;
        error?: string;
        fields?: { key: string | null; message: string }[];
        confirmation_required?: Consequence;
      };
      if (response.status === 409 && payload.confirmation_required) {
        setConfirmation({
          consequence: payload.confirmation_required,
          updates,
          confirm,
        });
        return;
      }
      if (response.status === 409) {
        setBusy(BUSY_MESSAGE);
        return;
      }
      if (!response.ok) {
        const fields = payload.fields ?? [];
        setErrors(
          Object.fromEntries(
            fields
              .filter((field) => field.key)
              .map((field) => [field.key!, field.message]),
          ),
        );
        // A refusal with no key belongs to no single setting (issue #55) —
        // render it as a form-level message near the section actions rather
        // than dropping it.
        const general = fields
          .filter((field) => !field.key)
          .map((field) => field.message);
        setBanner(
          general.length > 0
            ? general.join(" ")
            : (payload.error ?? "Nothing was saved."),
        );
        return;
      }
      const sentKeys = new Set(Object.keys(updates));
      setSettings(payload.settings ?? settings);
      if (payload.last_agent !== undefined) setLastAgent(payload.last_agent);
      if (payload.usage_report) setUsageReport(payload.usage_report);
      setDrafts((old) => withoutKeys(old, sentKeys));
      setRevealed((old) => withoutKeys(old, sentKeys));
      setReplacing((old) => withoutKeys(old, sentKeys));
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
      // The page's own reindex confirmation, shown before ever contacting the
      // server: nicer UX, but the server remains the authority (issue #57) —
      // the save still carries "reindex" in `confirm`, and an API caller that
      // skips this dialog still gets refused with `409` by the server.
      setConfirmation({ consequence: "reindex", updates, confirm: [] });
      return;
    }
    void send(updates);
  };

  const generateMcpToken = async () => {
    setBanner(null);
    try {
      const response = await generateMcpTokenCandidate();
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

    if (setting.kind === "number") {
      // Drafts for a BYTES setting are already in megabytes (what the box
      // shows and what the user types); only the value read straight off the
      // server (in bytes, before any edit) needs converting. Applying toMb
      // to an in-progress MB draft double-converts it down to ~0 (S2).
      const shown =
        setting.key.includes("BYTES") && drafts[setting.key] === undefined
          ? toMb(value)
          : value;
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
          <VaultSettingsIndex
            selectedVaultId={selectedVaultId}
            onSelectVault={(vaultId) => {
              setShowDrafts(false);
              setSelectedVaultId(vaultId);
            }}
            autoOpenCreation={autoOpenCreation}
          />
          <p className="settings-index-group">This server</p>
          <nav aria-label="Settings sections">
            {heldDrafts.length > 0 ? (
              <button
                type="button"
                className="settings-index-item"
                data-active={showDrafts}
                onClick={() => {
                  setSelectedVaultId(null);
                  setShowDrafts(true);
                }}
              >
                <span className="settings-index-title">Unsaved drafts</span>
                <span className="settings-index-count">
                  {heldDrafts.length}
                </span>
              </button>
            ) : null}
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
                  data-active={!showDrafts && item.id === active}
                  onClick={() => {
                    setSelectedVaultId(null);
                    setShowDrafts(false);
                    setActive(item.id);
                  }}
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

        {selectedVaultId ? (
          <VaultSettingsDetail
            vaultId={selectedVaultId}
            serverIdentity={{
              name:
                settings.find(
                  (item) => item.key === "HATCHDOOR_GIT_AUTHOR_NAME",
                )?.value ?? "",
              email:
                settings.find(
                  (item) => item.key === "HATCHDOOR_GIT_AUTHOR_EMAIL",
                )?.value ?? "",
            }}
            onDisconnect={() => setSelectedVaultId(null)}
          />
        ) : showDrafts ? (
          <UnsavedDrafts
            drafts={heldDrafts}
            vaults={vaults}
            onRestoreCreateDraft={onRestoreCreateDraft}
            onDiscard={handleDiscardHeldDraft}
          />
        ) : (
          <div className="settings-main">
            <div className="settings-sec-head">
              <div>
                <h2 className="settings-sec-title">
                  <span className="settings-sec-num">{section.number}</span>{" "}
                  {section.title}
                </h2>
                <p className="settings-sec-blurb">
                  {section.blurb} <ContextualHelpLink to={section.manual} />
                </p>
                {active === "agents" ? (
                  <p className="settings-sec-blurb" data-testid="last-agent">
                    {lastAgent
                      ? `${lastAgent.name} connected ${
                          formatWhen(lastAgent.connected_at) ?? "just now"
                        }`
                      : "No agent has connected yet"}
                  </p>
                ) : null}
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
              <div
                className="settings-notice settings-notice-warn"
                role="alert"
              >
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
                          <span
                            className="settings-dirty"
                            aria-label="unsaved"
                          />
                        ) : null}
                      </div>
                      <p className="settings-row-help">
                        {copy.help}
                        {copy.manual ? (
                          <>
                            {" "}
                            <ContextualHelpLink to={copy.manual} />
                          </>
                        ) : null}
                      </p>
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
                <p className="settings-plaque-head">
                  Managed outside this page
                </p>
                <dl>
                  {locked.map((setting) => {
                    const copy = COPY[setting.key];
                    return (
                      <div className="settings-plaque-row" key={setting.key}>
                        <dt>
                          {copy.label}
                          <code>{setting.key}</code>
                          {copy.manual ? (
                            <ContextualHelpLink to={copy.manual} />
                          ) : null}
                        </dt>
                        <dd>{plaqueValue(setting)}</dd>
                      </div>
                    );
                  })}
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

            {/* The server builds this text with the code that builds the real
              report, so it is shown as it arrives and never reformatted. */}
            {active === "usage" && usageReport ? (
              <div className="settings-report" data-testid="usage-report">
                {usageReport.install_id ? (
                  <p className="settings-report-id">
                    <span>Install ID</span>
                    <code>{usageReport.install_id}</code>
                  </p>
                ) : null}
                {usageReport.enabled && usageReport.install_id ? (
                  <p
                    className="settings-report-id"
                    data-testid="usage-report-last-sent"
                  >
                    <span>Last report</span>{" "}
                    {usageReport.last_sent_at ? (
                      <time
                        dateTime={usageReport.last_sent_at}
                        title={usageReport.last_sent_at}
                      >
                        {formatWhen(usageReport.last_sent_at) ??
                          usageReport.last_sent_at}
                      </time>
                    ) : (
                      <span>None sent yet</span>
                    )}
                  </p>
                ) : null}
                <p className="settings-plaque-head">
                  {usageReport.enabled && usageReport.install_id
                    ? "The next report"
                    : "The report this server would send"}
                </p>
                <pre className="settings-report-body" tabIndex={0}>
                  {usageReport.report}
                </pre>
              </div>
            ) : null}
          </div>
        )}
      </div>

      {confirmation ? (
        <SettingsModal
          label="Before this is saved"
          onClose={() => setConfirmation(null)}
        >
          <h3>Before this is saved</h3>
          <p>{CONSEQUENCE_COPY[confirmation.consequence]}</p>
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
                void send(pending.updates, [
                  ...pending.confirm,
                  pending.consequence,
                ]);
              }}
            >
              Go ahead
            </button>
          </div>
        </SettingsModal>
      ) : null}
    </div>
  );
}

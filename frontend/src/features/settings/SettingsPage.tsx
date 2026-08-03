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

const COPY: Record<
  string,
  {
    section: (typeof SECTIONS)[number]["id"];
    label: string;
    help: string;
    unit?: string;
  }
> = {
  HATCHDOOR_ARCHIVE_PREFIX: {
    section: "vault",
    label: "Archive folder",
    help: "Notes under this folder are treated as archived: still searchable, but ranked below everything else.",
  },
  HATCHDOOR_EXCLUDE: {
    section: "vault",
    label: "Ignore these files",
    help: "Patterns to leave out of search entirely, separated by commas.",
  },
  HATCHDOOR_EMBED_LAYERS: {
    section: "vault",
    label: "Meaning search in demoted layers",
    help: "Whether notes in demoted layers are also searchable by meaning.",
  },
  HATCHDOOR_MCP_ENABLED: {
    section: "agents",
    label: "Let assistants connect (MCP)",
    help: "Opens a second, token-protected door into this vault for AI assistants.",
  },
  HATCHDOOR_MCP_WRITE_ENABLED: {
    section: "agents",
    label: "Let assistants change notes",
    help: "Allows connected assistants to create, edit, move and delete notes and attachments.",
  },
  HATCHDOOR_MCP_BEARER_TOKEN: {
    section: "agents",
    label: "MCP password",
    help: "Required whenever assistants are allowed to connect.",
  },
  HATCHDOOR_MCP_ALLOWED_ORIGINS: {
    section: "agents",
    label: "Websites allowed to connect",
    help: "Browser-based assistants may connect only from these addresses.",
  },
  HATCHDOOR_MAX_ATTACHMENT_BYTES: {
    section: "uploads",
    label: "Largest file from this app",
    help: "The biggest file you can drop into a note from your browser.",
    unit: "MB",
  },
  HATCHDOOR_MCP_MAX_BASE64_BYTES: {
    section: "uploads",
    label: "Largest file from an assistant",
    help: "The biggest file an assistant can send inline.",
    unit: "MB",
  },
  HATCHDOOR_GIT_SYNC_ENABLED: {
    section: "versioning",
    label: "Keep a history of changes",
    help: "Whether Hatchdoor records changes to your vault.",
  },
  HATCHDOOR_GIT_HTTPS_USERNAME: {
    section: "versioning",
    label: "Username",
    help: "The username paired with the access token.",
  },
  HATCHDOOR_GIT_HTTPS_TOKEN: {
    section: "versioning",
    label: "Access token",
    help: "The token that lets Hatchdoor send changes to the server.",
  },
  HATCHDOOR_GIT_DEBOUNCE_SECONDS: {
    section: "versioning",
    label: "Wait before recording",
    help: "How long Hatchdoor waits after activity before recording a batch.",
    unit: "seconds",
  },
  HATCHDOOR_GIT_AUTHOR_NAME: {
    section: "versioning",
    label: "Recorded as (name)",
    help: "The name attached to recorded changes.",
  },
  HATCHDOOR_GIT_AUTHOR_EMAIL: {
    section: "versioning",
    label: "Recorded as (email)",
    help: "The email attached to recorded changes.",
  },
  HATCHDOOR_GIT_BRANCH: {
    section: "versioning",
    label: "Branch",
    help: "The line of history the vault is currently on.",
  },
};

function mbValue(value: string | null): string {
  const bytes = Number(value);
  return Number.isFinite(bytes) ? String(bytes / (1024 * 1024)) : "";
}

function wireValue(setting: Setting, value: string): string {
  return setting.key.includes("BYTES")
    ? String(Math.round(Number(value) * 1024 * 1024))
    : value;
}

function sourceLabel(source: Setting["source"]): string {
  return source === "environment"
    ? "Set in .env"
    : source === "stored"
      ? "Saved here"
      : "Using the default";
}

function lockExplanation(lock: NonNullable<Setting["locked"]>): string {
  return lock === "environment"
    ? "This value comes from your .env file, which always wins. To change it, edit that file and restart Hatchdoor."
    : "Hatchdoor always follows whichever branch your vault folder is on, so there is nothing to choose.";
}

export function SettingsPage() {
  const [settings, setSettings] = useState<Setting[]>([]);
  const [section, setSection] =
    useState<(typeof SECTIONS)[number]["id"]>("vault");
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [message, setMessage] = useState<string | null>(null);
  const [webToken, setWebToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  const load = async () => {
    const response = await apiFetch("/api/settings");
    if (!response.ok) throw new Error("Settings could not be loaded.");
    const payload = (await response.json()) as { settings: Setting[] };
    setSettings(payload.settings);
    setDrafts({});
    setErrors({});
  };

  useEffect(() => {
    void load()
      .catch((error: unknown) =>
        setMessage(
          error instanceof Error
            ? error.message
            : "Settings could not be loaded.",
        ),
      )
      .finally(() => setLoading(false));
  }, []);

  const current = useMemo(
    () => SECTIONS.find((item) => item.id === section)!,
    [section],
  );
  const visible = settings.filter(
    (setting) => COPY[setting.key]?.section === section,
  );
  const editable = visible.filter((setting) => !setting.locked);
  const locked = visible.filter((setting) => setting.locked);
  const changed = (setting: Setting) => drafts[setting.key] !== undefined;

  const save = async () => {
    const updates = Object.fromEntries(
      editable
        .filter(changed)
        .map((setting) => [
          setting.key,
          wireValue(setting, drafts[setting.key]),
        ]),
    );
    if (Object.keys(updates).length === 0) return;
    setSaving(true);
    setErrors({});
    setMessage(null);
    try {
      const response = await apiFetch("/api/settings", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ updates }),
      });
      const payload = (await response.json()) as {
        settings?: Setting[];
        error?: string;
        fields?: { key: string; message: string }[];
      };
      if (!response.ok) {
        setErrors(
          Object.fromEntries(
            (payload.fields ?? []).map((field) => [field.key, field.message]),
          ),
        );
        setMessage(payload.error ?? "Nothing was saved.");
        return;
      }
      setSettings(payload.settings ?? settings);
      setDrafts({});
      setMessage("Saved and applied from server truth.");
    } catch {
      setMessage("Settings could not be saved. Try again.");
    } finally {
      setSaving(false);
    }
  };

  const revealWebToken = async () => {
    setMessage(null);
    try {
      const response = await apiFetch("/api/settings/web-token/reveal", {
        method: "POST",
      });
      if (!response.ok) {
        setMessage("No web access token is configured for this server.");
        return;
      }
      const payload = (await response.json()) as { value?: string };
      setWebToken(payload.value ?? null);
    } catch {
      setMessage("The web access token could not be revealed.");
    }
  };

  if (loading)
    return (
      <main className="settings-page">
        <p>Loading settings…</p>
      </main>
    );
  return (
    <main className="settings-page">
      <header className="settings-header">
        <p className="settings-eyebrow">Server settings</p>
        <h1>Settings</h1>
        <p>Changes apply to this running Hatchdoor server.</p>
        <p className="settings-web-token">
          <button
            onClick={() => {
              if (webToken) setWebToken(null);
              else void revealWebToken();
            }}
          >
            {webToken ? "Hide web access token" : "Show web access token"}
          </button>
          {webToken ? <code>{webToken}</code> : null}
        </p>
      </header>
      <div className="settings-live">
        <span>
          <b>Search index</b> Up to date
        </span>
        <span>
          <b>Versioning</b> Status is managed by the server
        </span>
      </div>
      <div className="settings-layout">
        <nav className="settings-index" aria-label="Settings sections">
          {SECTIONS.map((item) => {
            const rows = settings.filter(
              (setting) => COPY[setting.key]?.section === item.id,
            );
            const editableCount = rows.filter(
              (setting) => !setting.locked,
            ).length;
            return (
              <button
                key={item.id}
                data-active={item.id === section}
                onClick={() => setSection(item.id)}
              >
                <span>{item.number}</span>
                {item.title}
                <small>
                  {editableCount} / {rows.length} editable
                </small>
              </button>
            );
          })}
        </nav>
        <section
          className="settings-section"
          aria-labelledby="settings-section-title"
        >
          <header>
            <p>{current.number}</p>
            <h2 id="settings-section-title">{current.title}</h2>
            <span>{current.blurb}</span>
          </header>
          {message ? (
            <p className="settings-message" role="status">
              {message}
            </p>
          ) : null}
          {editable.map((setting) => (
            <label className="settings-field" key={setting.key}>
              <span>
                {COPY[setting.key].label}
                <code>{setting.key}</code>
                <small>{sourceLabel(setting.source)}</small>
              </span>
              {setting.kind === "switch" ? (
                <input
                  type="checkbox"
                  checked={(drafts[setting.key] ?? setting.value) === "true"}
                  onChange={(event) =>
                    setDrafts((old) => ({
                      ...old,
                      [setting.key]: event.target.checked ? "true" : "false",
                    }))
                  }
                />
              ) : (
                <input
                  type={
                    setting.kind === "number"
                      ? "number"
                      : setting.kind === "secret"
                        ? "password"
                        : "text"
                  }
                  value={
                    drafts[setting.key] ??
                    (setting.key.includes("BYTES")
                      ? mbValue(setting.value)
                      : (setting.value ?? ""))
                  }
                  placeholder={
                    setting.kind === "secret" && setting.configured
                      ? "Configured"
                      : ""
                  }
                  onChange={(event) =>
                    setDrafts((old) => ({
                      ...old,
                      [setting.key]: event.target.value,
                    }))
                  }
                />
              )}
              {COPY[setting.key].unit ? (
                <em>{COPY[setting.key].unit}</em>
              ) : null}
              <small>{COPY[setting.key].help}</small>
              {setting.class === "reindex" ? (
                <small>Saving this rebuilds the search index.</small>
              ) : null}
              {errors[setting.key] ? (
                <strong>{errors[setting.key]}</strong>
              ) : null}
            </label>
          ))}
          {editable.length ? (
            <div className="settings-actions">
              <button
                onClick={() => {
                  setDrafts({});
                  setErrors({});
                }}
                disabled={saving}
              >
                Discard
              </button>
              <button onClick={() => void save()} disabled={saving}>
                {saving ? "Saving…" : `Save ${current.title}`}
              </button>
            </div>
          ) : null}
          {locked.length ? (
            <aside className="settings-plaque">
              <h3>Managed outside this page</h3>
              {locked.map((setting) => (
                <div key={setting.key}>
                  <b>{COPY[setting.key].label}</b>
                  <code>{setting.key}</code>
                  <span>
                    {setting.kind === "secret"
                      ? setting.configured
                        ? "set"
                        : "not set"
                      : setting.value}
                  </span>
                  <p>{lockExplanation(setting.locked!)}</p>
                </div>
              ))}
            </aside>
          ) : null}
        </section>
      </div>
    </main>
  );
}

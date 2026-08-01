/**
 * PROTOTYPE — throwaway. Variant C: "The ledger".
 *
 * Every setting is one row of a single table, and the row that explains where
 * each value comes from is the same row for all of them. Editing happens in
 * place: a row opens into a small editor with its own save, so there is no
 * page-wide dirty state. A locked row simply does not open, and the "Where"
 * column says why — the affordance is the missing chevron plus one column that
 * treats env, saved and default values alike.
 */

import { Fragment, useState } from "react";

import {
  COPY,
  LOCK_COPY,
  REMOTE_ONLY,
  SECTIONS,
  VERSIONING_DETAIL,
  fmtEta,
  fromMb,
  toMb,
  type Setting,
} from "./fixtures";
import type { Sandbox } from "./useSandbox";

export const NAME = "The ledger";

const MODE_LABEL: Record<string, string> = {
  off: "Off",
  local: "On, this machine",
  remote: "On, sending elsewhere",
};

function shown(setting: Setting, sb: Sandbox): string {
  const v = sb.effective(setting.key);
  if (setting.kind === "secret") {
    return (
      sb.revealed[setting.key] ??
      (setting.configured ? "••••••••••••" : "not set")
    );
  }
  if (setting.kind === "switch") return v === "true" ? "On" : "Off";
  if (setting.kind === "mode") return MODE_LABEL[v] ?? v;
  if (setting.key.includes("BYTES")) return `${toMb(v)} MB`;
  if (setting.key.endsWith("SECONDS")) return `${v} seconds`;
  return v || "empty";
}

function whereLabel(setting: Setting): { text: string; tone: string } {
  if (setting.locked === "never") return { text: "Fixed", tone: "fixed" };
  if (setting.locked === "environment")
    return { text: ".env file", tone: "env" };
  if (setting.source === "stored")
    return { text: "Saved here", tone: "stored" };
  return { text: "Default", tone: "default" };
}

export function VariantC({ sb }: { sb: Sandbox }) {
  const [open, setOpen] = useState<string | null>(null);
  const mode = sb.effective("HATCHDOOR_GIT_SYNC_ENABLED");
  const visible = (s: Setting) => {
    if (mode === "off" && VERSIONING_DETAIL.includes(s.key)) return false;
    if (mode === "local" && REMOTE_ONLY.includes(s.key)) return false;
    return true;
  };

  const errorFor = (key: string) =>
    sb.refusal?.kind === "invalid"
      ? sb.refusal.fields.find((f) => f.key === key)
      : undefined;

  return (
    <div className="sp-c">
      <div className="sp-c-console">
        <div className="sp-c-console-main">
          <p className="sp-eyebrow">Server settings</p>
          <h1 className="sp-title">Ledger</h1>
        </div>
        <div className="sp-c-console-stats">
          <div className="sp-c-stat">
            <span className="sp-c-stat-num">{sb.editableCount}</span>
            <span className="sp-c-stat-lbl">editable here</span>
          </div>
          <div className="sp-c-stat">
            <span className="sp-c-stat-num">{sb.pinnedCount}</span>
            <span className="sp-c-stat-lbl">set in .env</span>
          </div>
          <div className="sp-c-stat">
            <span className="sp-c-stat-num">
              <span className={`sp-dot is-${sb.git.lifecycle}`} />
            </span>
            <span className="sp-c-stat-lbl">
              {sb.git.lifecycle === "disabled"
                ? "no history"
                : sb.git.lifecycle === "starting"
                  ? "starting"
                  : sb.git.lifecycle === "stopping"
                    ? "finishing"
                    : sb.git.mode === "local"
                      ? "history, local"
                      : `history, ${sb.git.unpushed ?? 0} to send`}
            </span>
          </div>
          <div className="sp-c-stat">
            <span className="sp-c-stat-num">
              {sb.index.state === "rebuilding"
                ? `${sb.index.percent}%`
                : sb.index.drift
                  ? "!"
                  : "OK"}
            </span>
            <span className="sp-c-stat-lbl">search index</span>
          </div>
        </div>
      </div>

      {sb.index.state === "rebuilding" ? (
        <div className="sp-strip">
          <div
            className="sp-strip-bar"
            style={{ width: `${sb.index.percent}%` }}
          />
          <div className="sp-strip-text">
            Rebuilding search. Answers still come from the previous setting
            until it finishes — {fmtEta(sb.index.etaSeconds)}.
          </div>
        </div>
      ) : null}

      {sb.editableCount === 0 ? (
        <div className="sp-notice">
          Every row below is set in your <code>.env</code> file, so this page is
          a read-only record of how the server is configured. Comment a line out
          of <code>.env</code> and restart to edit it here instead.
        </div>
      ) : null}
      {sb.refusal?.kind === "invalid" ? (
        <div className="sp-notice sp-notice-err">
          {sb.refusal.overall}{" "}
          {sb.refusal.fields
            .filter((f) => f.key && f.key !== open)
            .map((f) => (
              <button
                key={f.key}
                className="sp-link"
                onClick={() => setOpen(f.key!)}
              >
                Open {COPY[f.key!].label}
              </button>
            ))}
        </div>
      ) : null}
      {sb.refusal?.kind === "busy" ? (
        <div className="sp-notice sp-notice-warn">{sb.refusal.message}</div>
      ) : null}

      <table className="sp-c-table">
        <thead>
          <tr>
            <th>Setting</th>
            <th>Value</th>
            <th>Where it is set</th>
            <th />
          </tr>
        </thead>
        {SECTIONS.map((section) => {
          const rows = sb.settings.filter(
            (s) => COPY[s.key].section === section.id && visible(s),
          );
          if (rows.length === 0) return null;
          return (
            <tbody key={section.id}>
              <tr className="sp-c-group">
                <th colSpan={4}>
                  <span className="sp-sec-num">{section.num}</span>{" "}
                  {section.title}
                  <span className="sp-c-group-blurb">{section.blurb}</span>
                </th>
              </tr>
              {rows.map((s) => {
                const where = whereLabel(s);
                const isOpen = open === s.key;
                const err = errorFor(s.key);
                const changed = sb.drafts[s.key] !== undefined;
                return (
                  <Fragment key={s.key}>
                    <tr
                      className={`sp-c-row${isOpen ? " is-open" : ""}${err ? " has-error" : ""}`}
                      data-locked={Boolean(s.locked)}
                    >
                      <td>
                        <div className="sp-c-label">{COPY[s.key].label}</div>
                        <code className="sp-c-key">{s.key}</code>
                      </td>
                      <td className="sp-c-value">
                        {shown(s, sb)}
                        {changed ? (
                          <span className="sp-c-changed">edited</span>
                        ) : null}
                        {s.cls === "reindex" ? (
                          <span className="sp-c-tag">rebuilds search</span>
                        ) : null}
                      </td>
                      <td>
                        <span className={`sp-c-where is-${where.tone}`}>
                          {where.text}
                        </span>
                      </td>
                      <td className="sp-c-action">
                        {s.locked ? (
                          <span
                            className="sp-c-nolock"
                            title={LOCK_COPY[s.locked].why}
                          >
                            —
                          </span>
                        ) : (
                          <button
                            className="sp-mini"
                            onClick={() => setOpen(isOpen ? null : s.key)}
                          >
                            {isOpen ? "Close" : "Edit"}
                          </button>
                        )}
                      </td>
                    </tr>
                    {isOpen ? (
                      <tr className="sp-c-editor-row">
                        <td colSpan={4}>
                          <div className="sp-c-editor">
                            <p className="sp-c-editor-help">
                              {COPY[s.key].help}
                            </p>

                            {s.kind === "switch" ? (
                              <div className="sp-c-editor-control">
                                <label className="sp-a-switch">
                                  <input
                                    type="checkbox"
                                    checked={sb.effective(s.key) === "true"}
                                    onChange={(e) =>
                                      sb.edit(
                                        s.key,
                                        e.target.checked ? "true" : "false",
                                      )
                                    }
                                  />
                                  <span>
                                    {sb.effective(s.key) === "true"
                                      ? "On"
                                      : "Off"}
                                  </span>
                                </label>
                              </div>
                            ) : null}

                            {s.kind === "mode" ? (
                              <div className="sp-b-segmented">
                                {Object.entries(MODE_LABEL).map(
                                  ([id, label]) => (
                                    <button
                                      key={id}
                                      type="button"
                                      data-selected={sb.effective(s.key) === id}
                                      onClick={() => sb.edit(s.key, id)}
                                    >
                                      {label}
                                    </button>
                                  ),
                                )}
                              </div>
                            ) : null}

                            {s.kind === "text" ? (
                              <input
                                className="sp-input"
                                value={sb.effective(s.key)}
                                onChange={(e) => sb.edit(s.key, e.target.value)}
                              />
                            ) : null}

                            {s.kind === "number" ? (
                              <div className="sp-b-inline">
                                <input
                                  className="sp-input sp-input-short"
                                  type="number"
                                  value={
                                    s.key.includes("BYTES")
                                      ? toMb(sb.effective(s.key))
                                      : sb.effective(s.key)
                                  }
                                  onChange={(e) =>
                                    sb.edit(
                                      s.key,
                                      s.key.includes("BYTES")
                                        ? fromMb(e.target.value)
                                        : e.target.value,
                                    )
                                  }
                                />
                                <span className="sp-unit">
                                  {COPY[s.key].hint}
                                </span>
                              </div>
                            ) : null}

                            {s.kind === "secret" ? (
                              <div className="sp-b-inline">
                                <input
                                  className="sp-input"
                                  placeholder="not set"
                                  value={
                                    sb.drafts[s.key] ??
                                    sb.revealed[s.key] ??
                                    (s.configured ? "••••••••••••" : "")
                                  }
                                  readOnly={
                                    sb.drafts[s.key] === undefined &&
                                    !sb.revealed[s.key]
                                  }
                                  onChange={(e) =>
                                    sb.edit(s.key, e.target.value)
                                  }
                                />
                                <button
                                  className="sp-mini"
                                  onClick={() =>
                                    sb.revealed[s.key]
                                      ? sb.hide(s.key)
                                      : sb.reveal(s.key)
                                  }
                                >
                                  {sb.revealed[s.key] ? "Hide" : "Show"}
                                </button>
                                {s.key === "HATCHDOOR_MCP_BEARER_TOKEN" ? (
                                  <button
                                    className="sp-mini"
                                    onClick={() => sb.generate(s.key)}
                                  >
                                    Generate
                                  </button>
                                ) : null}
                              </div>
                            ) : null}

                            {err ? (
                              <p className="sp-error">{err.message}</p>
                            ) : null}

                            {sb.refusal?.kind === "confirm" ? (
                              <div className="sp-c-confirm">
                                <p>{sb.refusal.message}</p>
                                <div className="sp-modal-actions">
                                  <button
                                    className="sp-btn"
                                    onClick={sb.dismissRefusal}
                                  >
                                    Cancel
                                  </button>
                                  <button
                                    className="sp-btn sp-btn-hot"
                                    onClick={() =>
                                      sb.confirmAndSave(
                                        (
                                          sb.refusal as {
                                            consequence:
                                              | "reindex"
                                              | "git_init"
                                              | "git_downgrade";
                                          }
                                        ).consequence,
                                      )
                                    }
                                  >
                                    Go ahead
                                  </button>
                                </div>
                              </div>
                            ) : (
                              <div className="sp-modal-actions">
                                <button
                                  className="sp-btn"
                                  onClick={() => {
                                    sb.discard();
                                    setOpen(null);
                                  }}
                                >
                                  Cancel
                                </button>
                                <button
                                  className="sp-btn sp-btn-hot"
                                  onClick={() => sb.save([s.key])}
                                  disabled={!changed || sb.saving}
                                >
                                  {sb.saving ? "Saving…" : "Save this setting"}
                                </button>
                                {sb.saved ? (
                                  <span className="sp-ok">{sb.saved}</span>
                                ) : null}
                              </div>
                            )}
                          </div>
                        </td>
                      </tr>
                    ) : null}
                  </Fragment>
                );
              })}
            </tbody>
          );
        })}
      </table>

      <p className="sp-c-foot">
        A row marked <span className="sp-c-where is-env">.env file</span> comes
        from your <code>.env</code> file, which always wins over anything saved
        here. Edit that file and restart to change it. A row marked{" "}
        <span className="sp-c-where is-fixed">Fixed</span> is decided by your
        vault folder itself and has nothing to choose.
      </p>
    </div>
  );
}

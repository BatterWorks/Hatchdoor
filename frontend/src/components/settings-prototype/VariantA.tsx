/**
 * PROTOTYPE — throwaway. Variant A: "The long form".
 *
 * One scrolling column in .env.example order, numbered section heads borrowed
 * from the Stats page, and a single page-wide save bar that rises from the
 * bottom the moment anything is dirty. Locked fields keep their row but lose
 * their box: the value prints as text with a small chip naming who owns it.
 */

import { Fragment } from "react";

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

export const NAME = "The long form";

function LockedRow({ setting }: { setting: Setting }) {
  const copy = COPY[setting.key];
  const lock = LOCK_COPY[setting.locked!];
  return (
    <div className="sp-a-field is-locked">
      <div className="sp-a-field-head">
        <span className="sp-a-label">{copy.label}</span>
        <span className={`sp-chip sp-chip-${setting.locked}`}>{lock.chip}</span>
      </div>
      <p className="sp-a-help">{copy.help}</p>
      <div className="sp-a-locked-value">
        {setting.kind === "secret"
          ? setting.configured
            ? "•••••••••••••••• (set)"
            : "not set"
          : setting.value || "empty"}
      </div>
      <p className="sp-a-lock-why">
        {lock.why} <code>{setting.key}</code>
      </p>
    </div>
  );
}

function Field({ setting, sb }: { setting: Setting; sb: Sandbox }) {
  if (setting.locked) return <LockedRow setting={setting} />;

  const copy = COPY[setting.key];
  const error =
    sb.refusal?.kind === "invalid"
      ? sb.refusal.fields.find((f) => f.key === setting.key)
      : undefined;
  const value = sb.effective(setting.key);
  const changed = sb.drafts[setting.key] !== undefined;

  return (
    <div
      className={`sp-a-field${error ? " has-error" : ""}${changed ? " is-changed" : ""}`}
    >
      <div className="sp-a-field-head">
        <span className="sp-a-label">{copy.label}</span>
        {setting.cls === "reindex" ? (
          <span className="sp-chip sp-chip-reindex">Rebuilds search</span>
        ) : null}
        {changed ? (
          <span className="sp-chip sp-chip-changed">Unsaved</span>
        ) : null}
      </div>
      <p className="sp-a-help">{copy.help}</p>

      {setting.kind === "switch" ? (
        <label className="sp-a-switch">
          <input
            type="checkbox"
            checked={value === "true"}
            onChange={(e) =>
              sb.edit(setting.key, e.target.checked ? "true" : "false")
            }
          />
          <span>{value === "true" ? "On" : "Off"}</span>
        </label>
      ) : null}

      {setting.kind === "mode" ? (
        <div className="sp-a-modes">
          {[
            { id: "off", label: "Off", note: "No history is kept." },
            {
              id: "local",
              label: "On this machine",
              note: "Nothing leaves this computer.",
            },
            {
              id: "remote",
              label: "Send elsewhere",
              note: "Also pushed to your server.",
            },
          ].map((m) => (
            <label
              key={m.id}
              className="sp-a-mode"
              data-selected={value === m.id}
            >
              <input
                type="radio"
                name={`${setting.key}-a`}
                checked={value === m.id}
                onChange={() => sb.edit(setting.key, m.id)}
              />
              <span className="sp-a-mode-label">{m.label}</span>
              <span className="sp-a-mode-note">{m.note}</span>
            </label>
          ))}
        </div>
      ) : null}

      {setting.kind === "text" ? (
        <input
          className="sp-input"
          type="text"
          value={value}
          onChange={(e) => sb.edit(setting.key, e.target.value)}
        />
      ) : null}

      {setting.kind === "number" ? (
        <div className="sp-a-inline">
          <input
            className="sp-input sp-input-short"
            type="number"
            value={setting.key.includes("BYTES") ? toMb(value) : value}
            onChange={(e) =>
              sb.edit(
                setting.key,
                setting.key.includes("BYTES")
                  ? fromMb(e.target.value)
                  : e.target.value,
              )
            }
          />
          <span className="sp-unit">{copy.hint}</span>
        </div>
      ) : null}

      {setting.kind === "secret" ? (
        <div className="sp-a-inline">
          <input
            className="sp-input"
            type="text"
            value={
              sb.drafts[setting.key] ??
              sb.revealed[setting.key] ??
              (setting.configured ? "••••••••••••••••" : "")
            }
            placeholder="not set"
            readOnly={
              sb.drafts[setting.key] === undefined && !sb.revealed[setting.key]
            }
            onChange={(e) => sb.edit(setting.key, e.target.value)}
          />
          {sb.revealed[setting.key] ? (
            <button className="sp-mini" onClick={() => sb.hide(setting.key)}>
              Hide
            </button>
          ) : (
            <button className="sp-mini" onClick={() => sb.reveal(setting.key)}>
              Show
            </button>
          )}
          {setting.key === "HATCHDOOR_MCP_BEARER_TOKEN" ? (
            <button
              className="sp-mini"
              onClick={() => sb.generate(setting.key)}
            >
              Generate
            </button>
          ) : null}
          {sb.drafts[setting.key] !== undefined ? (
            <button
              className="sp-mini"
              onClick={() => sb.edit(setting.key, "")}
            >
              Clear
            </button>
          ) : null}
        </div>
      ) : null}

      {setting.key === "HATCHDOOR_MCP_BEARER_TOKEN" ? (
        <p className="sp-a-aside">
          Changing this also changes who can upload files to your vault, not
          only who can talk to assistants.
        </p>
      ) : null}

      {error ? <p className="sp-error">{error.message}</p> : null}
      {!error && copy.hint && setting.kind !== "number" ? (
        <p className="sp-a-hint">{copy.hint}</p>
      ) : null}
      <code className="sp-a-key">{setting.key}</code>
    </div>
  );
}

export function VariantA({ sb }: { sb: Sandbox }) {
  const mode = sb.effective("HATCHDOOR_GIT_SYNC_ENABLED");
  const visible = (s: Setting) => {
    if (mode === "off" && VERSIONING_DETAIL.includes(s.key)) return false;
    if (mode === "local" && REMOTE_ONLY.includes(s.key)) return false;
    return true;
  };
  const allPinned = sb.editableCount === 0;

  return (
    <div className="sp-a">
      <div className="sp-a-header">
        <p className="sp-eyebrow">Server · this instance</p>
        <h1 className="sp-title">Settings</h1>
        <p className="sp-a-sub">
          These are settings for the whole server, not just this browser.
          Everyone who opens Hatchdoor sees the same values.
        </p>
      </div>

      {allPinned ? (
        <div className="sp-notice">
          All {sb.pinnedCount} settings on this page are set in your{" "}
          <code>.env</code> file, so they are shown here but cannot be changed.
          That is a perfectly normal way to run Hatchdoor. To edit one here
          instead, comment out its line in <code>.env</code> and restart.
        </div>
      ) : sb.pinnedCount > 0 ? (
        <div className="sp-notice sp-notice-quiet">
          {sb.pinnedCount} settings are set in your <code>.env</code> file and
          are shown here as read-only.
        </div>
      ) : null}

      {sb.index.state === "rebuilding" ? (
        <div className="sp-strip">
          <div
            className="sp-strip-bar"
            style={{ width: `${sb.index.percent}%` }}
          />
          <div className="sp-strip-text">
            <strong>Rebuilding search — {sb.index.percent}%.</strong> Search
            keeps working and keeps answering from your previous setting until
            this finishes. {fmtEta(sb.index.etaSeconds)}.
          </div>
        </div>
      ) : null}
      {sb.index.state === "idle" && sb.index.drift ? (
        <div className="sp-notice sp-notice-warn">
          Search is still answering from the old setting. The last rebuild did
          not finish. Your next change, or a restart, starts it again.
        </div>
      ) : null}

      {SECTIONS.map((section) => {
        const fields = sb.settings.filter(
          (s) => COPY[s.key].section === section.id && visible(s),
        );
        if (fields.length === 0) return null;
        return (
          <section className="sp-a-section" key={section.id}>
            <div className="sp-sec-head">
              <span className="sp-sec-num">{section.num}</span>
              <h2 className="sp-sec-title">{section.title}</h2>
            </div>
            <p className="sp-sec-blurb">{section.blurb}</p>

            {section.id === "versioning" && sb.git.lifecycle !== "disabled" ? (
              <div className="sp-git-line">
                <span className={`sp-dot is-${sb.git.lifecycle}`} />
                <span>
                  {sb.git.lifecycle === "starting"
                    ? "Starting…"
                    : sb.git.lifecycle === "stopping"
                      ? "Finishing the last batch…"
                      : sb.git.mode === "local"
                        ? "Recording changes on this machine"
                        : "Recording and sending changes"}
                </span>
                {sb.git.lastCommit ? (
                  <span className="sp-muted">
                    last change recorded {sb.git.lastCommit}
                  </span>
                ) : null}
                {sb.git.mode === "remote" && sb.git.unpushed != null ? (
                  <span className="sp-muted">
                    {sb.git.unpushed} waiting to send
                  </span>
                ) : null}
              </div>
            ) : null}

            {fields.map((s) => (
              <Fragment key={s.key}>
                <Field setting={s} sb={sb} />
              </Fragment>
            ))}
          </section>
        );
      })}

      <div className="sp-a-spacer" />

      {sb.dirty.length > 0 || sb.saving || sb.saved || sb.refusal ? (
        <div className="sp-a-savebar">
          <div className="sp-a-savebar-text">
            {sb.refusal?.kind === "invalid" ? (
              <span className="sp-error">{sb.refusal.overall}</span>
            ) : sb.refusal?.kind === "busy" ? (
              <span className="sp-error">{sb.refusal.message}</span>
            ) : sb.saved ? (
              <span>{sb.saved}</span>
            ) : (
              <span>
                {sb.dirty.length} unsaved{" "}
                {sb.dirty.length === 1 ? "change" : "changes"}
              </span>
            )}
          </div>
          <button
            className="sp-btn"
            onClick={sb.discard}
            disabled={sb.dirty.length === 0}
          >
            Discard
          </button>
          <button
            className="sp-btn sp-btn-hot"
            onClick={() => sb.save()}
            disabled={sb.saving || sb.dirty.length === 0}
          >
            {sb.saving ? "Saving…" : "Save changes"}
          </button>
        </div>
      ) : null}

      {sb.refusal?.kind === "confirm" ? (
        <div className="sp-modal-back">
          <div className="sp-modal">
            <h3>Before this is saved</h3>
            <p>{sb.refusal.message}</p>
            <div className="sp-modal-actions">
              <button className="sp-btn" onClick={sb.dismissRefusal}>
                Cancel
              </button>
              <button
                className="sp-btn sp-btn-hot"
                onClick={() =>
                  sb.confirmAndSave(
                    (
                      sb.refusal as {
                        consequence: "reindex" | "git_init" | "git_downgrade";
                      }
                    ).consequence,
                  )
                }
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

/**
 * PROTOTYPE — throwaway. Variant B: "One section at a time".
 *
 * A sticky index on the left, one section on the right, and the save action
 * lives in the section head rather than the page. Locked settings are pulled
 * out of the form entirely into a "managed elsewhere" plaque at the foot of
 * their section: they are facts about the server, not controls, so they never
 * sit in the tab order between two editable boxes.
 */

import { useState } from "react";

import {
  COPY,
  LOCK_COPY,
  REMOTE_ONLY,
  SECTIONS,
  VERSIONING_DETAIL,
  fmtEta,
  fromMb,
  toMb,
  type SectionId,
  type Setting,
} from "./fixtures";
import type { Sandbox } from "./useSandbox";

export const NAME = "One section at a time";

function Control({ setting, sb }: { setting: Setting; sb: Sandbox }) {
  const value = sb.effective(setting.key);
  const copy = COPY[setting.key];

  switch (setting.kind) {
    case "switch":
      return (
        <button
          type="button"
          className="sp-b-toggle"
          data-on={value === "true"}
          onClick={() =>
            sb.edit(setting.key, value === "true" ? "false" : "true")
          }
        >
          <span className="sp-b-toggle-track">
            <span className="sp-b-toggle-knob" />
          </span>
          <span>{value === "true" ? "On" : "Off"}</span>
        </button>
      );
    case "mode":
      return (
        <div className="sp-b-segmented" role="group">
          {[
            { id: "off", label: "Off" },
            { id: "local", label: "This machine" },
            { id: "remote", label: "Send elsewhere" },
          ].map((m) => (
            <button
              key={m.id}
              type="button"
              data-selected={value === m.id}
              onClick={() => sb.edit(setting.key, m.id)}
            >
              {m.label}
            </button>
          ))}
        </div>
      );
    case "number":
      return (
        <div className="sp-b-inline">
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
      );
    case "secret":
      return (
        <div className="sp-b-inline">
          <input
            className="sp-input"
            type="text"
            placeholder="not set"
            value={
              sb.drafts[setting.key] ??
              sb.revealed[setting.key] ??
              (setting.configured ? "••••••••••••••••" : "")
            }
            readOnly={
              sb.drafts[setting.key] === undefined && !sb.revealed[setting.key]
            }
            onChange={(e) => sb.edit(setting.key, e.target.value)}
          />
          <button
            className="sp-mini"
            onClick={() =>
              sb.revealed[setting.key]
                ? sb.hide(setting.key)
                : sb.reveal(setting.key)
            }
          >
            {sb.revealed[setting.key] ? "Hide" : "Show"}
          </button>
          {setting.key === "HATCHDOOR_MCP_BEARER_TOKEN" ? (
            <button
              className="sp-mini"
              onClick={() => sb.generate(setting.key)}
            >
              Generate
            </button>
          ) : null}
        </div>
      );
    default:
      return (
        <input
          className="sp-input"
          type="text"
          value={value}
          onChange={(e) => sb.edit(setting.key, e.target.value)}
        />
      );
  }
}

function Row({ setting, sb }: { setting: Setting; sb: Sandbox }) {
  const copy = COPY[setting.key];
  const error =
    sb.refusal?.kind === "invalid"
      ? sb.refusal.fields.find((f) => f.key === setting.key)
      : undefined;
  const changed = sb.drafts[setting.key] !== undefined;

  return (
    <div className={`sp-b-row${error ? " has-error" : ""}`}>
      <div className="sp-b-row-left">
        <div className="sp-b-row-label">
          {copy.label}
          {changed ? (
            <span className="sp-b-dirty" aria-label="unsaved" />
          ) : null}
        </div>
        <p className="sp-b-row-help">{copy.help}</p>
        {setting.cls === "reindex" ? (
          <p className="sp-b-row-class">
            Saving this rebuilds the search index.
          </p>
        ) : null}
        {setting.key === "HATCHDOOR_MCP_BEARER_TOKEN" ? (
          <p className="sp-b-row-class">
            This password also controls who can upload files, not only who can
            talk to assistants.
          </p>
        ) : null}
        {error ? <p className="sp-error">{error.message}</p> : null}
      </div>
      <div className="sp-b-row-right">
        <Control setting={setting} sb={sb} />
      </div>
    </div>
  );
}

function Plaque({ settings }: { settings: Setting[] }) {
  if (settings.length === 0) return null;
  const kinds = new Set(settings.map((s) => s.locked));
  return (
    <div className="sp-b-plaque">
      <div className="sp-b-plaque-head">Managed outside this page</div>
      <dl>
        {settings.map((s) => (
          <div className="sp-b-plaque-row" key={s.key}>
            <dt>
              {COPY[s.key].label}
              <code>{s.key}</code>
            </dt>
            <dd>
              {s.kind === "secret"
                ? s.configured
                  ? "set"
                  : "not set"
                : s.value || "empty"}
            </dd>
          </div>
        ))}
      </dl>
      {[...kinds].map((k) => (
        <p className="sp-b-plaque-why" key={k}>
          {LOCK_COPY[k!].why}
        </p>
      ))}
    </div>
  );
}

export function VariantB({ sb }: { sb: Sandbox }) {
  const [active, setActive] = useState<SectionId>("vault");
  const mode = sb.effective("HATCHDOOR_GIT_SYNC_ENABLED");
  const visible = (s: Setting) => {
    if (mode === "off" && VERSIONING_DETAIL.includes(s.key)) return false;
    if (mode === "local" && REMOTE_ONLY.includes(s.key)) return false;
    return true;
  };

  const inSection = (id: SectionId) =>
    sb.settings.filter((s) => COPY[s.key].section === id && visible(s));
  const section = SECTIONS.find((s) => s.id === active)!;
  const fields = inSection(active);
  const editable = fields.filter((s) => !s.locked);
  const locked = fields.filter((s) => s.locked);
  const sectionDirty = editable
    .filter((s) => sb.drafts[s.key] !== undefined)
    .map((s) => s.key);

  return (
    <div className="sp-b">
      <aside className="sp-b-index">
        <p className="sp-eyebrow">Server settings</p>
        <nav>
          {SECTIONS.map((s) => {
            const items = inSection(s.id);
            const dirty = items.filter(
              (i) => sb.drafts[i.key] !== undefined,
            ).length;
            return (
              <button
                key={s.id}
                type="button"
                className="sp-b-index-item"
                data-active={s.id === active}
                onClick={() => setActive(s.id)}
              >
                <span className="sp-b-index-num">{s.num}</span>
                <span className="sp-b-index-title">{s.title}</span>
                {dirty > 0 ? <span className="sp-b-dirty" /> : null}
                <span className="sp-b-index-count">
                  {items.filter((i) => !i.locked).length}/{items.length}
                </span>
              </button>
            );
          })}
        </nav>
        <div className="sp-b-index-foot">
          {sb.editableCount === 0 ? (
            <p>
              Every setting is set in <code>.env</code>. Nothing on this page
              can be changed from here.
            </p>
          ) : (
            <p>
              {sb.editableCount} editable here, {sb.pinnedCount} set in{" "}
              <code>.env</code>.
            </p>
          )}
        </div>
      </aside>

      <div className="sp-b-main">
        <div className="sp-b-console">
          <div className="sp-b-console-cell">
            <span className="sp-b-console-lbl">Search index</span>
            {sb.index.state === "rebuilding" ? (
              <>
                <span className="sp-b-console-val">
                  Rebuilding {sb.index.percent}%
                </span>
                <div className="sp-b-mini-bar">
                  <span style={{ width: `${sb.index.percent}%` }} />
                </div>
                <span className="sp-muted">
                  Still answering from the old setting ·{" "}
                  {fmtEta(sb.index.etaSeconds)}
                </span>
              </>
            ) : sb.index.drift ? (
              <span className="sp-b-console-val sp-warn">
                Behind your settings
              </span>
            ) : (
              <span className="sp-b-console-val">Up to date</span>
            )}
          </div>
          <div className="sp-b-console-cell">
            <span className="sp-b-console-lbl">Versioning</span>
            <span className="sp-b-console-val">
              <span className={`sp-dot is-${sb.git.lifecycle}`} />
              {sb.git.lifecycle === "disabled"
                ? "Off"
                : sb.git.lifecycle === "starting"
                  ? "Starting…"
                  : sb.git.lifecycle === "stopping"
                    ? "Finishing…"
                    : sb.git.mode === "local"
                      ? "On, this machine"
                      : "On, sending"}
            </span>
            {sb.git.lastCommit ? (
              <span className="sp-muted">
                last recorded {sb.git.lastCommit}
              </span>
            ) : null}
            {sb.git.mode === "remote" && sb.git.unpushed != null ? (
              <span className="sp-muted">
                {sb.git.unpushed} waiting to send
              </span>
            ) : null}
          </div>
        </div>

        <div className="sp-b-sec-head">
          <div>
            <h2 className="sp-sec-title">
              <span className="sp-sec-num">{section.num}</span> {section.title}
            </h2>
            <p className="sp-sec-blurb">{section.blurb}</p>
          </div>
          {/* A section with nothing to edit is a record, not a form: no dead
              save button above a plaque holding all its content. */}
          {editable.length === 0 ? null : (
            <div className="sp-b-sec-actions">
              {sb.saved ? <span className="sp-ok">{sb.saved}</span> : null}
              <button
                className="sp-btn"
                onClick={sb.discard}
                disabled={sectionDirty.length === 0}
              >
                Discard
              </button>
              <button
                className="sp-btn sp-btn-hot"
                onClick={() => sb.save(sectionDirty)}
                disabled={sectionDirty.length === 0 || sb.saving}
              >
                {sb.saving ? "Saving…" : `Save ${section.title.toLowerCase()}`}
              </button>
            </div>
          )}
        </div>

        {sb.refusal?.kind === "invalid" ? (
          <div className="sp-notice sp-notice-err">{sb.refusal.overall}</div>
        ) : null}
        {sb.refusal?.kind === "busy" ? (
          <div className="sp-notice sp-notice-warn">{sb.refusal.message}</div>
        ) : null}

        <div className="sp-b-rows" data-empty={editable.length === 0}>
          {editable.map((s) => (
            <Row key={s.key} setting={s} sb={sb} />
          ))}
        </div>

        <Plaque settings={locked} />
      </div>

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

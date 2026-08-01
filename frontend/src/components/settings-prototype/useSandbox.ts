/**
 * PROTOTYPE — throwaway. In-memory stand-in for the settings API, so all three
 * variants argue about layout over identical behaviour. No network, no
 * persistence: reloading resets everything.
 *
 * It fakes the three refusal shapes from the API contract (#55): per-field
 * validation, a confirmation that must be accepted and re-sent, and a busy
 * refusal while versioning drains.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  MAX_UPLOAD_BYTES,
  payloadFor,
  type GitStatus,
  type IndexStatus,
  type Payload,
  type Scenario,
  type Setting,
} from "./fixtures";

export type FieldError = { key: string | null; message: string };

export type Consequence = "reindex" | "git_init" | "git_downgrade";

export type SaveRefusal =
  | { kind: "invalid"; overall: string; fields: FieldError[] }
  | { kind: "confirm"; consequence: Consequence; message: string }
  | { kind: "busy"; message: string };

export type Sandbox = ReturnType<typeof useSandbox>;

export function useSandbox(scenario: Scenario) {
  const [payload, setPayload] = useState<Payload>(() => payloadFor(scenario));
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [refusal, setRefusal] = useState<SaveRefusal | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<Record<string, string>>({});
  const [acceptedGitInit, setAcceptedGitInit] = useState(false);
  const timers = useRef<number[]>([]);

  // A scenario switch is a fresh page load.
  useEffect(() => {
    setPayload(payloadFor(scenario));
    setDrafts({});
    setRefusal(null);
    setSaved(null);
    setRevealed({});
    setAcceptedGitInit(false);
  }, [scenario]);

  useEffect(
    () => () => {
      timers.current.forEach((t) => window.clearTimeout(t));
    },
    [],
  );

  const later = useCallback((fn: () => void, ms: number) => {
    timers.current.push(window.setTimeout(fn, ms));
  }, []);

  const byKey = useMemo(() => {
    const map: Record<string, Setting> = {};
    payload.settings.forEach((s) => {
      map[s.key] = s;
    });
    return map;
  }, [payload.settings]);

  const effective = useCallback(
    (key: string): string => drafts[key] ?? byKey[key]?.value ?? "",
    [drafts, byKey],
  );

  const dirty = Object.keys(drafts);

  const edit = useCallback((key: string, value: string) => {
    setSaved(null);
    setRefusal(null);
    setDrafts((prev) => ({ ...prev, [key]: value }));
  }, []);

  const discard = useCallback(() => {
    setDrafts({});
    setRefusal(null);
    setSaved(null);
  }, []);

  const reveal = useCallback((key: string) => {
    setRevealed((prev) => ({
      ...prev,
      [key]: key.includes("MCP")
        ? "hd_mcp_9f3c1a77e21b4d05"
        : "ghp_R7xK2mQ1vB8sT4wZ",
    }));
  }, []);

  const hide = useCallback((key: string) => {
    setRevealed((prev) => {
      const next = { ...prev };
      delete next[key];
      return next;
    });
  }, []);

  /** Fills the box; persists nothing until a normal save (#55). */
  const generate = useCallback(
    (key: string) => {
      edit(key, `hd_mcp_${Math.random().toString(16).slice(2, 18)}`);
    },
    [edit],
  );

  function validate(next: Record<string, string>): FieldError[] {
    const errors: FieldError[] = [];
    const val = (k: string) => next[k] ?? byKey[k]?.value ?? "";
    const configured = (k: string) =>
      next[k] !== undefined ? next[k] !== "" : Boolean(byKey[k]?.configured);

    if (
      val("HATCHDOOR_MCP_ENABLED") === "true" &&
      !configured("HATCHDOOR_MCP_BEARER_TOKEN")
    ) {
      errors.push({
        key: "HATCHDOOR_MCP_BEARER_TOKEN",
        message:
          "Assistants cannot be let in without a password. Generate one, or turn the connection back off.",
      });
    }
    if (
      val("HATCHDOOR_GIT_SYNC_ENABLED") === "remote" &&
      !configured("HATCHDOOR_GIT_HTTPS_TOKEN")
    ) {
      errors.push({
        key: "HATCHDOOR_GIT_HTTPS_TOKEN",
        message:
          "Sending changes elsewhere needs an access token. Paste one, or choose to keep the history on this machine.",
      });
    }
    for (const key of [
      "HATCHDOOR_MAX_ATTACHMENT_BYTES",
      "HATCHDOOR_MCP_MAX_BASE64_BYTES",
    ]) {
      const bytes = Number(val(key));
      if (!Number.isFinite(bytes) || bytes <= 0) {
        errors.push({ key, message: "Enter a size larger than zero." });
      } else if (bytes > MAX_UPLOAD_BYTES) {
        errors.push({
          key,
          message:
            "That is too large to be safe. Hatchdoor holds a whole upload in memory while it writes it, so a limit above 512 MB can take the server down. 512 MB is the most it will accept.",
        });
      }
    }
    const debounce = Number(val("HATCHDOOR_GIT_DEBOUNCE_SECONDS"));
    if (!Number.isFinite(debounce) || debounce < 1) {
      errors.push({
        key: "HATCHDOOR_GIT_DEBOUNCE_SECONDS",
        message: "Enter a whole number of seconds, at least 1.",
      });
    }
    return errors;
  }

  const save = useCallback(
    (keys?: string[], accepted?: Consequence) => {
      const scope = keys ?? Object.keys(drafts);
      const patch: Record<string, string> = {};
      scope.forEach((k) => {
        if (drafts[k] !== undefined) patch[k] = drafts[k];
      });
      if (Object.keys(patch).length === 0) return;

      setRefusal(null);
      setSaving(true);

      later(() => {
        setSaving(false);

        const errors = validate(patch);
        if (errors.length > 0) {
          setRefusal({
            kind: "invalid",
            overall:
              "Nothing was saved. Two settings disagree with each other.",
            fields: errors,
          });
          return;
        }

        // Busy: versioning is still shutting down.
        if (
          patch.HATCHDOOR_GIT_SYNC_ENABLED !== undefined &&
          payload.git.lifecycle === "stopping"
        ) {
          setRefusal({
            kind: "busy",
            message:
              "Still finishing the last batch of changes. Try again in a few seconds — nothing was lost.",
          });
          return;
        }

        // Confirmations, one at a time, accepted then re-sent.
        const mode = patch.HATCHDOOR_GIT_SYNC_ENABLED;
        if (mode === "local" && !acceptedGitInit && accepted !== "git_init") {
          setRefusal({
            kind: "confirm",
            consequence: "git_init",
            message:
              "Your notes folder has no history yet, so Hatchdoor will start one. This creates a hidden folder inside it that grows for good: every image and PDF you add stays in there even after you delete the note.",
          });
          return;
        }
        if (
          mode === "local" &&
          byKey.HATCHDOOR_GIT_SYNC_ENABLED?.value === "remote" &&
          accepted !== "git_downgrade"
        ) {
          setRefusal({
            kind: "confirm",
            consequence: "git_downgrade",
            message:
              "From now on your notes stop leaving this machine. The history stays here, and nothing is sent to your server until you switch back.",
          });
          return;
        }
        const touchesIndex =
          patch.HATCHDOOR_EXCLUDE !== undefined ||
          patch.HATCHDOOR_EMBED_LAYERS !== undefined;
        if (touchesIndex && accepted !== "reindex") {
          setRefusal({
            kind: "confirm",
            consequence: "reindex",
            message:
              "Saving this rebuilds the search index. The setting takes effect right away and search keeps working the whole time — it just keeps answering from the old setting until the rebuild finishes.",
          });
          return;
        }

        // Apply.
        setPayload((prev) => {
          const settings = prev.settings.map((s) =>
            patch[s.key] !== undefined
              ? {
                  ...s,
                  value: s.kind === "secret" ? null : patch[s.key],
                  configured:
                    s.kind === "secret" ? patch[s.key] !== "" : s.configured,
                  source: "stored" as const,
                }
              : s,
          );
          let git: GitStatus = prev.git;
          if (patch.HATCHDOOR_GIT_SYNC_ENABLED !== undefined) {
            const m = patch.HATCHDOOR_GIT_SYNC_ENABLED as GitStatus["mode"];
            git = {
              lifecycle: m === "off" ? "stopping" : "starting",
              mode: m,
              unpushed: m === "remote" ? 0 : null,
              lastCommit: prev.git.lastCommit,
              lastError: null,
            };
            later(
              () =>
                setPayload((p) => ({
                  ...p,
                  git: {
                    ...p.git,
                    lifecycle: m === "off" ? "disabled" : "running",
                    lastCommit: m === "off" ? p.git.lastCommit : "just now",
                  },
                })),
              2600,
            );
          }
          let index: IndexStatus = prev.index;
          if (touchesIndex) {
            index = {
              state: "rebuilding",
              percent: 4,
              etaSeconds: 320,
              drift: true,
              lastFailure: null,
            };
          }
          return { settings, git, index };
        });
        if (patch.HATCHDOOR_GIT_SYNC_ENABLED === "local")
          setAcceptedGitInit(true);
        setDrafts((prev) => {
          const next = { ...prev };
          scope.forEach((k) => delete next[k]);
          return next;
        });
        setSaved(
          touchesIndex
            ? "Saved. Rebuilding search in the background."
            : "Saved.",
        );
        later(() => setSaved(null), 4000);
      }, 420);
    },
    [drafts, byKey, payload.git.lifecycle, acceptedGitInit, later],
  );

  const confirmAndSave = useCallback(
    (consequence: Consequence) => {
      const scope = Object.keys(drafts);
      setRefusal(null);
      // Re-send with the consequence accepted.
      save(scope, consequence);
    },
    [drafts, save],
  );

  // Rebuild ticks along so the strip is judged in motion, not as a still.
  useEffect(() => {
    if (payload.index.state !== "rebuilding") return;
    const id = window.setInterval(() => {
      setPayload((prev) => {
        if (prev.index.state !== "rebuilding") return prev;
        const percent = prev.index.percent + 3;
        if (percent >= 100) {
          return {
            ...prev,
            index: {
              state: "idle",
              percent: 0,
              etaSeconds: null,
              drift: false,
              lastFailure: null,
            },
          };
        }
        return {
          ...prev,
          index: {
            ...prev.index,
            percent,
            etaSeconds: Math.max(5, Math.round(((100 - percent) / 100) * 420)),
          },
        };
      });
    }, 900);
    return () => window.clearInterval(id);
  }, [payload.index.state]);

  const pinnedCount = payload.settings.filter(
    (s) => s.locked === "environment",
  ).length;
  const editableCount = payload.settings.filter(
    (s) => s.locked === null,
  ).length;

  return {
    settings: payload.settings,
    byKey,
    git: payload.git,
    index: payload.index,
    effective,
    drafts,
    dirty,
    edit,
    discard,
    save,
    confirmAndSave,
    refusal,
    dismissRefusal: () => setRefusal(null),
    saving,
    saved,
    revealed,
    reveal,
    hide,
    generate,
    pinnedCount,
    editableCount,
  };
}

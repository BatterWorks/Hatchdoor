import { useCallback, useEffect, useRef, useState } from "react";

import { apiFetch } from "../api/api";
import { safeGetItem, safeSetItem } from "../lib/storage";

export type StartupStatus =
  | { state: "terms_required" }
  | {
      state: "downloading";
      /** `download` while the files arrive, `load` once they are on disk
       * and the model is loading (#642). */
      step?: "download" | "load";
      model?: string;
      downloaded_bytes?: number;
      total_bytes?: number;
      percent?: number;
    }
  | { state: "scanning" }
  | {
      state: "indexing";
      notes_completed: number;
      notes_total: number;
      chunks_completed: number;
      chunks_total: number;
      tokens_completed: number;
      tokens_total: number;
      percent: number;
      eta_seconds?: number;
    }
  | { state: "ready" }
  | { state: "failed"; message?: string };

const POLL_INTERVAL_MS = 1_000;
/** Ceiling for the retry delay while the status route keeps failing. Doubling
 * from `POLL_INTERVAL_MS` reaches it after five failures. */
const MAX_FAILURE_BACKOFF_MS = 30_000;

/** The delay before the next poll after `failures` consecutive failed polls:
 * the normal interval at zero, doubling per failure up to the ceiling, so an
 * offline or restarting server is not asked once a second forever (#304). */
export function startupPollDelay(failures: number): number {
  if (failures <= 0) {
    return POLL_INTERVAL_MS;
  }
  return Math.min(POLL_INTERVAL_MS * 2 ** failures, MAX_FAILURE_BACKOFF_MS);
}
/** Shown when a model-setup request never reached the server. */
export const MODEL_SETUP_CONNECTION_ERROR =
  "Could not reach Hatchdoor. Check the connection and try again.";

/** The server's own sentence from a refused model-setup request
 * (`{ "error": "..." }`), or a status-based one when the body has none. */
async function modelSetupErrorMessage(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as { error?: unknown };
    if (typeof body.error === "string" && body.error.trim().length > 0) {
      return body.error;
    }
  } catch {
    // Not JSON: fall through to the status-based sentence.
  }
  return `Model setup could not be started (HTTP ${response.status}).`;
}

const HAS_STEPPED_PAST_GATE_KEY = "hatchdoor:startup-gate-stepped-past";

/**
 * Polls `/api/startup-status` and owns the model-setup actions
 * (accept/decline Gemma terms, retry). Shared between `StartupGate` (the
 * full-screen model-step gate, #150) and the rest of the shell, which
 * surfaces the same data in the Scope zone slot and the search dialog once
 * the gate has stepped aside.
 *
 * Polling is enabled only after ordinary Vault discovery has resolved, never
 * for zero-Vault or broken-registry workspaces (#150). It stops once the
 * backend reports `ready` or `failed`. A poll that fails (network error or a
 * non-2xx answer) backs off, doubling the delay up to 30s, and the first poll
 * that succeeds returns to the 1s interval (#304). A model-setup action
 * (accept/decline/retry) optimistically moves local state to `downloading`
 * and explicitly resumes polling, since the loop would otherwise stay parked
 * at its last terminal state forever — required for a real retry after a
 * failed model download to ever reach `ready` again. A refused or failed
 * action leaves the state as it was and sets `actionError` instead (#642).
 */
export function useStartupStatus(enabled = true) {
  const [status, setStatus] = useState<StartupStatus | null>(null);
  const [connectionIssue, setConnectionIssue] = useState(false);
  /** Why the last accept, decline or retry request did not start setup. */
  const [actionError, setActionError] = useState<string | null>(null);
  // The gate is an instance-first-run surface, not a mount-first-run surface:
  // retain the latch across a browser reload so a retry never re-blocks a
  // client that has already reached the workspace.
  const [hasSteppedPastGate, setHasSteppedPastGate] = useState(
    () => safeGetItem(HAS_STEPPED_PAST_GATE_KEY) === "1",
  );
  const hasSteppedPastGateRef = useRef(hasSteppedPastGate);
  const activeRef = useRef(true);
  const timerRef = useRef<number | undefined>(undefined);
  const failuresRef = useRef(0);
  const lastStateRef = useRef<StartupStatus["state"] | null>(null);
  // Holds the latest `poll` so the scheduled setTimeout and the model-setup
  // actions below can trigger a re-poll without `poll` referencing itself
  // directly (a stale-closure footgun the react-hooks lint rule now flags).
  const pollRef = useRef<() => Promise<void>>(async () => {});

  const poll = useCallback(async () => {
    let shouldPoll = true;
    try {
      const response = await fetch("/api/startup-status", {
        cache: "no-store",
      });
      if (!response.ok) {
        throw new Error(`startup status returned ${response.status}`);
      }
      const next = (await response.json()) as StartupStatus;
      if (!activeRef.current) return;
      failuresRef.current = 0;
      // A refusal explains the state it was made in. Once the server has
      // moved on, say because another client started setup, it no longer
      // applies and must not sit under a different button.
      if (
        lastStateRef.current !== null &&
        lastStateRef.current !== next.state
      ) {
        setActionError(null);
      }
      lastStateRef.current = next.state;
      setStatus(next);
      setConnectionIssue(false);
      if (
        !hasSteppedPastGateRef.current &&
        next.state !== "terms_required" &&
        next.state !== "downloading"
      ) {
        hasSteppedPastGateRef.current = true;
        safeSetItem(HAS_STEPPED_PAST_GATE_KEY, "1");
        setHasSteppedPastGate(true);
      }
      shouldPoll = next.state !== "ready" && next.state !== "failed";
    } catch {
      if (!activeRef.current) return;
      failuresRef.current += 1;
      setConnectionIssue(true);
    }

    if (activeRef.current && shouldPoll) {
      if (timerRef.current !== undefined) window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(
        () => void pollRef.current(),
        startupPollDelay(failuresRef.current),
      );
    }
  }, []);
  pollRef.current = poll;

  useEffect(() => {
    if (!enabled) {
      activeRef.current = false;
      if (timerRef.current !== undefined) window.clearTimeout(timerRef.current);
      return;
    }
    activeRef.current = true;
    void poll();
    return () => {
      activeRef.current = false;
      if (timerRef.current !== undefined) window.clearTimeout(timerRef.current);
    };
  }, [enabled, poll]);

  /** Run one model-setup request. On success the local state moves to
   * `downloading` and polling resumes; on a refusal or a failed request the
   * reason lands in `actionError` for the screen to show (#642), and nothing
   * is thrown. */
  const runModelAction = useCallback(
    async (path: string, optimistic: StartupStatus) => {
      setActionError(null);
      let response: Response;
      try {
        response = await apiFetch(path, { method: "POST" });
      } catch {
        if (activeRef.current) setActionError(MODEL_SETUP_CONNECTION_ERROR);
        return;
      }
      if (!response.ok) {
        const message = await modelSetupErrorMessage(response);
        if (activeRef.current) setActionError(message);
        return;
      }
      setStatus(optimistic);
      void poll();
    },
    [poll],
  );

  const acceptGemma = useCallback(
    () =>
      runModelAction("/api/model/accept-gemma", {
        state: "downloading",
        model: "EmbeddingGemma 300M Q4",
      }),
    [runModelAction],
  );

  const declineGemma = useCallback(
    () =>
      runModelAction("/api/model/decline-gemma", {
        state: "downloading",
        model: "Nomic Embed Text v1.5",
      }),
    [runModelAction],
  );

  const retryModelSetup = useCallback(
    () => runModelAction("/api/model/retry", { state: "downloading" }),
    [runModelAction],
  );

  return {
    status,
    connectionIssue,
    actionError,
    hasSteppedPastGate,
    acceptGemma,
    declineGemma,
    retryModelSetup,
  };
}

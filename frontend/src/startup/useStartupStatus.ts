import { useCallback, useEffect, useRef, useState } from "react";

import { apiFetch } from "../api/api";

export type StartupStatus =
  | { state: "terms_required" }
  | {
      state: "downloading";
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
 * backend reports `ready` or `failed`. A model-setup action
 * (accept/decline/retry) optimistically moves local state to `downloading`
 * and explicitly resumes polling, since the loop would otherwise stay parked
 * at its last terminal state forever — required for a real retry after a
 * failed model download to ever reach `ready` again.
 */
export function useStartupStatus(enabled = true) {
  const [status, setStatus] = useState<StartupStatus | null>(null);
  const [connectionIssue, setConnectionIssue] = useState(false);
  // The gate is an instance-first-run surface, not a mount-first-run surface:
  // retain the latch across a browser reload so a retry never re-blocks a
  // client that has already reached the workspace.
  const [hasSteppedPastGate, setHasSteppedPastGate] = useState(
    () => window.localStorage.getItem(HAS_STEPPED_PAST_GATE_KEY) === "1",
  );
  const hasSteppedPastGateRef = useRef(hasSteppedPastGate);
  const activeRef = useRef(true);
  const timerRef = useRef<number | undefined>(undefined);
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
      setStatus(next);
      setConnectionIssue(false);
      if (
        !hasSteppedPastGateRef.current &&
        next.state !== "terms_required" &&
        next.state !== "downloading"
      ) {
        hasSteppedPastGateRef.current = true;
        window.localStorage.setItem(HAS_STEPPED_PAST_GATE_KEY, "1");
        setHasSteppedPastGate(true);
      }
      shouldPoll = next.state !== "ready" && next.state !== "failed";
    } catch {
      if (!activeRef.current) return;
      setConnectionIssue(true);
    }

    if (activeRef.current && shouldPoll) {
      timerRef.current = window.setTimeout(
        () => void pollRef.current(),
        POLL_INTERVAL_MS,
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

  const acceptGemma = useCallback(async () => {
    const response = await apiFetch("/api/model/accept-gemma", {
      method: "POST",
    });
    if (response.ok) {
      setStatus({ state: "downloading", model: "EmbeddingGemma 300M Q4" });
      void poll();
    }
  }, [poll]);

  const declineGemma = useCallback(async () => {
    const response = await apiFetch("/api/model/decline-gemma", {
      method: "POST",
    });
    if (response.ok) {
      setStatus({ state: "downloading", model: "Nomic Embed Text v1.5" });
      void poll();
    }
  }, [poll]);

  const retryModelSetup = useCallback(async () => {
    const response = await apiFetch("/api/model/retry", { method: "POST" });
    if (response.ok) {
      setStatus({ state: "downloading" });
      void poll();
    }
  }, [poll]);

  return {
    status,
    connectionIssue,
    hasSteppedPastGate,
    acceptGemma,
    declineGemma,
    retryModelSetup,
  };
}

import { useEffect, useState, type ReactNode } from "react";

import { useTheme } from "../hooks/useTheme";

type StartupStatus =
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
const THEME_ICON = { auto: "◑", light: "○", dark: "●" } as const;

export function StartupGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<StartupStatus | null>(null);
  const [connectionIssue, setConnectionIssue] = useState(false);
  const { theme, cycleTheme } = useTheme();

  useEffect(() => {
    let active = true;
    let timer: number | undefined;

    const poll = async () => {
      let shouldPoll = true;
      try {
        const response = await fetch("/api/startup-status", {
          cache: "no-store",
        });
        if (!response.ok) {
          throw new Error(`startup status returned ${response.status}`);
        }
        const next = (await response.json()) as StartupStatus;
        if (!active) return;
        setStatus(next);
        setConnectionIssue(false);
        shouldPoll = next.state !== "ready" && next.state !== "failed";
      } catch {
        if (!active) return;
        setConnectionIssue(true);
      }

      if (active && shouldPoll) {
        timer = window.setTimeout(poll, POLL_INTERVAL_MS);
      }
    };

    void poll();
    return () => {
      active = false;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, []);

  if (status?.state === "ready") {
    return children;
  }

  const failed = status?.state === "failed";
  const indexing = status?.state === "indexing" ? status : null;
  const percent = indexing?.percent ?? 0;
  const eta = indexing?.eta_seconds;
  const progressLabel = indexing
    ? `${percent}% of embedding work complete`
    : "Measuring the vault";

  return (
    <div className="startup-shell">
      <div className="hotbar" aria-hidden="true" />
      <header className="startup-topbar">
        <div className="startup-brand" aria-label="Hatchdoor">
          <span className="startup-brand-mark" aria-hidden="true">
            [<i />]
          </span>
          <span>HATCHDOOR</span>
        </div>
        <span className="startup-crumb">Preparing vault</span>
        <button
          type="button"
          className="icon-button startup-theme"
          onClick={cycleTheme}
          aria-label={`Theme: ${theme}`}
        >
          {THEME_ICON[theme]}
        </button>
      </header>

      <main className="startup-main">
        <p className="startup-kicker">SEARCH INDEX</p>
        <h1>{failed ? "Vault unavailable" : "Preparing your vault"}</h1>
        <p className="startup-lede" aria-live="polite">
          {failed
            ? (status.message ?? "Indexing could not be completed.")
            : connectionIssue
              ? "Waiting for Hatchdoor to respond…"
              : indexing
                ? "Building the search index. Your notes stay locked until it is ready."
                : "Scanning notes and measuring the work ahead…"}
        </p>

        {!failed ? (
          <>
            <div
              className={`startup-progress ${indexing ? "" : "is-indeterminate"}`}
              role="progressbar"
              aria-label={progressLabel}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={indexing ? percent : undefined}
            >
              <span style={{ transform: `scaleX(${percent / 100})` }} />
            </div>

            <div className="startup-progress-line" aria-live="polite">
              <strong>{indexing ? `${percent}%` : "Scanning"}</strong>
              <span>{formatEta(eta)}</span>
            </div>

            {indexing ? (
              <dl className="startup-measures">
                <div>
                  <dt>Notes</dt>
                  <dd>
                    {formatCount(indexing.notes_completed)} of{" "}
                    {formatCount(indexing.notes_total)} notes
                  </dd>
                </div>
                <div>
                  <dt>Chunks</dt>
                  <dd>
                    {formatCount(indexing.chunks_completed)} of{" "}
                    {formatCount(indexing.chunks_total)} chunks
                  </dd>
                </div>
              </dl>
            ) : null}
          </>
        ) : (
          <p className="startup-failure-help">
            Check the Hatchdoor logs, correct the indexing error, then restart
            the service.
          </p>
        )}
      </main>
    </div>
  );
}

function formatCount(value: number) {
  return value.toLocaleString();
}

function formatEta(seconds?: number) {
  if (seconds === undefined) return "Estimating time remaining";
  if (seconds < 45) return "Less than a minute remaining";
  const minutes = Math.max(1, Math.round(seconds / 60));
  return `About ${minutes} min remaining`;
}

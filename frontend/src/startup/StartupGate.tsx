import { useEffect, useState, type ReactNode } from "react";

import { useTheme } from "../hooks/useTheme";
import { apiFetch } from "../api/api";

type StartupStatus =
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

  const acceptGemma = async () => {
    const response = await apiFetch("/api/model/accept-gemma", { method: "POST" });
    if (response.ok) {
      setStatus({ state: "downloading", model: "EmbeddingGemma 300M Q4" });
    }
  };

  const declineGemma = async () => {
    const response = await apiFetch("/api/model/decline-gemma", { method: "POST" });
    if (response.ok) {
      setStatus({ state: "downloading", model: "Nomic Embed Text v1.5" });
    }
  };

  const retryModelSetup = async () => {
    const response = await apiFetch("/api/model/retry", { method: "POST" });
    if (response.ok) {
      setStatus({ state: "downloading" });
    }
  };

  if (status?.state === "ready") {
    return children;
  }

  const failed = status?.state === "failed";
  const indexing = status?.state === "indexing" ? status : null;
  const downloading = status?.state === "downloading" ? status : null;
  const termsRequired = status?.state === "terms_required";
  const percent = indexing?.percent ?? downloading?.percent ?? 0;
  const eta = indexing?.eta_seconds;
  const progressLabel = indexing
    ? `${percent}% of embedding work complete`
    : downloading?.percent !== undefined
      ? `${percent}% of model download complete`
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
        <span className="startup-crumb">
          {termsRequired ? "Model setup" : "Preparing vault"}
        </span>
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
        <p className="startup-kicker">{termsRequired ? "SEARCH MODEL" : "SEARCH INDEX"}</p>
        <h1>
          {termsRequired
            ? "Set up multilingual search"
            : failed
              ? "Vault unavailable"
              : "Preparing your vault"}
        </h1>
        {termsRequired ? (
          <section className="startup-terms" aria-label="Gemma terms">
            <p>
              Hatchdoor can download EmbeddingGemma, a multilingual search
              model licensed under Google’s Gemma Terms and Prohibited Use
              Policy.
            </p>
            <p>
              Accepting these terms only allows Hatchdoor to download and use
              the Gemma model. It does not change ownership of your vault or
              its contents. Hatchdoor does not send your notes to Google when
              indexing or searching.
            </p>
            <p>
              <a href="https://ai.google.dev/gemma/terms" target="_blank" rel="noreferrer">
                Read Gemma Terms
              </a>{" "}
              <a
                href="https://ai.google.dev/gemma/prohibited_use_policy"
                target="_blank"
                rel="noreferrer"
              >
                Read Prohibited Use Policy
              </a>
            </p>
            <div className="startup-actions">
              <button type="button" className="startup-primary" onClick={() => void acceptGemma()}>
                Accept terms and set up Gemma
              </button>
              <button type="button" className="startup-secondary" onClick={() => void declineGemma()}>
                Use Nomic instead
              </button>
            </div>
            <p className="startup-fallback-note">
              Nomic is the no-extra-terms fallback. It is English-only and not
              as good as Gemma for multilingual search.
            </p>
          </section>
        ) : (
          <p className="startup-lede" aria-live="polite">
            {failed
            ? (status.message ?? "Indexing could not be completed.")
            : connectionIssue
              ? "Waiting for Hatchdoor to respond…"
              : downloading
                ? `Downloading ${downloading.model ?? "the search model"}. Your vault stays locked until setup is complete.`
              : indexing
                ? "Building the search index. Your notes stay locked until it is ready."
                : "Scanning notes and measuring the work ahead…"}
          </p>
        )}

        {!failed && !termsRequired ? (
          <>
            <div
              className={`startup-progress ${indexing || downloading?.percent !== undefined ? "" : "is-indeterminate"}`}
              role="progressbar"
              aria-label={progressLabel}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={indexing || downloading?.percent !== undefined ? percent : undefined}
            >
              <span style={{ transform: `scaleX(${percent / 100})` }} />
            </div>

            <div className="startup-progress-line" aria-live="polite">
              <strong>{indexing || downloading?.percent !== undefined ? `${percent}%` : downloading ? "Downloading" : "Scanning"}</strong>
              <span>{indexing ? formatEta(eta) : downloading ? formatDownload(downloading.downloaded_bytes, downloading.total_bytes) : formatEta(eta)}</span>
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
        ) : failed ? (
          <p className="startup-failure-help">
            Check the Hatchdoor logs, then retry setup.
            <br />
            <button
              type="button"
              className="startup-secondary"
              onClick={() => void retryModelSetup()}
            >
              Retry setup
            </button>
          </p>
        ) : null}
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

function formatDownload(downloaded?: number, total?: number) {
  if (downloaded === undefined || total === undefined) return "Connecting to model source";
  return `${formatBytes(downloaded)} of ${formatBytes(total)}`;
}

function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

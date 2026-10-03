import type { ReactNode } from "react";

import { CONTEXTUAL_HELP, ContextualHelpLink } from "../features/help";
import { useTheme } from "../hooks/useTheme";
import type { StartupStatus } from "./useStartupStatus";

const THEME_ICON = { auto: "◑", light: "○", dark: "●" } as const;

/**
 * The full-screen model-setup gate (#150). It shrinks to exactly the model
 * step: it renders only for `terms_required`/`downloading`. A `downloading`
 * state gates only until `hasSteppedPastGate` first flips — after that a
 * retry's re-download shows in the Scope zone slot and the search dialog
 * instead (#339). `terms_required` gates again even after the latch, since
 * nothing else in the app can offer the model choice. A demo instance never
 * gates on `terms_required`, latched or not: its server 404s both accept and
 * decline, so the visitor gets the workspace and the search dialog's
 * visitor-grade "unavailable" sentence instead of dead operator controls.
 *
 * The decision waits for its inputs (#339): until discovery has resolved and
 * the first startup answer has landed, nothing mounts, so the workspace is
 * never mounted only to be torn down a fetch later when that answer gates.
 * A missing answer still never locks anyone out: a failed poll
 * (`connectionIssue`) releases the hold, and a zero-Vault or broken-registry
 * workspace, which never polls, is never held.
 */
export function StartupGate({
  status,
  connectionIssue,
  hasSteppedPastGate,
  discoveryLoading,
  hasRegistryRecovery,
  hasNoVaults,
  demoMode = false,
  onAcceptGemma,
  onDeclineGemma,
  children,
}: {
  status: StartupStatus | null;
  connectionIssue: boolean;
  hasSteppedPastGate: boolean;
  discoveryLoading: boolean;
  hasRegistryRecovery: boolean;
  hasNoVaults: boolean;
  demoMode?: boolean;
  onAcceptGemma: () => void;
  onDeclineGemma: () => void;
  children: ReactNode;
}) {
  const { theme, cycleTheme } = useTheme();

  const statusExpected = !hasRegistryRecovery && !hasNoVaults;
  const deciding =
    discoveryLoading || (statusExpected && status === null && !connectionIssue);
  if (deciding) {
    return <div className="startup-shell" aria-busy="true" />;
  }

  const shouldGate =
    statusExpected &&
    ((status?.state === "terms_required" && !demoMode) ||
      (status?.state === "downloading" && !hasSteppedPastGate));
  if (!shouldGate) {
    return children;
  }

  const termsRequired = status?.state === "terms_required";
  const downloading = status?.state === "downloading" ? status : null;
  const percent = downloading?.percent ?? 0;
  const progressLabel =
    downloading?.percent !== undefined
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
        <p className="startup-kicker">
          {termsRequired ? "SEARCH MODEL" : "SEARCH INDEX"}
        </p>
        <h1>
          {termsRequired
            ? "Set up multilingual search"
            : "Preparing your vault"}
        </h1>
        {termsRequired ? (
          <section className="startup-terms" aria-label="Gemma terms">
            <p>
              Hatchdoor can download EmbeddingGemma, a multilingual search model
              licensed under Google’s Gemma Terms and Prohibited Use Policy.
            </p>
            <p>
              Accepting these terms only allows Hatchdoor to download and use
              the Gemma model. It does not change ownership of your vault or its
              contents. Hatchdoor does not send your notes to Google when
              indexing or searching.
            </p>
            <p>
              <a
                href="https://ai.google.dev/gemma/terms"
                target="_blank"
                rel="noreferrer"
              >
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
              <button
                type="button"
                className="startup-primary"
                onClick={onAcceptGemma}
              >
                Accept terms and set up Gemma
              </button>
              <button
                type="button"
                className="startup-secondary"
                onClick={onDeclineGemma}
              >
                Use Nomic instead
              </button>
            </div>
            <p className="startup-fallback-note">
              Nomic is the fallback if you decline Gemma. It supports English
              only. It still provides solid search, but Gemma performed better
              in our tests, including English searches. Nomic uses about 1.3 GB
              of RAM while indexing; Gemma uses about 0.5 GB.
            </p>
            <p className="startup-fallback-note">
              <ContextualHelpLink to={CONTEXTUAL_HELP.modelChoice} />
            </p>
          </section>
        ) : (
          <>
            <p className="startup-lede" aria-live="polite">
              {connectionIssue
                ? "Waiting for Hatchdoor to respond…"
                : downloading
                  ? `Downloading ${downloading.model ?? "the search model"}. Your vault stays locked until setup is complete.`
                  : "Scanning notes and measuring the work ahead…"}
            </p>

            <div
              className={`startup-progress ${downloading?.percent !== undefined ? "" : "is-indeterminate"}`}
              role="progressbar"
              aria-label={progressLabel}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={
                downloading?.percent !== undefined ? percent : undefined
              }
            >
              <span style={{ transform: `scaleX(${percent / 100})` }} />
            </div>

            <div className="startup-progress-line" aria-live="polite">
              <strong>
                {downloading?.percent !== undefined
                  ? `${percent}%`
                  : downloading
                    ? "Downloading"
                    : "Scanning"}
              </strong>
              <span>
                {formatDownload(
                  downloading?.downloaded_bytes,
                  downloading?.total_bytes,
                )}
              </span>
            </div>
          </>
        )}
      </main>
    </div>
  );
}

function formatDownload(downloaded?: number, total?: number) {
  if (downloaded === undefined || total === undefined)
    return "Connecting to model source";
  return `${formatBytes(downloaded)} of ${formatBytes(total)}`;
}

function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

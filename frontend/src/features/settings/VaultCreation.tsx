import { useRef, useState } from "react";

import type { GitBehavior } from "./vaultGitBehavior";
import {
  behaviorOptions,
  buildSourceForBehavior,
  clampPollMinutes,
  DEFAULT_POLL_MINUTES,
  fetchRegistryRevision,
  isRemoteBacked,
  MAX_POLL_MINUTES,
  MIN_POLL_MINUTES,
  withIdentityFields,
} from "./vaultGitBehavior";
import type { CreateVaultKind } from "./vaultCreation";
import {
  baseSourceForKind,
  createVault,
  validateCreateSource,
} from "./vaultCreation";
import type { VaultSummary } from "../../types";

const KIND_LABEL: Record<CreateVaultKind, string> = {
  own: "A folder on this server",
  managed: "A managed Git checkout",
};

function defaultBehaviorFor(kind: CreateVaultKind): GitBehavior {
  return kind === "managed" ? "pull_only" : "no_git";
}

/** The one creation flow every `Add a Vault` entry point opens (issue #153):
 * the settings index (#148) and the zero-Vault workspace state (#150) both
 * render their own button, but neither owned wiring it to a real flow — this
 * is that flow, mounted once per open by its callers rather than kept alive
 * hidden, so its form state always starts fresh. */
export function VaultCreationDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (vault: VaultSummary) => void;
}) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<CreateVaultKind>("own");
  const [pathDraft, setPathDraft] = useState("");
  const [behavior, setBehavior] = useState<GitBehavior>(
    defaultBehaviorFor("own"),
  );
  const [repoUrlDraft, setRepoUrlDraft] = useState("");
  const [branchDraft, setBranchDraft] = useState("");
  const [subdirDraft, setSubdirDraft] = useState("");
  const [pollMinutesDraft, setPollMinutesDraft] = useState(
    String(DEFAULT_POLL_MINUTES),
  );
  const [signIn, setSignIn] = useState<"none" | "token">("none");
  const [credToken, setCredToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  // A ref rather than the `submitting` state alone: two click events fired
  // before React commits the state update from the first (a double-click or
  // double-tap) must not both pass this guard, and only a synchronous read
  // is safe against that race.
  const submittingRef = useRef(false);

  const baseSource = baseSourceForKind(kind, pathDraft);
  const behaviorOpts = behaviorOptions(baseSource);
  const remoteBacked = isRemoteBacked(behavior);
  const finalSource = withIdentityFields(
    buildSourceForBehavior(baseSource, behavior),
    {
      repositoryUrl: repoUrlDraft,
      branch: branchDraft,
      subdirectory: subdirDraft,
      pollMinutes: clampPollMinutes(pollMinutesDraft),
    },
  );

  // The remote fields (repository URL, branch, subdirectory, schedule,
  // sign-in) only mean anything once the chosen behaviour actually talks to
  // a remote — clearing them whenever a switch leaves that state keeps a
  // value typed for an earlier, abandoned choice from lingering invisibly
  // and reappearing pre-filled (or, for sign-in, being sent against a source
  // the server would refuse to accept it on at all) if the user switches
  // back later.
  const resetRemoteFields = () => {
    setRepoUrlDraft("");
    setBranchDraft("");
    setSubdirDraft("");
    setPollMinutesDraft(String(DEFAULT_POLL_MINUTES));
    setSignIn("none");
    setCredToken("");
  };

  const selectKind = (next: CreateVaultKind) => {
    setKind(next);
    const nextBehavior = defaultBehaviorFor(next);
    setBehavior(nextBehavior);
    if (!isRemoteBacked(nextBehavior)) resetRemoteFields();
  };

  const selectBehavior = (next: GitBehavior) => {
    setBehavior(next);
    if (!isRemoteBacked(next)) resetRemoteFields();
  };

  const handleSubmit = async () => {
    if (submittingRef.current) return;
    setError(null);
    const trimmedName = name.trim();
    if (!trimmedName) {
      setError("Enter a name for this Vault.");
      return;
    }
    const sourceError = validateCreateSource(finalSource);
    if (sourceError) {
      setError(sourceError);
      return;
    }
    if (remoteBacked && signIn === "token" && !credToken.trim()) {
      setError("Enter an access token, or choose No sign-in.");
      return;
    }
    submittingRef.current = true;
    setSubmitting(true);
    const revision = await fetchRegistryRevision();
    if (revision === null) {
      setError("Could not reach the server. Check the connection and try again.");
      submittingRef.current = false;
      setSubmitting(false);
      return;
    }
    const credentials =
      remoteBacked && signIn === "token"
        ? { token: credToken.trim() }
        : undefined;
    const result = await createVault({
      expectedRegistryRevision: revision,
      name: trimmedName,
      source: finalSource,
      credentials,
    });
    submittingRef.current = false;
    setSubmitting(false);
    if (!result.ok) {
      setError(
        result.code === "registry_revision_conflict"
          ? "This changed elsewhere just now. Try again."
          : (result.message ?? "This Vault could not be created."),
      );
      return;
    }
    onCreated(result.vault);
  };

  return (
    <div className="settings-modal-back">
      <div
        className="settings-modal settings-modal-form"
        role="dialog"
        aria-modal="true"
        aria-label="Add a Vault"
      >
        <h3>Add a Vault</h3>

        <label className="settings-row">
          <span>
            <span className="settings-row-label">Name</span>
          </span>
          <input
            className="settings-input"
            aria-label="Vault name"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </label>

        <div className="settings-row">
          <span>
            <span className="settings-row-label">Where is this Vault?</span>
          </span>
          <div className="settings-segmented" role="group" aria-label="Where is this Vault?">
            {(Object.keys(KIND_LABEL) as CreateVaultKind[]).map((item) => (
              <button
                key={item}
                type="button"
                aria-pressed={kind === item}
                onClick={() => selectKind(item)}
              >
                {KIND_LABEL[item]}
              </button>
            ))}
          </div>
        </div>

        {kind === "own" ? (
          <label className="settings-row">
            <span>
              <span className="settings-row-label">Folder path</span>
              <span className="settings-row-help">
                A folder already on this server.
              </span>
            </span>
            <input
              className="settings-input"
              aria-label="Folder path"
              value={pathDraft}
              onChange={(event) => setPathDraft(event.target.value)}
            />
          </label>
        ) : null}

        <div className="settings-row">
          <span>
            <span className="settings-row-label">Git behaviour</span>
          </span>
          <div className="settings-segmented" role="group" aria-label="Git behaviour">
            {behaviorOpts.map((item) => (
              <button
                key={item.id}
                type="button"
                aria-pressed={behavior === item.id}
                onClick={() => selectBehavior(item.id)}
              >
                {item.label}
              </button>
            ))}
          </div>
        </div>

        {remoteBacked ? (
          <>
            <label className="settings-row">
              <span>
                <span className="settings-row-label">Repository URL</span>
              </span>
              <input
                className="settings-input"
                aria-label="Repository URL"
                value={repoUrlDraft}
                onChange={(event) => setRepoUrlDraft(event.target.value)}
              />
            </label>
            <label className="settings-row">
              <span>
                <span className="settings-row-label">Branch (optional)</span>
              </span>
              <input
                className="settings-input"
                aria-label="Branch"
                value={branchDraft}
                onChange={(event) => setBranchDraft(event.target.value)}
              />
            </label>
            <label className="settings-row">
              <span>
                <span className="settings-row-label">
                  Folder within the repository (optional)
                </span>
              </span>
              <input
                className="settings-input"
                aria-label="Folder within the repository"
                value={subdirDraft}
                onChange={(event) => setSubdirDraft(event.target.value)}
              />
            </label>
            <div className="settings-row">
              <span>
                <span className="settings-row-label">Sign-in</span>
              </span>
              <div>
                <div
                  className="settings-segmented"
                  role="group"
                  aria-label="Sign-in"
                >
                  <button
                    type="button"
                    aria-pressed={signIn === "none"}
                    onClick={() => {
                      setSignIn("none");
                      setCredToken("");
                    }}
                  >
                    No sign-in
                  </button>
                  <button
                    type="button"
                    aria-pressed={signIn === "token"}
                    onClick={() => setSignIn("token")}
                  >
                    Access token
                  </button>
                </div>
                {signIn === "token" ? (
                  <input
                    className="settings-input"
                    type="password"
                    aria-label="Repository access token"
                    value={credToken}
                    onChange={(event) => setCredToken(event.target.value)}
                  />
                ) : null}
              </div>
            </div>
            <div className="settings-row">
              <span>
                <span className="settings-row-label">Sync schedule</span>
                <span className="settings-row-help">
                  Hatchdoor has no way to be told when something is pushed, so
                  it checks on this schedule.
                </span>
              </span>
              <div className="settings-inline">
                <input
                  className="settings-input settings-input-short"
                  type="number"
                  min={MIN_POLL_MINUTES}
                  max={MAX_POLL_MINUTES}
                  aria-label="Sync schedule in minutes"
                  value={pollMinutesDraft}
                  onChange={(event) => setPollMinutesDraft(event.target.value)}
                />
                <span className="settings-unit">minutes</span>
              </div>
            </div>
          </>
        ) : null}

        {error ? (
          <div className="settings-notice settings-notice-err" role="alert">
            {error}
          </div>
        ) : null}

        <div className="settings-modal-actions">
          <button
            type="button"
            className="settings-btn"
            onClick={onClose}
            disabled={submitting}
          >
            Cancel
          </button>
          <button
            type="button"
            className="settings-btn settings-btn-hot"
            onClick={() => void handleSubmit()}
            disabled={submitting}
          >
            {submitting ? "Creating…" : "Create Vault"}
          </button>
        </div>
      </div>
    </div>
  );
}

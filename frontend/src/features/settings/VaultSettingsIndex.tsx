import { Fragment, useCallback, useEffect, useState } from "react";

import { apiFetch } from "../../api/api";
import { VaultSlot } from "../../app/vaultSlot";
import { deriveVaultSlot } from "../../app/vaultSlotLogic";
import { StateBlock } from "../../components/ui";
import type {
  VaultDiscoveryResponse,
  VaultId,
  VaultRegistryRecovery,
  VaultSource,
  VaultSummary,
} from "../../types";
import { VaultCreationDialog } from "./VaultCreation";
import {
  behaviorOf,
  behaviorOptions,
  buildSourceForBehavior,
  clampPollMinutes,
  clearRecoveryPending,
  DEFAULT_POLL_MINUTES,
  describeGitFailure,
  type GitBehavior,
  isRecoveryPending,
  isRemoteBacked,
  markRecoveryPending,
  MAX_POLL_MINUTES,
  MIN_POLL_MINUTES,
  missingRequiredRepositoryUrl,
  recoverPausedVault,
  REPOSITORY_URL_REQUIRED_MESSAGE,
  requestJson,
  sameSourceIdentity,
  sourceLabel,
  withIdentityFields,
} from "./vaultGitBehavior";

type Counts = Record<VaultId, number>;

function conditionSentence(
  vault: VaultSummary,
  count: number | undefined,
): string {
  const slot = deriveVaultSlot(vault, count);
  return slot.kind === "condition"
    ? slot.sentence
    : "This Vault is ready to use.";
}

function lastChanged(mtimeNs: number | undefined): string {
  if (!mtimeNs) return "no indexed changes yet";
  const date = new Date(mtimeNs / 1_000_000);
  return Number.isNaN(date.valueOf())
    ? "last change unavailable"
    : `changed ${date.toLocaleDateString()}`;
}

/** The settings index includes disabled Vaults; workspace discovery does not. */
export function VaultSettingsIndex({
  selectedVaultId,
  onSelectVault,
  autoOpenCreation,
  onVaultCreated,
}: {
  selectedVaultId: VaultId | null;
  onSelectVault: (vaultId: VaultId) => void;
  /** Set by `SettingsPage` when navigation carried an "open the creation
   * flow immediately" request — the zero-Vault workspace state (#150)'s
   * `Add a Vault` button lands here rather than rendering its own copy of
   * the flow. */
  autoOpenCreation?: boolean;
  /** Called after a successful create, in addition to this component's own
   * list refresh, so the app-wide Vault discovery that drives the sidebar
   * and scope zone also picks up the new Vault without a reload. */
  onVaultCreated?: () => void;
}) {
  const [vaults, setVaults] = useState<VaultSummary[]>([]);
  const [counts, setCounts] = useState<Counts>({});
  const [recovering, setRecovering] = useState<Record<VaultId, boolean>>({});
  const [demoMode, setDemoMode] = useState(false);
  const [creationOpen, setCreationOpen] = useState(Boolean(autoOpenCreation));
  // The persisted registry file itself is unreadable (#150) — distinct from
  // a Vault-level `needs attention` recovery above, this replaces the whole
  // group. `legacy_migration_recovery` is deliberately not surfaced here:
  // the registry loads fine (empty) in that case, so the group renders its
  // ordinary zero-Vault "Add a Vault" state.
  const [registryRecovery, setRegistryRecovery] =
    useState<VaultRegistryRecovery | null>(null);

  const loadVaults = useCallback(async (signal?: { cancelled: boolean }) => {
    const response = await apiFetch("/api/v1/vaults");
    if (!response.ok || signal?.cancelled) return;
    const discovery = (await response.json()) as VaultDiscoveryResponse;
    if (!Array.isArray(discovery.vaults)) return;
    setVaults(discovery.vaults);
    setDemoMode(discovery.demo_mode);
    setRegistryRecovery(discovery.recovery ?? null);
    if (discovery.recovery) return;
    const stats = await apiFetch("/api/v1/vaults/all/stats");
    if (!stats.ok || signal?.cancelled) return;
    const payload = (await stats.json()) as {
      data?: Array<{ vault_id: VaultId; note_count: number }>;
    };
    if (!signal?.cancelled)
      setCounts(
        Object.fromEntries(
          (payload.data ?? []).map(({ vault_id, note_count }) => [
            vault_id,
            note_count,
          ]),
        ),
      );
  }, []);

  useEffect(() => {
    const signal = { cancelled: false };
    void loadVaults(signal).catch(() => undefined);
    return () => {
      signal.cancelled = true;
    };
  }, [loadVaults]);

  const handleRecover = async (vaultId: VaultId) => {
    setRecovering((old) => ({ ...old, [vaultId]: true }));
    const result = await recoverPausedVault(vaultId);
    if (result.ok)
      setVaults((old) =>
        old.map((item) =>
          item.vault_id === vaultId
            ? (result.vault ?? { ...item, enabled: true })
            : item,
        ),
      );
    setRecovering((old) => ({ ...old, [vaultId]: false }));
  };

  if (registryRecovery) {
    return (
      <section className="settings-vault-index" aria-label="Vaults">
        <p className="settings-index-group">Vaults</p>
        <StateBlock
          tone="error"
          title="Vault Registry Unavailable"
          description={`${registryRecovery.message} Nothing was changed, and your Markdown is untouched.`}
          actionLabel="Try again"
          onAction={() => void loadVaults()}
        />
      </section>
    );
  }

  return (
    <section className="settings-vault-index" aria-label="Vaults">
      <p className="settings-index-group">Vaults</p>
      {vaults.map((vault) => {
        const needsRecovery =
          !vault.enabled && isRecoveryPending(vault.vault_id);
        return (
          <Fragment key={vault.vault_id}>
            <button
              className="settings-index-item settings-vault-index-item"
              data-active={vault.vault_id === selectedVaultId}
              data-paused={!vault.enabled}
              data-recovery={needsRecovery}
              onClick={() => onSelectVault(vault.vault_id)}
              type="button"
            >
              <span className="settings-index-title">{vault.name}</span>
              {needsRecovery ? (
                <span className="settings-vault-paused settings-vault-needs-attention">
                  needs attention
                </span>
              ) : vault.enabled ? (
                <VaultSlot vault={vault} noteCount={counts[vault.vault_id]} />
              ) : (
                <span className="settings-vault-paused">paused</span>
              )}
            </button>
            {needsRecovery ? (
              <div className="settings-recovery-line" role="alert">
                <span>This Vault changed but did not start back up.</span>
                <button
                  type="button"
                  className="settings-mini settings-btn-danger"
                  disabled={recovering[vault.vault_id]}
                  onClick={() => void handleRecover(vault.vault_id)}
                >
                  Try again
                </button>
              </div>
            ) : null}
          </Fragment>
        );
      })}
      {demoMode ? null : (
        <button
          className="settings-link"
          type="button"
          onClick={() => setCreationOpen(true)}
        >
          Add a Vault
        </button>
      )}
      {creationOpen && !demoMode ? (
        <VaultCreationDialog
          onClose={() => setCreationOpen(false)}
          onCreated={(vault) => {
            setCreationOpen(false);
            setVaults((old) => [...old, vault]);
            onVaultCreated?.();
            onSelectVault(vault.vault_id);
          }}
        />
      ) : null}
    </section>
  );
}

function draftsFromSource(source: VaultSource | undefined) {
  const behavior = source ? behaviorOf(source) : null;
  const repoUrl =
    source && source.type !== "local" ? (source.repository_url ?? "") : "";
  const branch = source && source.type !== "local" ? (source.branch ?? "") : "";
  const subdirectory =
    source && source.type !== "local" ? (source.vault_subdirectory ?? "") : "";
  const pollMinutes =
    source && source.type !== "local"
      ? String(
          Math.max(
            MIN_POLL_MINUTES,
            Math.round(source.poll_interval_secs / 60),
          ),
        )
      : String(DEFAULT_POLL_MINUTES);
  return { behavior, repoUrl, branch, subdirectory, pollMinutes };
}

const IDENTITY_CHANGE_CONSEQUENCE =
  "This runs as one step: the Vault pauses, the change saves, and the Vault starts back up. It stays out of the sidebar and All Vaults for that moment.";

const LOCAL_HISTORY_CONSEQUENCE =
  "Local history creates a hidden .git folder inside this Vault's notes folder to hold its history. That folder grows permanently: every image and PDF attached stays in it, even after you delete the file from the Vault.";

export function VaultSettingsDetail({
  vaultId,
  serverIdentity,
  onDisconnect,
}: {
  vaultId: VaultId;
  serverIdentity: { name: string; email: string };
  onDisconnect: () => void;
}) {
  const [vault, setVault] = useState<VaultSummary | null>(null);
  const [revision, setRevision] = useState<number | null>(null);
  const [count, setCount] = useState<number>();
  const [changed, setChanged] = useState<number>();
  const [name, setName] = useState("");
  const [exclude, setExclude] = useState("");
  const [archive, setArchive] = useState("");
  const [identityName, setIdentityName] = useState("");
  const [identityEmail, setIdentityEmail] = useState("");
  const [draftBehavior, setDraftBehavior] = useState<GitBehavior | null>(null);
  const [repoUrlDraft, setRepoUrlDraft] = useState("");
  const [branchDraft, setBranchDraft] = useState("");
  const [subdirDraft, setSubdirDraft] = useState("");
  const [pollMinutesDraft, setPollMinutesDraft] = useState(
    String(DEFAULT_POLL_MINUTES),
  );
  const [plaqueEditing, setPlaqueEditing] = useState(false);
  const [signIn, setSignIn] = useState<"none" | "token">("none");
  const [credToken, setCredToken] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [confirmation, setConfirmation] = useState<{
    newSource: VaultSource;
    localHistory: boolean;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [recoveryPending, setRecoveryPending] = useState(false);

  const applyVault = (next: VaultSummary) => {
    setVault(next);
    setName(next.name);
    setExclude(next.exclude_patterns.join(", "));
    setArchive(next.archive_folder ?? "");
    setIdentityName(next.commit_identity?.name ?? "");
    setIdentityEmail(next.commit_identity?.email ?? "");
    const drafts = draftsFromSource(next.source);
    setDraftBehavior(drafts.behavior);
    setRepoUrlDraft(drafts.repoUrl);
    setBranchDraft(drafts.branch);
    setSubdirDraft(drafts.subdirectory);
    setPollMinutesDraft(drafts.pollMinutes);
    setSignIn(next.credential_configured ? "token" : "none");
    setCredToken("");
    setPlaqueEditing(false);
    if (next.enabled && isRecoveryPending(next.vault_id)) {
      clearRecoveryPending(next.vault_id);
    }
    setRecoveryPending(!next.enabled && isRecoveryPending(next.vault_id));
  };

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const discoveryResponse = await apiFetch("/api/v1/vaults");
      if (!discoveryResponse.ok) return;
      const discovery =
        (await discoveryResponse.json()) as VaultDiscoveryResponse;
      const next = discovery.vaults?.find((item) => item.vault_id === vaultId);
      if (!next || cancelled) return;
      applyVault(next);
      setRevision(discovery.registry_revision ?? null);
      const [statsResponse, recentResponse] = await Promise.all([
        apiFetch("/api/v1/vaults/all/stats"),
        apiFetch(`/api/v1/vaults/${vaultId}/recent?limit=1`),
      ]);
      if (cancelled) return;
      if (statsResponse.ok) {
        const stats = (await statsResponse.json()) as {
          data?: Array<{ vault_id: VaultId; note_count: number }>;
        };
        setCount(
          stats.data?.find((item) => item.vault_id === vaultId)?.note_count,
        );
      }
      if (recentResponse.ok) {
        const recent = (await recentResponse.json()) as {
          data?: Array<{ mtime_ns: number }>;
        };
        setChanged(recent.data?.[0]?.mtime_ns);
      }
    })().catch(() => setMessage("This Vault could not be loaded."));
    return () => {
      cancelled = true;
    };
  }, [vaultId]);

  if (!vault)
    return (
      <div className="settings-main">
        <p className="settings-muted">Loading Vault…</p>
      </div>
    );

  const paused = !vault.enabled;
  const identity =
    identityName || identityEmail
      ? { name: identityName, email: identityEmail }
      : null;

  const draftSource: VaultSource | undefined =
    vault.source && draftBehavior
      ? withIdentityFields(
          buildSourceForBehavior(vault.source, draftBehavior),
          {
            repositoryUrl: repoUrlDraft,
            branch: branchDraft,
            subdirectory: subdirDraft,
            pollMinutes: clampPollMinutes(pollMinutesDraft),
          },
        )
      : vault.source;

  const identityChanged =
    vault.source && draftSource
      ? !sameSourceIdentity(vault.source, draftSource)
      : false;

  const remoteBackedDraft = isRemoteBacked(draftBehavior);
  const showPlaqueFields = draftBehavior !== null && draftBehavior !== "no_git";
  const plaqueFieldsEditable = vault.source?.type === "local" || plaqueEditing;

  const credentialsPatch = ():
    | { action: "keep" }
    | { action: "remove" }
    | { action: "replace"; token: string } => {
    if (signIn === "none") return { action: "remove" };
    if (credToken.trim()) return { action: "replace", token: credToken.trim() };
    return { action: "keep" };
  };

  /** The `PATCH` body shared by a plain field save and the identity round
   * trip's edit step — they differ only in which revision they carry, which
   * source they send, and whether the server needs to be told this is an
   * identity change it must otherwise refuse. */
  const editVaultBody = (
    source: VaultSource | undefined,
    expectedRevision: number,
    confirmIdentityChange: boolean,
  ) => ({
    expected_registry_revision: expectedRevision,
    name,
    source: source ?? vault.source,
    exclude_patterns: exclude
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean),
    https_credentials: credentialsPatch(),
    ...(confirmIdentityChange ? { confirm_identity_change: true } : {}),
    archive_folder: archive || null,
    commit_identity: identity,
  });

  const mutate = async (path: string, init: RequestInit) => {
    setMessage(null);
    const { ok, payload: raw } = await requestJson(path, init);
    const payload = raw as {
      vault?: VaultSummary;
      registry_revision?: number;
      message?: string;
    };
    if (!ok) {
      setMessage(payload.message ?? "This Vault could not be changed.");
      return false;
    }
    if (payload.vault) applyVault(payload.vault);
    if (payload.registry_revision !== undefined)
      setRevision(payload.registry_revision);
    setMessage("Saved.");
    return true;
  };

  const plainSave = async (source: VaultSource | undefined) => {
    if (revision === null) return;
    await mutate(`/api/v1/vaults/${vaultId}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(editVaultBody(source, revision, false)),
    });
  };

  /** Issue #121's round trip: accepting an identity change runs pause, edit
   * and un-pause as one client-orchestrated act, so a settings change never
   * makes a Vault silently disappear from the sidebar. Three distinct
   * failure points, three distinct outcomes: a failed pause has nothing to
   * roll back; a failed edit is rolled back by re-enabling and reporting
   * nothing changed; a failed final un-pause is the one condition allowed to
   * persist across visits. */
  const runIdentityChange = async (newSource: VaultSource) => {
    if (revision === null) return;
    setConfirmation(null);
    setBusy(true);
    setMessage(null);

    const disableResult = await requestJson(
      `/api/v1/vaults/${vaultId}/disable?expected_registry_revision=${revision}`,
      { method: "POST" },
    );
    const disablePayload = disableResult.payload as {
      registry_revision?: number;
      message?: string;
    };
    if (!disableResult.ok || disablePayload.registry_revision === undefined) {
      setMessage(
        disablePayload.message
          ? `Nothing changed. ${disablePayload.message}`
          : "Nothing changed — this Vault could not be paused for the edit.",
      );
      setBusy(false);
      return;
    }
    const pausedRevision = disablePayload.registry_revision;
    setRevision(pausedRevision);
    setVault((current) => (current ? { ...current, enabled: false } : current));

    const editResult = await requestJson(`/api/v1/vaults/${vaultId}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(editVaultBody(newSource, pausedRevision, true)),
    });
    const editPayload = editResult.payload as {
      vault?: VaultSummary;
      registry_revision?: number;
      message?: string;
    };
    if (!editResult.ok || editPayload.registry_revision === undefined) {
      const rollback = await recoverPausedVault(vaultId);
      if (rollback.ok) {
        if (rollback.vault) applyVault(rollback.vault);
        else
          setVault((current) =>
            current ? { ...current, enabled: true } : current,
          );
        setMessage(
          editPayload.message
            ? `Nothing changed. ${editPayload.message}`
            : "Nothing changed.",
        );
      } else {
        markRecoveryPending(vaultId);
        setRecoveryPending(true);
        setMessage(
          "This Vault is paused and could not be restored automatically. Use the button below to bring it back.",
        );
      }
      setBusy(false);
      return;
    }
    const editedRevision = editPayload.registry_revision;
    setRevision(editedRevision);
    if (editPayload.vault) applyVault(editPayload.vault);

    // Set before the final call: if it fails, the marker is what makes the
    // red-line recovery state survive a reload (issue #121).
    markRecoveryPending(vaultId);
    const enableResult = await requestJson(
      `/api/v1/vaults/${vaultId}/enable?expected_registry_revision=${editedRevision}`,
      { method: "POST" },
    );
    const enablePayload = enableResult.payload as {
      vault?: VaultSummary;
      registry_revision?: number;
    };
    if (!enableResult.ok) {
      setRecoveryPending(true);
      setMessage(
        "This Vault changed but Hatchdoor could not turn it back on. It is paused and hidden until you bring it back below.",
      );
      setBusy(false);
      return;
    }
    clearRecoveryPending(vaultId);
    setRecoveryPending(false);
    if (enablePayload.vault) applyVault(enablePayload.vault);
    else {
      setVault((current) =>
        current ? { ...current, enabled: true } : current,
      );
      if (enablePayload.registry_revision !== undefined)
        setRevision(enablePayload.registry_revision);
    }
    setMessage("Saved.");
    setBusy(false);
  };

  const handleSave = () => {
    if (!draftSource) {
      void plainSave(undefined);
      return;
    }
    if (missingRequiredRepositoryUrl(draftSource)) {
      setMessage(REPOSITORY_URL_REQUIRED_MESSAGE);
      return;
    }
    if (identityChanged) {
      setConfirmation({
        newSource: draftSource,
        localHistory:
          draftSource.type === "existing_git" &&
          draftSource.mode === "local_history",
      });
      return;
    }
    void plainSave(draftSource);
  };

  const handleRecover = async () => {
    setBusy(true);
    setMessage(null);
    const result = await recoverPausedVault(vaultId);
    if (result.ok) {
      setRecoveryPending(false);
      if (result.vault) applyVault(result.vault);
      setMessage("This Vault is back.");
    } else {
      setMessage(result.message);
    }
    setBusy(false);
  };

  const syncOrRetry = async () => {
    setSyncing(true);
    setMessage(null);
    const healthy = vault.git !== "unavailable";
    const { ok, payload } = await requestJson(
      `/api/v1/vaults/${vaultId}/${healthy ? "sync" : "retry"}`,
      { method: "POST" },
    );
    if (!ok)
      setMessage(
        (payload as { message?: string }).message ??
          "Could not start a Git sync for this Vault.",
      );
    const discovery = await requestJson("/api/v1/vaults");
    if (discovery.ok) {
      const refreshed = (
        discovery.payload as VaultDiscoveryResponse
      ).vaults?.find((item) => item.vault_id === vaultId);
      if (refreshed) applyVault(refreshed);
    }
    setSyncing(false);
  };

  const gitFailure =
    vault.git === "unavailable" && vault.git_error
      ? describeGitFailure(vault.git_error)
      : null;
  const consoleVisible = vault.source !== undefined && vault.git !== "disabled";

  return (
    <div className="settings-main settings-vault-detail">
      <div className="settings-sec-head">
        <div>
          <h2 className="settings-sec-title">{vault.name}</h2>
          <p className="settings-sec-blurb">
            {sourceLabel(vault.source)} · {count ?? 0} notes ·{" "}
            {lastChanged(changed)}
          </p>
        </div>
      </div>
      {recoveryPending ? (
        <div className="settings-recovery-line" role="alert">
          <span>
            This Vault changed but did not start back up. It is paused and
            hidden until it is back.
          </span>
          <button
            type="button"
            className="settings-mini settings-btn-danger"
            disabled={busy}
            onClick={() => void handleRecover()}
          >
            Try to bring this Vault back
          </button>
        </div>
      ) : null}
      {consoleVisible ? (
        <div className="settings-console settings-git-console">
          <div className="settings-console-cell">
            <span className="settings-console-lbl">Sync</span>
            <span className="settings-console-val">
              {gitFailure ? gitFailure.label : "Healthy"}
            </span>
          </div>
          <div
            className="settings-console-strip"
            data-tier={gitFailure ? gitFailure.tier : "ok"}
          >
            <p>
              {gitFailure
                ? gitFailure.sentence
                : "This Vault's Git sync is healthy."}
            </p>
            {gitFailure?.files ? (
              <ul className="settings-console-files">
                {gitFailure.files.map((path) => (
                  <li key={path}>{path}</li>
                ))}
                {gitFailure.filesTotal !== undefined &&
                gitFailure.filesTotal > gitFailure.files.length ? (
                  <li>
                    and {gitFailure.filesTotal - gitFailure.files.length} more
                  </li>
                ) : null}
              </ul>
            ) : null}
            <button
              type="button"
              className="settings-btn"
              disabled={!vault.enabled || syncing || vault.git === "pending"}
              onClick={() => void syncOrRetry()}
            >
              {gitFailure ? "Try again" : "Sync now"}
            </button>
          </div>
        </div>
      ) : null}
      <p className="settings-vault-condition">
        {paused
          ? "This Vault is paused. It is kept here so you can turn it back on."
          : conditionSentence(vault, count)}
      </p>
      {message ? (
        <div className="settings-notice" role="status">
          {message}
        </div>
      ) : null}
      <div className="settings-rows">
        <label className="settings-row">
          <span>
            <span className="settings-row-label">Name</span>
            <span className="settings-row-help">
              The name used everywhere this Vault is shown.
            </span>
          </span>
          <input
            className="settings-input"
            aria-label="Vault name"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </label>
        <label className="settings-row">
          <span>
            <span className="settings-row-label">
              Ignore these files and folders
            </span>
            <span className="settings-row-help">
              Comma-separated patterns left out of this Vault’s search.
            </span>
          </span>
          <input
            className="settings-input"
            aria-label="Ignore these files and folders"
            value={exclude}
            onChange={(event) => setExclude(event.target.value)}
          />
        </label>
        <label className="settings-row">
          <span>
            <span className="settings-row-label">Archive folder</span>
            <span className="settings-row-help">
              Empty uses this server’s archive folder.
            </span>
          </span>
          <input
            className="settings-input"
            aria-label="Archive folder"
            value={archive}
            onChange={(event) => setArchive(event.target.value)}
          />
        </label>
        <label className="settings-row">
          <span>
            <span className="settings-row-label">Recorded as (name)</span>
            <span className="settings-row-help">
              Empty uses the server identity.
            </span>
          </span>
          <input
            className="settings-input"
            aria-label="Recorded as (name)"
            placeholder={serverIdentity.name || "server value"}
            value={identityName}
            onChange={(event) => setIdentityName(event.target.value)}
          />
        </label>
        <label className="settings-row">
          <span>
            <span className="settings-row-label">Recorded as (email)</span>
            <span className="settings-row-help">
              Empty uses the server identity.
            </span>
          </span>
          <input
            className="settings-input"
            aria-label="Recorded as (email)"
            placeholder={serverIdentity.email || "server value"}
            value={identityEmail}
            onChange={(event) => setIdentityEmail(event.target.value)}
          />
        </label>
      </div>
      {vault.source ? (
        <div className="settings-plaque">
          <div className="settings-plaque-head-row">
            <p className="settings-plaque-head">Identity</p>
            {showPlaqueFields && !plaqueFieldsEditable ? (
              <button
                type="button"
                className="settings-mini"
                onClick={() => setPlaqueEditing(true)}
              >
                Edit
              </button>
            ) : null}
          </div>
          <dl>
            <div className="settings-plaque-row">
              <dt>Where this Vault came from</dt>
              <dd>{sourceLabel(vault.source)}</dd>
            </div>
            <div className="settings-plaque-row">
              <dt>Commit identity</dt>
              <dd>
                {vault.commit_identity
                  ? `${vault.commit_identity.name} <${vault.commit_identity.email}>`
                  : `${serverIdentity.name || "not set"} <${serverIdentity.email || "not set"}>`}
              </dd>
            </div>
            {showPlaqueFields ? (
              <>
                <div className="settings-plaque-row">
                  <dt>Repository</dt>
                  <dd>
                    {plaqueFieldsEditable ? (
                      <input
                        className="settings-input settings-plaque-field"
                        aria-label="Repository URL"
                        value={repoUrlDraft}
                        onChange={(event) =>
                          setRepoUrlDraft(event.target.value)
                        }
                      />
                    ) : (
                      repoUrlDraft || "not set"
                    )}
                  </dd>
                </div>
                <div className="settings-plaque-row">
                  <dt>Branch</dt>
                  <dd>
                    {plaqueFieldsEditable ? (
                      <input
                        className="settings-input settings-plaque-field"
                        aria-label="Branch"
                        value={branchDraft}
                        onChange={(event) => setBranchDraft(event.target.value)}
                      />
                    ) : (
                      branchDraft || "repository default"
                    )}
                  </dd>
                </div>
                <div className="settings-plaque-row">
                  <dt>Folder</dt>
                  <dd>
                    {plaqueFieldsEditable ? (
                      <input
                        className="settings-input settings-plaque-field"
                        aria-label="Folder within the repository"
                        value={subdirDraft}
                        onChange={(event) => setSubdirDraft(event.target.value)}
                      />
                    ) : (
                      subdirDraft || "repository root"
                    )}
                  </dd>
                </div>
              </>
            ) : null}
          </dl>
        </div>
      ) : null}
      {vault.source ? (
        <div className="settings-rows">
          <div className="settings-row">
            <span>
              <span className="settings-row-label">Git behaviour</span>
              <span className="settings-row-help">
                {vault.source.type === "managed_git"
                  ? "How Hatchdoor keeps this checkout's history."
                  : "Whether — and how — this Vault's folder keeps Git history."}
              </span>
            </span>
            <div
              className="settings-segmented"
              role="group"
              aria-label="Git behaviour"
            >
              {behaviorOptions(vault.source).map((item) => (
                <button
                  key={item.id}
                  type="button"
                  aria-pressed={draftBehavior === item.id}
                  onClick={() => setDraftBehavior(item.id)}
                >
                  {item.label}
                </button>
              ))}
            </div>
            {identityChanged ? (
              <p className="settings-row-class">
                Saving this runs the Vault through a pause‑edit‑restart round
                trip.
              </p>
            ) : null}
          </div>
          {remoteBackedDraft ? (
            <>
              <div className="settings-row">
                <span>
                  <span className="settings-row-label">
                    Sign-in
                    {signIn === "token" ? (
                      <span
                        className={
                          identityChanged
                            ? "settings-token-state settings-token-state-warn"
                            : "settings-token-state"
                        }
                      >
                        {identityChanged
                          ? "will be cleared"
                          : vault.credential_configured
                            ? "saved"
                            : "none"}
                      </span>
                    ) : null}
                  </span>
                  <span className="settings-row-help">
                    {signIn === "token"
                      ? identityChanged
                        ? "This identity change clears the stored token even if left blank. Sign in again afterward if this Vault still needs one."
                        : "Blank means keep."
                      : "No sign-in removes any stored token."}
                  </span>
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
              {/* #148's AC4 (per-Vault write-debounce) resolved: the legacy
                  HATCHDOOR_GIT_DEBOUNCE_SECONDS concept ("wait N seconds
                  after the last local edit before committing") has no
                  successor in the multi-Vault pipeline, which already
                  coalesces writes through a fixed watcher debounce
                  independent of any per-Vault setting. This schedule field —
                  "how often to check the remote" — is a different question
                  Hatchdoor genuinely has no other way to answer, and is the
                  only per-Vault timing control this ticket adds. #148's AC4
                  is retired with no successor, not folded into this field. */}
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
                    onChange={(event) =>
                      setPollMinutesDraft(event.target.value)
                    }
                  />
                  <span className="settings-unit">minutes</span>
                </div>
              </div>
            </>
          ) : null}
        </div>
      ) : null}
      <div className="settings-sec-actions">
        <button
          className="settings-btn settings-btn-hot"
          disabled={revision === null || busy}
          onClick={handleSave}
          type="button"
        >
          Save Vault
        </button>
      </div>
      <div className="settings-vault-actions">
        <button
          className="settings-btn"
          disabled={revision === null || busy}
          onClick={() =>
            void mutate(
              `/api/v1/vaults/${vaultId}/${paused ? "enable" : "disable"}?expected_registry_revision=${revision}`,
              { method: "POST" },
            )
          }
          type="button"
        >
          {paused ? "Resume Vault" : "Pause Vault"}
        </button>
        <button
          className="settings-btn"
          disabled={paused}
          onClick={() =>
            void mutate(`/api/v1/vaults/${vaultId}/refresh`, { method: "POST" })
          }
          type="button"
        >
          Rebuild search index
        </button>
        <button
          className="settings-btn settings-btn-danger"
          disabled={revision === null || busy}
          onClick={async () => {
            if (
              await mutate(
                `/api/v1/vaults/${vaultId}?expected_registry_revision=${revision}`,
                { method: "DELETE" },
              )
            )
              onDisconnect();
          }}
          type="button"
        >
          Disconnect Vault
        </button>
      </div>
      {confirmation ? (
        <div className="settings-modal-back">
          <div
            className="settings-modal"
            role="dialog"
            aria-modal="true"
            aria-label="Before this is saved"
          >
            <h3>Before this is saved</h3>
            <p>{IDENTITY_CHANGE_CONSEQUENCE}</p>
            {confirmation.localHistory ? (
              <p>{LOCAL_HISTORY_CONSEQUENCE}</p>
            ) : null}
            {vault.credential_configured ? (
              <p>
                Its stored access token will be cleared — sign in again
                afterward if this Vault still needs one.
              </p>
            ) : null}
            <div className="settings-modal-actions">
              <button
                type="button"
                className="settings-btn"
                onClick={() => setConfirmation(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="settings-btn settings-btn-hot"
                onClick={() => void runIdentityChange(confirmation.newSource)}
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

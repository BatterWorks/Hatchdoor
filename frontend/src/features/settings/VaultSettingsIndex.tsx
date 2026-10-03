import { Fragment, useCallback, useEffect, useRef, useState } from "react";

import { apiFetch } from "../../api/api";
import { UnknownCount, VaultSlot } from "../../app/vaultSlot";
import type { VaultSlotState } from "../../app/vaultSlotLogic";
import { StateBlock } from "../../components/ui";
import {
  CONTEXTUAL_HELP,
  ContextualHelpLink,
  gitConsoleHelp,
  vaultConditionHelp,
} from "../help";
import { copyText } from "../../lib/clipboard";
import type { VaultId, VaultSource, VaultSummary } from "../../types";
import { useVaultCollection, useVaultProjection } from "../../vaults";
import { formatWhen } from "./relativeTime";
import { SettingsModal } from "./SettingsModal";
import { VaultCreationDialog } from "./VaultCreation";
import {
  behaviorOf,
  behaviorOptions,
  buildSourceForBehavior,
  clampPollMinutes,
  clearRecoveryPending,
  DEFAULT_POLL_MINUTES,
  describeGitFailure,
  describeRecoveryFailure,
  fetchRegistryRevision,
  gitSource,
  type GitBehavior,
  isRecoveryPending,
  isRemoteBacked,
  markRecoveryPending,
  MAX_POLL_MINUTES,
  MIN_POLL_MINUTES,
  missingRequiredRepositoryUrl,
  parseExcludePatterns,
  recoverPausedVault,
  recoveryBranchName,
  recoveryBranchUrl,
  REPOSITORY_URL_REQUIRED_MESSAGE,
  requestJson,
  sameSourceIdentity,
  sourceLabel,
  withIdentityFields,
} from "./vaultGitBehavior";

function conditionSentence(slot: VaultSlotState): string {
  return slot.kind === "condition"
    ? slot.sentence
    : "This Vault is ready to use.";
}

/** What the detail header knows about a Vault's last change: still being
 * read, refused (an unavailable or still-indexing Vault answers `/recent`
 * with an error), or read, possibly with nothing in it yet. */
type LastChange =
  | { state: "loading" }
  | { state: "unavailable" }
  | { state: "loaded"; mtimeNs: number | undefined };

function lastChanged(change: LastChange): string {
  if (change.state === "loading") return "checking last change";
  if (change.state === "unavailable") return "last change unavailable";
  const mtimeNs = change.mtimeNs;
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
}: {
  selectedVaultId: VaultId | null;
  onSelectVault: (vaultId: VaultId) => void;
  /** Set by `SettingsPage` when navigation carried an "open the creation
   * flow immediately" request — the zero-Vault workspace state (#150)'s
   * `Add a Vault` button lands here rather than rendering its own copy of
   * the flow. */
  autoOpenCreation?: boolean;
}) {
  const [recovering, setRecovering] = useState<Record<VaultId, boolean>>({});
  const [creationOpen, setCreationOpen] = useState(Boolean(autoOpenCreation));
  // The whole Vault collection, disabled Vaults included, from the one client
  // every surface reads (#198). `recovery` means the persisted registry file
  // itself is unreadable (#150) — distinct from a Vault-level `needs
  // attention` recovery below, it replaces the whole group.
  // `legacy_migration_recovery` is deliberately not surfaced here: the
  // registry loads fine (empty) in that case, so the group renders its
  // ordinary zero-Vault "Add a Vault" state.
  const {
    allVaults: vaults,
    noteCounts: counts,
    demoMode,
    recovery: registryRecovery,
    readState,
    error: discoveryError,
    refresh: loadVaults,
  } = useVaultCollection();

  const handleRecover = async (vaultId: VaultId) => {
    setRecovering((old) => ({ ...old, [vaultId]: true }));
    const result = await recoverPausedVault(vaultId);
    if (result.ok) await loadVaults();
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
          help={<ContextualHelpLink to={CONTEXTUAL_HELP.registryRecovery} />}
        />
      </section>
    );
  }

  // A failed discovery knows nothing about the registry: an empty group with
  // its Add a Vault action would read as "you have no Vaults" (#333).
  if (readState === "error") {
    return (
      <section className="settings-vault-index" aria-label="Vaults">
        <p className="settings-index-group">Vaults</p>
        <StateBlock
          tone="error"
          title="Vaults Unavailable"
          description={`${discoveryError ?? "Could not load your Vaults."} Nothing was changed, and your Markdown is untouched.`}
          actionLabel="Try again"
          onAction={() => void loadVaults()}
          help={<ContextualHelpLink to={CONTEXTUAL_HELP.vaultsUnavailable} />}
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
        // A row in the index, not a link under it: adding a Vault is the last
        // entry in the collection this list is, and the underlined link made
        // the one thing you cannot select the loudest thing in the list
        // (#120).
        <button
          className="settings-index-item settings-vault-index-add"
          type="button"
          onClick={() => setCreationOpen(true)}
        >
          <span className="settings-index-title">Add a Vault</span>
        </button>
      )}
      {creationOpen && !demoMode ? (
        <VaultCreationDialog
          onClose={() => setCreationOpen(false)}
          onCreated={(vault) => {
            setCreationOpen(false);
            // Every surface picks the new Vault up from the collection
            // client's own refresh; nothing here hands it along.
            void loadVaults();
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

/** The server's own `registry_revision_conflict` message is an internal
 * diagnostic (`expected registry revision N, current revision is M`), so this
 * page says what it means instead (#338). A Save carries every field of this
 * form, so after a conflict it is not silently re-sent over whatever changed:
 * the page adopts the fresh revision and asks for the Save again. A pause,
 * resume or disconnect carries no form fields, reads the revision fresh, and
 * so only conflicts when something moved in the instant between. */
const SAVE_CONFLICT_MESSAGE =
  "This Vault's settings changed elsewhere since this page opened. Your edits are still here: press Save Vault again to save them over that change.";
const ACTION_CONFLICT_MESSAGE =
  "This Vault changed elsewhere just now. Try again.";
const UNREACHABLE_MESSAGE =
  "Could not reach the server. Check the connection and try again.";

function failureText(payload: Record<string, unknown>, fallback: string) {
  if (payload.code === "registry_revision_conflict")
    return ACTION_CONFLICT_MESSAGE;
  return typeof payload.message === "string" ? payload.message : fallback;
}

/** The recovery-branch half of a conflicted Vault's Git console (ADR-30):
 * publish this Vault's side to a branch, then merge it on the Git host.
 * The outcome arrives on the Vault's `recovery_branch` status, not on the
 * request, so this renders from the summary and only starts the publish. */
function RecoveryBranchPanel({
  vault,
  publishing,
  onPublish,
}: {
  vault: VaultSummary;
  publishing: boolean;
  onPublish: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const status = vault.recovery_branch;
  const branch = recoveryBranchName(vault);
  const url = branch ? recoveryBranchUrl(vault.source, branch) : null;
  const configuredBranch = gitSource(vault.source)?.branch;
  const publishedLine = status?.published_commit
    ? `Published ${status.published_commit.slice(0, 7)}${
        status.published_at ? ` ${formatWhen(status.published_at)}` : ""
      }. Saves made since then are not on the branch until you publish again.`
    : null;
  return (
    <div className="settings-console-recovery">
      <p>
        Publish this Vault&rsquo;s side to its own branch on the remote, merge
        that branch into {configuredBranch ?? "the synced branch"} with your
        usual Git tools, and syncing picks up again by itself. Hatchdoor never
        overwrites or deletes the branch.
      </p>
      {branch ? (
        <div className="settings-console-recovery-branch">
          <code>{branch}</code>
          <button
            type="button"
            className="settings-mini"
            onClick={() => {
              void copyText(branch).then((ok) => setCopied(ok));
            }}
          >
            {copied ? "Copied" : "Copy"}
          </button>
          {url && status?.published_commit ? (
            <a href={url} target="_blank" rel="noreferrer">
              Open on the Git host
            </a>
          ) : null}
        </div>
      ) : null}
      {publishedLine ? <p>{publishedLine}</p> : null}
      {status?.error ? (
        <p role="alert">{describeRecoveryFailure(status.error)}</p>
      ) : null}
      <button
        type="button"
        className="settings-btn"
        disabled={publishing || !vault.enabled}
        onClick={onPublish}
      >
        {status?.published_commit
          ? "Publish again"
          : "Publish my side to a branch"}
      </button>
    </div>
  );
}

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
  const {
    allVaults,
    registryRevision,
    noteCounts,
    refresh: refreshCollection,
  } = useVaultCollection();
  const vaultProjection = useVaultProjection();
  const summary = allVaults.find((item) => item.vault_id === vaultId);
  const count = noteCounts[vaultId];
  const [vault, setVault] = useState<VaultSummary | null>(null);
  // The mutation-sequencing token, not a projection of the collection: a
  // pause/edit/un-pause round trip carries the revision each step returned,
  // so it is seeded from the client and then advanced by the responses. It is
  // the base a Save is checked against; a conflict re-reads it (#338).
  const [revision, setRevision] = useState<number | null>(null);
  const [changed, setChanged] = useState<LastChange>({ state: "loading" });
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
  // A failure is announced assertively and drawn as one; progress and
  // success stay polite (#338).
  const [notice, setNotice] = useState<{ text: string; alert: boolean } | null>(
    null,
  );
  const setMessage = (text: string | null) =>
    setNotice(text === null ? null : { text, alert: false });
  const setFailure = (text: string) => setNotice({ text, alert: true });
  const [confirmation, setConfirmation] = useState<{
    newSource: VaultSource;
    localHistory: boolean;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [publishing, setPublishing] = useState(false);
  const [recoveryPending, setRecoveryPending] = useState(false);

  const applyVault = useCallback((next: VaultSummary) => {
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
  }, []);

  // The Vault record and its note count come from the collection client; only
  // this Vault's own last-change time is read here, because nothing else in
  // the app wants it. The editable drafts are seeded once per Vault: a later
  // collection refresh must not overwrite fields somebody is part-way through
  // editing.
  const appliedVaultIdRef = useRef<VaultId | null>(null);
  useEffect(() => {
    if (!summary || appliedVaultIdRef.current === vaultId) {
      return;
    }
    appliedVaultIdRef.current = vaultId;
    applyVault(summary);
    setRevision(registryRevision);
  }, [applyVault, registryRevision, summary, vaultId]);

  // The displayed record, though, follows the collection: another writer's
  // change or an SSE revision must not leave this page describing a Vault the
  // Settings index disagrees with. Only a genuinely new collection state is
  // adopted (the client keeps a Vault's identity across a refresh that found
  // nothing new), so a mutation's own fresher response is never overwritten by
  // a collection read that has not caught up yet — and the identity round trip
  // below, which shows its intermediate pause/edit/un-pause states on purpose,
  // is left alone while it runs. A record that arrives mid-round-trip is left
  // unconsumed rather than marked adopted and dropped: the collection keeps a
  // Vault's identity across a refresh that found nothing new, so a record
  // consumed without being applied would never be offered again.
  const adoptedSummaryRef = useRef<VaultSummary | undefined>(undefined);
  useEffect(() => {
    if (!summary || summary === adoptedSummaryRef.current || busy) {
      return;
    }
    adoptedSummaryRef.current = summary;
    setVault(summary);
  }, [busy, summary]);

  // Reset on every Vault switch, and a refusal recorded as one: otherwise the
  // previous Vault's date would stay printed under this Vault's name (#338).
  useEffect(() => {
    let cancelled = false;
    setChanged({ state: "loading" });
    void (async () => {
      const response = await apiFetch(
        `/api/v1/vaults/${vaultId}/recent?limit=1`,
      );
      if (cancelled) return;
      if (!response.ok) {
        setChanged({ state: "unavailable" });
        return;
      }
      const recent = (await response.json()) as {
        data?: Array<{ mtime_ns: number }>;
      };
      if (!cancelled)
        setChanged({ state: "loaded", mtimeNs: recent.data?.[0]?.mtime_ns });
    })().catch(() => {
      if (!cancelled) setChanged({ state: "unavailable" });
    });
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

  // The saved sign-in state, which the controls fall back to whenever the
  // drafted behaviour stops showing them.
  const savedSignIn = vault.credential_configured ? "token" : "none";

  /** A behaviour switch clears whatever it takes off the screen, as the
   * creation flow's `selectBehavior` does (#338): a token typed for a remote
   * behaviour must not ride along, unseen, on a save the server refuses for
   * it. Fields that leave go back to their saved values rather than empty,
   * since this edits an existing Vault. */
  const selectBehavior = (next: GitBehavior) => {
    setDraftBehavior(next);
    if (!isRemoteBacked(next)) {
      const saved = draftsFromSource(vault.source);
      setSignIn(savedSignIn);
      setCredToken("");
      setPollMinutesDraft(saved.pollMinutes);
      if (next === "no_git") {
        setRepoUrlDraft(saved.repoUrl);
        setBranchDraft(saved.branch);
        setSubdirDraft(saved.subdirectory);
        setPlaqueEditing(false);
      }
    }
  };

  const credentialsPatch = ():
    | { action: "keep" }
    | { action: "remove" }
    | { action: "replace"; token: string } => {
    // Sign-in is not on screen for a behaviour without a remote, and the
    // registry drops a credential on such a source anyway.
    if (!remoteBackedDraft) return { action: "remove" };
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
    exclude_patterns: parseExcludePatterns(exclude),
    https_credentials: credentialsPatch(),
    ...(confirmIdentityChange ? { confirm_identity_change: true } : {}),
    archive_folder: archive || null,
    commit_identity: identity,
  });

  /** A registry-revision conflict means the base this page holds is behind;
   * adopt the current one so the next attempt can succeed rather than
   * re-sending the same stale number forever (#338). */
  const refreshRevision = async () => {
    const fresh = await fetchRegistryRevision();
    if (fresh !== null) setRevision(fresh);
  };

  const mutate = async (
    path: string,
    init: RequestInit,
    conflictMessage = ACTION_CONFLICT_MESSAGE,
  ) => {
    setMessage(null);
    const { ok, payload: raw } = await requestJson(path, init);
    const payload = raw as {
      vault?: VaultSummary;
      registry_revision?: number;
      message?: string;
      code?: string;
    };
    if (!ok) {
      if (payload.code === "registry_revision_conflict") {
        await refreshRevision();
        setFailure(conflictMessage);
      } else {
        setFailure(failureText(raw, "This Vault could not be changed."));
      }
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
    await mutate(
      `/api/v1/vaults/${vaultId}`,
      {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(editVaultBody(source, revision, false)),
      },
      SAVE_CONFLICT_MESSAGE,
    );
  };

  /** Pause, resume and disconnect say what to do, not what the fields should
   * be, so they read the revision at the moment of the click, as creation
   * and recovery already do, instead of trusting the one this page opened
   * with (#338). */
  const runAction = async (
    path: (expectedRevision: number) => string,
    init: RequestInit,
  ) => {
    setMessage(null);
    const fresh = await fetchRegistryRevision();
    if (fresh === null) {
      setFailure(UNREACHABLE_MESSAGE);
      return false;
    }
    setRevision(fresh);
    return mutate(path(fresh), init);
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
      if (disableResult.payload.code === "registry_revision_conflict") {
        await refreshRevision();
        setFailure(SAVE_CONFLICT_MESSAGE);
      } else {
        setFailure(
          disablePayload.message
            ? `Nothing changed. ${disablePayload.message}`
            : "Nothing changed — this Vault could not be paused for the edit.",
        );
      }
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
        setFailure(
          editPayload.message
            ? `Nothing changed. ${failureText(editResult.payload, editPayload.message)}`
            : "Nothing changed.",
        );
      } else {
        markRecoveryPending(vaultId);
        setRecoveryPending(true);
        setFailure(
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
      setFailure(
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
      setFailure(REPOSITORY_URL_REQUIRED_MESSAGE);
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
      setFailure(result.message);
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
      setFailure(
        failureText(payload, "Could not start a Git turn for this Vault."),
      );
    // No re-read here: the refresh publishes the new record and the effect
    // above adopts it, same as any other writer's change.
    await refreshCollection();
    setSyncing(false);
  };

  const publishRecovery = async () => {
    setPublishing(true);
    setMessage(null);
    const { ok, payload } = await requestJson(
      `/api/v1/vaults/${vaultId}/recovery-branch`,
      { method: "POST" },
    );
    if (ok)
      setMessage(
        "Publishing this Vault's side. The branch appears below once it is on the remote.",
      );
    else
      setFailure(
        failureText(payload, "Could not start publishing this Vault's side."),
      );
    await refreshCollection();
    setPublishing(false);
  };

  const gitFailure =
    vault.git === "unavailable" && vault.git_error
      ? describeGitFailure(vault.git_error)
      : null;
  const consoleVisible = vault.source !== undefined && vault.git !== "disabled";
  // A Vault with no remote commits and nothing else, so the console must not
  // offer it a sync the backend would only refuse, or claim a remote it does
  // not have (#267). Read off the capability record rather than the Git mode
  // string, and definition-derived, so a failing Vault keeps its own label.
  const remoteBacked = vault.capabilities.sync;
  const actionLabel = gitFailure
    ? "Try again"
    : remoteBacked
      ? "Sync now"
      : "Commit now";
  const conditionHelp = vaultConditionHelp(vault, paused);
  const healthySentence = remoteBacked
    ? "This Vault's Git sync is healthy."
    : "This Vault's Git history is up to date.";

  return (
    <div className="settings-main settings-vault-detail">
      <div className="settings-sec-head">
        <div>
          <h2 className="settings-sec-title">{vault.name}</h2>
          <p className="settings-sec-blurb">
            {sourceLabel(vault.source)} ·{" "}
            {count === undefined ? <UnknownCount inline /> : count} notes ·{" "}
            {lastChanged(changed)}
          </p>
        </div>
        {/* Save sits in the section head, where every instance section on this
            page keeps it — a Vault is a section like any other (#120). */}
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
            <span className="settings-console-lbl">
              {remoteBacked ? "Sync" : "History"}
            </span>
            <span className="settings-console-val">
              {gitFailure ? gitFailure.label : "Healthy"}
            </span>
          </div>
          <div
            className="settings-console-strip"
            data-tier={gitFailure ? gitFailure.tier : "ok"}
          >
            <p>
              {gitFailure ? gitFailure.sentence : healthySentence}{" "}
              <ContextualHelpLink to={gitConsoleHelp(vault)} />
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
            {vault.capabilities.publish_recovery ? (
              <RecoveryBranchPanel
                vault={vault}
                publishing={publishing}
                onPublish={() => void publishRecovery()}
              />
            ) : null}
            <button
              type="button"
              className="settings-btn"
              disabled={!vault.enabled || syncing || vault.git === "pending"}
              onClick={() => void syncOrRetry()}
            >
              {actionLabel}
            </button>
          </div>
        </div>
      ) : null}
      <p className="settings-vault-condition">
        {paused
          ? "This Vault is paused. It is kept here so you can turn it back on."
          : conditionSentence(vaultProjection.slotFor(vault))}
        {conditionHelp ? (
          <>
            {" "}
            <ContextualHelpLink to={conditionHelp} />
          </>
        ) : null}
      </p>
      {notice ? (
        <div
          className={
            notice.alert
              ? "settings-notice settings-notice-err"
              : "settings-notice"
          }
          role={notice.alert ? "alert" : "status"}
        >
          {notice.text}
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
                  onClick={() => selectBehavior(item.id)}
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
                <div className="settings-choice-stack">
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
      <div className="settings-vault-actions">
        <button
          className="settings-btn"
          disabled={busy}
          onClick={() =>
            void runAction(
              (expected) =>
                `/api/v1/vaults/${vaultId}/${paused ? "enable" : "disable"}?expected_registry_revision=${expected}`,
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
          disabled={busy}
          onClick={async () => {
            if (
              await runAction(
                (expected) =>
                  `/api/v1/vaults/${vaultId}?expected_registry_revision=${expected}`,
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
      {/* Said before the click, not after it: the word "disconnect" carries no
          promise about the notes on disk, and this is the only place that can
          make one (#120). */}
      <p className="settings-vault-disconnect-note">
        Disconnecting forgets this Vault. It never deletes your notes, the
        folder, its history, or anything on the server.
      </p>
      {confirmation ? (
        <SettingsModal
          label="Before this is saved"
          onClose={() => setConfirmation(null)}
        >
          <h3>Before this is saved</h3>
          <p>{IDENTITY_CHANGE_CONSEQUENCE}</p>
          {confirmation.localHistory ? (
            <p>{LOCAL_HISTORY_CONSEQUENCE}</p>
          ) : null}
          {vault.credential_configured ? (
            <p>
              Its stored access token will be cleared — sign in again afterward
              if this Vault still needs one.
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
        </SettingsModal>
      ) : null}
    </div>
  );
}

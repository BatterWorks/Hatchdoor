import { useEffect, useState } from "react";

import { apiFetch } from "../../api/api";
import { VaultSlot } from "../../app/vaultSlot";
import { deriveVaultSlot } from "../../app/vaultSlotLogic";
import type {
  VaultDiscoveryResponse,
  VaultId,
  VaultSummary,
} from "../../types";

type Counts = Record<VaultId, number>;

function sourceLabel(source: unknown): string {
  if (!source || typeof source !== "object" || !("type" in source))
    return "Vault source";
  const type = (source as { type?: string }).type;
  if (type === "local") return "A folder on this server";
  if (type === "existing_git") return "An existing Git folder";
  if (type === "managed_git") return "A managed Git checkout";
  return "Vault source";
}

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
}: {
  selectedVaultId: VaultId | null;
  onSelectVault: (vaultId: VaultId) => void;
}) {
  const [vaults, setVaults] = useState<VaultSummary[]>([]);
  const [counts, setCounts] = useState<Counts>({});

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const response = await apiFetch("/api/v1/vaults");
      if (!response.ok) return;
      const discovery = (await response.json()) as VaultDiscoveryResponse;
      if (cancelled || !Array.isArray(discovery.vaults)) return;
      setVaults(discovery.vaults);
      const stats = await apiFetch("/api/v1/vaults/all/stats");
      if (!stats.ok || cancelled) return;
      const payload = (await stats.json()) as {
        data?: Array<{ vault_id: VaultId; note_count: number }>;
      };
      if (!cancelled)
        setCounts(
          Object.fromEntries(
            (payload.data ?? []).map(({ vault_id, note_count }) => [
              vault_id,
              note_count,
            ]),
          ),
        );
    })().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <section className="settings-vault-index" aria-label="Vaults">
      <p className="settings-index-group">Vaults</p>
      {vaults.map((vault) => (
        <button
          className="settings-index-item settings-vault-index-item"
          data-active={vault.vault_id === selectedVaultId}
          data-paused={!vault.enabled}
          key={vault.vault_id}
          onClick={() => onSelectVault(vault.vault_id)}
          type="button"
        >
          <span className="settings-index-title">{vault.name}</span>
          {vault.enabled ? (
            <VaultSlot vault={vault} noteCount={counts[vault.vault_id]} />
          ) : (
            <span className="settings-vault-paused">paused</span>
          )}
        </button>
      ))}
      <button className="settings-link" type="button">
        Add a Vault
      </button>
    </section>
  );
}

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
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const discoveryResponse = await apiFetch("/api/v1/vaults");
      if (!discoveryResponse.ok) return;
      const discovery =
        (await discoveryResponse.json()) as VaultDiscoveryResponse;
      const next = discovery.vaults?.find((item) => item.vault_id === vaultId);
      if (!next || cancelled) return;
      setVault(next);
      setRevision(discovery.registry_revision ?? null);
      setName(next.name);
      setExclude(next.exclude_patterns.join(", "));
      setArchive(next.archive_folder ?? "");
      setIdentityName(next.commit_identity?.name ?? "");
      setIdentityEmail(next.commit_identity?.email ?? "");
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

  const mutate = async (path: string, init: RequestInit) => {
    setMessage(null);
    const response = await apiFetch(path, init);
    const payload = (await response.json()) as {
      vault?: VaultSummary;
      registry_revision?: number;
      message?: string;
    };
    if (!response.ok) {
      setMessage(payload.message ?? "This Vault could not be changed.");
      return false;
    }
    if (payload.vault) setVault(payload.vault);
    if (payload.registry_revision !== undefined)
      setRevision(payload.registry_revision);
    setMessage("Saved.");
    return true;
  };

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
      <div className="settings-sec-actions">
        <button
          className="settings-btn settings-btn-hot"
          disabled={revision === null}
          onClick={() =>
            void mutate(`/api/v1/vaults/${vaultId}`, {
              method: "PATCH",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({
                expected_registry_revision: revision,
                name,
                source: vault.source,
                exclude_patterns: exclude
                  .split(",")
                  .map((item) => item.trim())
                  .filter(Boolean),
                https_credentials: { action: "keep" },
                archive_folder: archive || null,
                commit_identity: identity,
              }),
            })
          }
          type="button"
        >
          Save Vault
        </button>
      </div>
      <div className="settings-plaque">
        <p className="settings-plaque-head">Identity</p>
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
        </dl>
      </div>
      <div className="settings-vault-actions">
        <button
          className="settings-btn"
          disabled={revision === null}
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
          disabled={revision === null}
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
    </div>
  );
}

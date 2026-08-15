/**
 * Recovery for drafts that predate Vault qualification (#137): a pre-#137
 * note draft was keyed by slug alone, and the standalone create draft has
 * never carried a Vault at all. Neither can be silently reused once notes
 * are addressed by Vault + slug, so both surface here for explicit,
 * one-row-at-a-time recovery instead. This is a migration artefact, not a
 * standing feature — the section withdraws for good once the last held
 * draft is dealt with.
 */

import { useState } from "react";
import { useNavigate } from "react-router-dom";

import { apiFetch } from "../../api/api";
import type { VaultId, VaultSummary } from "../../types";
import { saveNoteDraft, type HeldDraft } from "../../lib/writeDrafts";
import { formatWhen } from "./relativeTime";

/** Opens the create-note dialog prefilled with a recovered draft, targeting a
 * specific Vault and folder rather than whatever the shell would otherwise
 * infer for the currently open note. */
export type OpenCreateDraft = (
  vaultId: VaultId,
  folder: string,
  name: string,
  content: string,
) => void;

function draftTitle(draft: HeldDraft): string {
  if (draft.kind === "note") return draft.slug;
  return draft.name.trim() || "Untitled";
}

function draftLocation(draft: HeldDraft): string {
  if (draft.kind === "note") return `was at ${draft.slug}`;
  const folder = draft.folder.trim();
  return folder ? `${folder}/${draftTitle(draft)}` : draftTitle(draft);
}

function draftPreview(content: string): string {
  const flat = content.trim().replace(/\s+/g, " ");
  return flat.length > 160 ? `${flat.slice(0, 160)}…` : flat || "(empty)";
}

function DraftRow({
  draft,
  vaults,
  onOpenCreateDraft,
  onDiscard,
}: {
  draft: HeldDraft;
  vaults: VaultSummary[];
  onOpenCreateDraft?: OpenCreateDraft;
  onDiscard: (id: string) => void;
}) {
  const navigate = useNavigate();
  const [destination, setDestination] = useState<VaultId | "">(
    vaults.length === 1 ? vaults[0].vault_id : "",
  );
  const [restoring, setRestoring] = useState(false);
  const [noSuchNote, setNoSuchNote] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const restoreAsNewNote = () => {
    if (!destination || !onOpenCreateDraft) return;
    onOpenCreateDraft(
      destination,
      draft.kind === "create" ? draft.folder : "",
      draft.kind === "note" ? draft.slug : draft.name,
      draft.content,
    );
    onDiscard(draft.id);
  };

  const handleRestore = async () => {
    if (!destination) return;
    setMessage(null);
    setNoSuchNote(false);

    if (draft.kind === "create") {
      restoreAsNewNote();
      return;
    }

    setRestoring(true);
    try {
      const res = await apiFetch(
        `/api/v1/vaults/${encodeURIComponent(destination)}/notes/${encodeURIComponent(draft.slug)}`,
      );
      if (res.status === 404) {
        setNoSuchNote(true);
        return;
      }
      if (!res.ok) {
        setMessage("This Vault could not be checked. Try again.");
        return;
      }
      // The draft's own baseContentHash — the version it was actually typed
      // against — is preserved rather than replaced with the destination
      // note's current hash, so NotePage's existing stale-draft check (which
      // compares the two) still fires correctly on a note that moved on disk
      // since the draft was made.
      saveNoteDraft(destination, draft.slug, {
        vaultId: destination,
        slug: draft.slug,
        content: draft.content,
        baseContentHash: draft.baseContentHash,
        savedAt: Date.now(),
      });
      onDiscard(draft.id);
      navigate(
        `/v/${encodeURIComponent(destination)}/n/${encodeURIComponent(draft.slug)}?restoreEdit=1`,
      );
    } catch {
      setMessage("This Vault could not be checked. Try again.");
    } finally {
      setRestoring(false);
    }
  };

  const handleDiscard = () => {
    if (!window.confirm("Discard this unsaved draft? This cannot be undone.")) {
      return;
    }
    onDiscard(draft.id);
  };

  return (
    <div className="settings-plaque settings-draft-row">
      <div className="settings-plaque-head-row">
        <p className="settings-plaque-head">{draftTitle(draft)}</p>
        <span className="settings-muted">
          {formatWhen(draft.savedAt) ?? "just now"}
        </span>
      </div>
      <p className="settings-row-help">{draftLocation(draft)}</p>
      <pre className="settings-draft-preview">{draftPreview(draft.content)}</pre>
      <div className="settings-row settings-draft-destination">
        <span>
          <span className="settings-row-label">Destination Vault</span>
        </span>
        <select
          className="settings-input"
          aria-label="Destination Vault"
          value={destination}
          onChange={(event) => {
            setDestination(event.target.value);
            setNoSuchNote(false);
            setMessage(null);
          }}
        >
          <option value="">Choose a Vault…</option>
          {vaults.map((vault) => (
            <option key={vault.vault_id} value={vault.vault_id}>
              {vault.name}
            </option>
          ))}
        </select>
      </div>
      {noSuchNote ? (
        <p className="settings-warn settings-draft-no-note">
          This Vault has no note at {draft.kind === "note" ? draft.slug : ""}
          . Choose a different Vault, or{" "}
          {onOpenCreateDraft ? (
            <button
              type="button"
              className="settings-link"
              onClick={restoreAsNewNote}
            >
              restore it as a new note here
            </button>
          ) : (
            "restore it as a new note."
          )}
        </p>
      ) : null}
      {message ? <p className="settings-error">{message}</p> : null}
      <div className="settings-sec-actions settings-draft-actions">
        <button
          type="button"
          className="settings-btn"
          onClick={handleDiscard}
        >
          Discard
        </button>
        <button
          type="button"
          className="settings-btn settings-btn-hot"
          disabled={!destination || restoring}
          onClick={() => void handleRestore()}
        >
          {restoring ? "Checking…" : "Restore"}
        </button>
      </div>
    </div>
  );
}

export function UnsavedDrafts({
  drafts,
  vaults,
  onOpenCreateDraft,
  onDiscard,
}: {
  drafts: HeldDraft[];
  vaults: VaultSummary[];
  onOpenCreateDraft?: OpenCreateDraft;
  onDiscard: (id: string) => void;
}) {
  return (
    <div className="settings-main">
      <div className="settings-sec-head">
        <div>
          <h2 className="settings-sec-title">Unsaved drafts</h2>
          <p className="settings-sec-blurb">
            Recently viewed notes and open folders were reset for the move to
            multiple Vaults. These drafts, typed before that, were kept
            exactly as you left them — restore each into a Vault, or discard
            it.
          </p>
        </div>
      </div>
      <div className="settings-drafts-list">
        {drafts.map((draft) => (
          <DraftRow
            key={draft.id}
            draft={draft}
            vaults={vaults}
            onOpenCreateDraft={onOpenCreateDraft}
            onDiscard={onDiscard}
          />
        ))}
      </div>
    </div>
  );
}

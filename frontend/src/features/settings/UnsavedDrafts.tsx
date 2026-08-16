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

/** Writes a recovered draft into a specific Vault as a new note and opens it.
 * The create dialog no longer collects a body — a note is created empty and
 * written in place — so a held draft's text is restored by creating the note
 * with it directly. Resolves false when the note could not be written, so the
 * held draft is kept rather than discarded. */
export type RestoreCreateDraft = (
  vaultId: VaultId,
  folder: string,
  name: string,
  content: string,
) => Promise<boolean>;

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
  onRestoreCreateDraft,
  onDiscard,
}: {
  draft: HeldDraft;
  vaults: VaultSummary[];
  onRestoreCreateDraft?: RestoreCreateDraft;
  onDiscard: (id: string) => void;
}) {
  const navigate = useNavigate();
  const [destination, setDestination] = useState<VaultId | "">(
    vaults.length === 1 ? vaults[0].vault_id : "",
  );
  const [restoring, setRestoring] = useState(false);
  const [noSuchNote, setNoSuchNote] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const restoreAsNewNote = async () => {
    if (!destination || !onRestoreCreateDraft) return;
    setRestoring(true);
    try {
      const created = await onRestoreCreateDraft(
        destination,
        draft.kind === "create" ? draft.folder : "",
        draft.kind === "note" ? draft.slug : draft.name,
        draft.content,
      );
      // Discarded only once the text is on disk. A failed write that took the
      // draft with it would lose the very thing this screen exists to save.
      if (created) {
        onDiscard(draft.id);
      } else {
        setMessage("This draft could not be written. It is still held here.");
      }
    } finally {
      setRestoring(false);
    }
  };

  const handleRestore = async () => {
    if (!destination) return;
    setMessage(null);
    setNoSuchNote(false);

    if (draft.kind === "create") {
      await restoreAsNewNote();
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
      <pre className="settings-draft-preview">
        {draftPreview(draft.content)}
      </pre>
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
          This Vault has no note at {draft.kind === "note" ? draft.slug : ""}.
          Choose a different Vault, or{" "}
          {onRestoreCreateDraft ? (
            <button
              type="button"
              className="settings-link"
              onClick={() => void restoreAsNewNote()}
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
        <button type="button" className="settings-btn" onClick={handleDiscard}>
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
  onRestoreCreateDraft,
  onDiscard,
}: {
  drafts: HeldDraft[];
  vaults: VaultSummary[];
  onRestoreCreateDraft?: RestoreCreateDraft;
  onDiscard: (id: string) => void;
}) {
  return (
    <div className="settings-main">
      <div className="settings-sec-head">
        <div>
          <h2 className="settings-sec-title">Unsaved drafts</h2>
          <p className="settings-sec-blurb">
            Recently viewed notes and open folders were reset for the move to
            multiple Vaults. These drafts, typed before that, were kept exactly
            as you left them — restore each into a Vault, or discard it.
          </p>
        </div>
      </div>
      <div className="settings-drafts-list">
        {drafts.map((draft) => (
          <DraftRow
            key={draft.id}
            draft={draft}
            vaults={vaults}
            onRestoreCreateDraft={onRestoreCreateDraft}
            onDiscard={onDiscard}
          />
        ))}
      </div>
    </div>
  );
}

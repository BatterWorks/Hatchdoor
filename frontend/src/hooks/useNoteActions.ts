import { useCallback, useState } from "react";
import { useNavigate } from "react-router-dom";

import type { NoteActionDialogKind } from "../components/NoteActionsDialog";
import type { ActiveNoteMeta, VaultSummary } from "../types";
import { clearCreateDraft } from "../lib/writeDrafts";
import { validateNotePath } from "../lib/writePaths";
import { resolvePrimaryVaultId } from "./useVaultScope";
import {
  archiveNote,
  createNote,
  deleteNote,
  describeWriteOutcome,
  moveNote,
  renameNote,
} from "../api/writeApi";

interface UseNoteActionsParams {
  activeNote: ActiveNoteMeta | null;
  vaults: VaultSummary[];
  refreshVault: () => Promise<void>;
  setWriteNotice: (notice: string | null) => void;
}

/**
 * Owns the note-action dialog (create/rename/move/archive/delete) and the
 * handlers that perform each write, refresh the vault, and navigate to the
 * result. Depends on the current note, the enabled-Vault list (to resolve a
 * target Vault for a brand-new note with none open — see
 * `resolvePrimaryVaultId`), the vault refresher, and the write-notice setter,
 * which are threaded in from the shell.
 */
export function useNoteActions({
  activeNote,
  vaults,
  refreshVault,
  setWriteNotice,
}: UseNoteActionsParams) {
  const navigate = useNavigate();
  const [noteActionDialog, setNoteActionDialog] =
    useState<NoteActionDialogKind | null>(null);
  const [noteActionError, setNoteActionError] = useState<string | null>(null);
  const [noteActionInitialFolder, setNoteActionInitialFolder] = useState("");

  const openCreateDialog = useCallback((folder: string) => {
    setNoteActionError(null);
    setNoteActionInitialFolder(folder);
    setNoteActionDialog("create");
  }, []);

  const openActionDialog = useCallback((kind: NoteActionDialogKind) => {
    setNoteActionError(null);
    setNoteActionDialog(kind);
  }, []);

  const closeNoteActionDialog = useCallback(() => {
    setNoteActionDialog(null);
    setNoteActionError(null);
    setNoteActionInitialFolder("");
  }, []);

  const requireActiveNoteHash = useCallback(() => {
    if (!activeNote?.slug || !activeNote.contentHash) {
      throw new Error("Current note is not ready for write actions");
    }
    return {
      vaultId: activeNote.vaultId,
      slug: activeNote.slug,
      contentHash: activeNote.contentHash,
    };
  }, [activeNote]);

  const handleCreateNote = useCallback(
    async (relativePath: string, content: string) => {
      setNoteActionError(null);
      const pathError = validateNotePath(relativePath, { label: "Note path" });
      if (pathError) {
        setNoteActionError(pathError);
        return;
      }
      const targetVaultId = resolvePrimaryVaultId(activeNote?.vaultId, vaults);
      if (!targetVaultId) {
        setNoteActionError("No Vault is available to create a note in.");
        return;
      }
      try {
        const outcome = await createNote(targetVaultId, relativePath, content);
        clearCreateDraft();
        setNoteActionDialog(null);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(
            `/v/${encodeURIComponent(outcome.vault_id)}/n/${encodeURIComponent(outcome.slug)}`,
          );
        }
      } catch (error) {
        setNoteActionError(
          error instanceof Error ? error.message : "Create failed",
        );
      }
    },
    [activeNote, navigate, refreshVault, setWriteNotice, vaults],
  );

  const handleRenameNote = useCallback(
    async (newTitle: string) => {
      setNoteActionError(null);
      const trimmed = newTitle.trim();
      if (!trimmed) {
        setNoteActionError("New title is required.");
        return;
      }
      try {
        const { vaultId, slug, contentHash } = requireActiveNoteHash();
        const outcome = await renameNote(vaultId, slug, trimmed, contentHash);
        setNoteActionDialog(null);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(
            `/v/${encodeURIComponent(outcome.vault_id)}/n/${encodeURIComponent(outcome.slug)}`,
          );
        }
      } catch (error) {
        setNoteActionError(
          error instanceof Error ? error.message : "Rename failed",
        );
      }
    },
    [navigate, refreshVault, requireActiveNoteHash, setWriteNotice],
  );

  const handleMoveNote = useCallback(
    async (targetFolder: string) => {
      setNoteActionError(null);
      const pathError = validateNotePath(targetFolder, {
        allowEmpty: true,
        label: "Target folder",
      });
      if (pathError) {
        setNoteActionError(pathError);
        return;
      }
      try {
        const { vaultId, slug, contentHash } = requireActiveNoteHash();
        const outcome = await moveNote(
          vaultId,
          slug,
          targetFolder,
          contentHash,
        );
        setNoteActionDialog(null);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(
            `/v/${encodeURIComponent(outcome.vault_id)}/n/${encodeURIComponent(outcome.slug)}`,
          );
        }
      } catch (error) {
        setNoteActionError(
          error instanceof Error ? error.message : "Move failed",
        );
      }
    },
    [navigate, refreshVault, requireActiveNoteHash, setWriteNotice],
  );

  const handleArchiveNote = useCallback(async () => {
    setNoteActionError(null);
    try {
      const { vaultId, slug, contentHash } = requireActiveNoteHash();
      const outcome = await archiveNote(vaultId, slug, contentHash);
      setNoteActionDialog(null);
      setWriteNotice(describeWriteOutcome(outcome));
      await refreshVault();
      if (outcome.slug) {
        navigate(
          `/v/${encodeURIComponent(outcome.vault_id)}/n/${encodeURIComponent(outcome.slug)}`,
        );
      }
    } catch (error) {
      setNoteActionError(
        error instanceof Error ? error.message : "Archive failed",
      );
    }
  }, [navigate, refreshVault, requireActiveNoteHash, setWriteNotice]);

  const handleDeleteNote = useCallback(async () => {
    setNoteActionError(null);
    try {
      const { vaultId, slug, contentHash } = requireActiveNoteHash();
      const outcome = await deleteNote(vaultId, slug, contentHash);
      setNoteActionDialog(null);
      setWriteNotice(describeWriteOutcome(outcome));
      await refreshVault();
      navigate("/");
    } catch (error) {
      setNoteActionError(
        error instanceof Error ? error.message : "Delete failed",
      );
    }
  }, [navigate, refreshVault, requireActiveNoteHash, setWriteNotice]);

  return {
    noteActionDialog,
    noteActionError,
    noteActionInitialFolder,
    openCreateDialog,
    openActionDialog,
    closeNoteActionDialog,
    handleCreateNote,
    handleRenameNote,
    handleMoveNote,
    handleArchiveNote,
    handleDeleteNote,
  };
}

import { useCallback, useState } from "react";
import { useNavigate } from "react-router-dom";

import type { NoteActionDialogKind } from "../components/NoteActionsDialog";
import type { ActiveNoteMeta, VaultId, VaultSummary } from "../types";
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
  /** A demo_read_only refusal takes over entirely (#152): the dialog closes
   * with no retry and the shell's own sentence lands in the notice strip
   * instead of this dialog's inline error. Returns whether the error was a
   * demo refusal (and so already handled) so each catch block can fall back
   * to its ordinary inline-error path otherwise. */
  onDemoRefusal?: (error: unknown) => boolean;
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
  onDemoRefusal,
}: UseNoteActionsParams) {
  const navigate = useNavigate();
  const [noteActionDialog, setNoteActionDialog] =
    useState<NoteActionDialogKind | null>(null);
  const [noteActionError, setNoteActionError] = useState<string | null>(null);
  const [noteActionInitialFolder, setNoteActionInitialFolder] = useState("");
  // Set only when a caller (draft recovery, #151) needs the note created in a
  // specific Vault rather than whichever one `resolvePrimaryVaultId` would
  // otherwise infer from the currently open note.
  const [createTargetVaultId, setCreateTargetVaultId] =
    useState<VaultId | null>(null);

  const openCreateDialog = useCallback(
    (folder: string, targetVaultId?: VaultId) => {
      setNoteActionError(null);
      setNoteActionInitialFolder(folder);
      setCreateTargetVaultId(targetVaultId ?? null);
      setNoteActionDialog("create");
    },
    [],
  );

  const openActionDialog = useCallback((kind: NoteActionDialogKind) => {
    setNoteActionError(null);
    setNoteActionDialog(kind);
  }, []);

  const closeNoteActionDialog = useCallback(() => {
    setNoteActionDialog(null);
    setNoteActionError(null);
    setNoteActionInitialFolder("");
    setCreateTargetVaultId(null);
  }, []);

  // Shared by every write handler's catch block below: a demo_read_only
  // refusal closes the dialog with no retry and defers to the shell's own
  // notice-strip sentence, rather than this dialog's inline error (#152).
  const handleDemoRefusal = useCallback(
    (error: unknown): boolean => {
      if (!onDemoRefusal?.(error)) {
        return false;
      }
      setNoteActionDialog(null);
      return true;
    },
    [onDemoRefusal],
  );

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
      const targetVaultId =
        createTargetVaultId ??
        resolvePrimaryVaultId(activeNote?.vaultId, vaults);
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
        if (handleDemoRefusal(error)) {
          return;
        }
        setNoteActionError(
          error instanceof Error ? error.message : "Create failed",
        );
      }
    },
    [
      activeNote,
      createTargetVaultId,
      handleDemoRefusal,
      navigate,
      refreshVault,
      setWriteNotice,
      vaults,
    ],
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
        if (handleDemoRefusal(error)) {
          return;
        }
        setNoteActionError(
          error instanceof Error ? error.message : "Rename failed",
        );
      }
    },
    [handleDemoRefusal, navigate, refreshVault, requireActiveNoteHash, setWriteNotice],
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
        if (handleDemoRefusal(error)) {
          return;
        }
        setNoteActionError(
          error instanceof Error ? error.message : "Move failed",
        );
      }
    },
    [handleDemoRefusal, navigate, refreshVault, requireActiveNoteHash, setWriteNotice],
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
      if (handleDemoRefusal(error)) {
        return;
      }
      setNoteActionError(
        error instanceof Error ? error.message : "Archive failed",
      );
    }
  }, [handleDemoRefusal, navigate, refreshVault, requireActiveNoteHash, setWriteNotice]);

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
      if (handleDemoRefusal(error)) {
        return;
      }
      setNoteActionError(
        error instanceof Error ? error.message : "Delete failed",
      );
    }
  }, [handleDemoRefusal, navigate, refreshVault, requireActiveNoteHash, setWriteNotice]);

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

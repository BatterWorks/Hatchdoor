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
    async (targetVaultId: VaultId, relativePath: string) => {
      setNoteActionError(null);
      const pathError = validateNotePath(relativePath, { label: "Note path" });
      if (pathError) {
        setNoteActionError(pathError);
        return;
      }
      if (!targetVaultId) {
        setNoteActionError("No Vault is available to create a note in.");
        return;
      }
      try {
        // Empty: the note is written in place once it opens, so there is
        // nothing for the dialog to collect first.
        const outcome = await createNote(targetVaultId, relativePath, "");
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
    [handleDemoRefusal, navigate, refreshVault, setWriteNotice],
  );

  /**
   * Recovery for a held draft that still carries a body (Settings, #151).
   * The create dialog has no content field to hand it back through, so the
   * note is written with that body directly and opened. Reports success so
   * the caller only discards the held draft once the text is safely on disk.
   */
  const restoreCreateDraft = useCallback(
    async (
      targetVaultId: VaultId,
      relativePath: string,
      content: string,
    ): Promise<boolean> => {
      const pathError = validateNotePath(relativePath, { label: "Note path" });
      if (pathError) {
        setWriteNotice(pathError);
        return false;
      }
      try {
        const outcome = await createNote(targetVaultId, relativePath, content);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(
            `/v/${encodeURIComponent(outcome.vault_id)}/n/${encodeURIComponent(outcome.slug)}`,
          );
        }
        return true;
      } catch (error) {
        if (handleDemoRefusal(error)) {
          return false;
        }
        setWriteNotice(
          error instanceof Error ? error.message : "Create failed",
        );
        return false;
      }
    },
    [handleDemoRefusal, navigate, refreshVault, setWriteNotice],
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
    [
      handleDemoRefusal,
      navigate,
      refreshVault,
      requireActiveNoteHash,
      setWriteNotice,
    ],
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
    [
      handleDemoRefusal,
      navigate,
      refreshVault,
      requireActiveNoteHash,
      setWriteNotice,
    ],
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
  }, [
    handleDemoRefusal,
    navigate,
    refreshVault,
    requireActiveNoteHash,
    setWriteNotice,
  ]);

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
  }, [
    handleDemoRefusal,
    navigate,
    refreshVault,
    requireActiveNoteHash,
    setWriteNotice,
  ]);

  // Create starts on the Vault the writer clicked in, falling back to the one
  // holding the open note. Every other action addresses the open note, so it
  // can only concern that note's own Vault.
  const noteActionInitialVaultId =
    noteActionDialog === "create"
      ? (createTargetVaultId ??
        resolvePrimaryVaultId(activeNote?.vaultId, vaults))
      : activeNote?.vaultId;

  return {
    noteActionDialog,
    noteActionError,
    noteActionInitialFolder,
    noteActionInitialVaultId,
    openCreateDialog,
    openActionDialog,
    closeNoteActionDialog,
    handleCreateNote,
    restoreCreateDraft,
    handleRenameNote,
    handleMoveNote,
    handleArchiveNote,
    handleDeleteNote,
  };
}

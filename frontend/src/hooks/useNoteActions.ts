import { useCallback, useState } from "react";
import { useNavigate } from "react-router-dom";

import type { NoteActionDialogKind } from "../components/NoteActionsDialog";
import type { ActiveNoteMeta } from "../types";
import { clearCreateDraft } from "../lib/writeDrafts";
import { validateNotePath } from "../lib/writePaths";
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
  refreshVault: () => Promise<void>;
  setWriteNotice: (notice: string | null) => void;
}

/**
 * Owns the note-action dialog (create/rename/move/archive/delete) and the
 * handlers that perform each write, refresh the vault, and navigate to the
 * result. Depends on the current note, the vault refresher, and the write-notice
 * setter, which are threaded in from the shell.
 */
export function useNoteActions({
  activeNote,
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
    return { slug: activeNote.slug, contentHash: activeNote.contentHash };
  }, [activeNote]);

  const handleCreateNote = useCallback(
    async (relativePath: string, content: string) => {
      setNoteActionError(null);
      const pathError = validateNotePath(relativePath, { label: "Note path" });
      if (pathError) {
        setNoteActionError(pathError);
        return;
      }
      try {
        const outcome = await createNote(relativePath, content);
        clearCreateDraft();
        setNoteActionDialog(null);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(`/n/${encodeURIComponent(outcome.slug)}`);
        }
      } catch (error) {
        setNoteActionError(
          error instanceof Error ? error.message : "Create failed",
        );
      }
    },
    [navigate, refreshVault, setWriteNotice],
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
        const { slug, contentHash } = requireActiveNoteHash();
        const outcome = await renameNote(slug, trimmed, contentHash);
        setNoteActionDialog(null);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(`/n/${encodeURIComponent(outcome.slug)}`);
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
        const { slug, contentHash } = requireActiveNoteHash();
        const outcome = await moveNote(slug, targetFolder, contentHash);
        setNoteActionDialog(null);
        setWriteNotice(describeWriteOutcome(outcome));
        await refreshVault();
        if (outcome.slug) {
          navigate(`/n/${encodeURIComponent(outcome.slug)}`);
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
      const { slug, contentHash } = requireActiveNoteHash();
      const outcome = await archiveNote(slug, contentHash);
      setNoteActionDialog(null);
      setWriteNotice(describeWriteOutcome(outcome));
      await refreshVault();
      if (outcome.slug) {
        navigate(`/n/${encodeURIComponent(outcome.slug)}`);
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
      const { slug, contentHash } = requireActiveNoteHash();
      const outcome = await deleteNote(slug, contentHash);
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

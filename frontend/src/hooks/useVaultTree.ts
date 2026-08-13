import { useCallback, useEffect, useMemo, useState } from "react";

import { apiFetch, withAccessToken } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import { collectFolderPaths } from "../lib/folderPaths";
import { flattenNoteCandidates } from "../lib/noteCandidates";
import { isExplorerTreeEqual } from "../lib/stateCompare";
import { missingVaultNames } from "../lib/vaultParticipants";
import type {
  ExplorerFolder,
  ModifiedNote,
  VaultReadProjection,
  VaultScope,
  VaultTree,
} from "../types";

/** Merges every participating Vault's tree into the one `ExplorerFolder`
 * shape the explorer already renders. With exactly one participant (a
 * single-enabled-Vault instance, or any narrowed scope) this is the
 * participant's own tree, unchanged — byte-identical to today. With more
 * than one, top-level folders and notes are concatenated without deep
 * merging same-named folders across Vaults: each note still carries its own
 * `vault_id`, so links and edits target the correct Vault, but the visual
 * grouping an accordion would give is a later slice (#117) this ticket
 * explicitly excludes. */
function mergeVaultTrees(vaultTrees: VaultTree[]): ExplorerFolder | null {
  if (vaultTrees.length === 0) {
    return null;
  }
  if (vaultTrees.length === 1) {
    return vaultTrees[0].tree;
  }
  return {
    name: "Vaults",
    folders: vaultTrees.flatMap((vaultTree) => vaultTree.tree.folders),
    notes: vaultTrees.flatMap((vaultTree) => vaultTree.tree.notes),
  };
}

/**
 * Owns the vault explorer tree and its live-refresh machinery for the given
 * scope: initial load, the SSE `vault-collection-revision` subscription that
 * bumps `vaultRevision`, and reload on revision change. Also derives the
 * folder-path and note-candidate lists the shell and dialogs consume.
 */
export function useVaultTree(scope: VaultScope) {
  const [tree, setTree] = useState<ExplorerFolder | null>(null);
  const [loadingTree, setLoadingTree] = useState(true);
  const [treeError, setTreeError] = useState<string | null>(null);
  const [treePartial, setTreePartial] = useState(false);
  const [modifiedNotes, setModifiedNotes] = useState<ModifiedNote[]>([]);
  const [modifiedNotesPartial, setModifiedNotesPartial] = useState(false);
  const [modifiedNotesMissingVaults, setModifiedNotesMissingVaults] =
    useState<string[]>([]);
  const [vaultRevision, setVaultRevision] = useState(0);

  const loadTree = useCallback(async () => {
    setTreeError(null);
    try {
      const res = await apiFetch(
        `/api/v1/vaults/${encodeURIComponent(scope)}/tree`,
      );
      if (!res.ok) {
        throw new Error(await readErrorMessage(res, "Failed loading tree"));
      }
      const projection = (await res.json()) as VaultReadProjection<VaultTree[]>;
      const nextTree = mergeVaultTrees(projection.data);
      setTree((prev) =>
        isExplorerTreeEqual(prev, nextTree) ? prev : nextTree,
      );
      setTreePartial(projection.partial);
    } catch (err) {
      setTreeError(
        err instanceof Error ? err.message : "Unknown tree loading error",
      );
    }
  }, [scope]);

  const loadModifiedNotes = useCallback(async () => {
    try {
      const params = new URLSearchParams({ limit: "5" });
      const res = await apiFetch(
        `/api/v1/vaults/${encodeURIComponent(scope)}/recent?${params.toString()}`,
      );
      if (!res.ok) {
        throw new Error(
          await readErrorMessage(res, "Failed loading modified notes"),
        );
      }
      const projection = (await res.json()) as VaultReadProjection<
        ModifiedNote[]
      >;
      setModifiedNotes(projection.data.slice(0, 5));
      setModifiedNotesPartial(projection.partial);
      setModifiedNotesMissingVaults(
        missingVaultNames(projection.participants),
      );
    } catch {
      setModifiedNotes([]);
      setModifiedNotesPartial(false);
      setModifiedNotesMissingVaults([]);
    }
  }, [scope]);

  useEffect(() => {
    void (async () => {
      setLoadingTree(true);
      await loadTree();
      await loadModifiedNotes();
      setLoadingTree(false);
    })();
  }, [loadModifiedNotes, loadTree]);

  useEffect(() => {
    if (!("EventSource" in window)) {
      return;
    }

    const events = new EventSource(withAccessToken("/api/v1/vaults/events"));
    const onCollectionRevision = (event: MessageEvent<string>) => {
      try {
        const payload = JSON.parse(event.data) as {
          collection_revision?: unknown;
        };
        if (typeof payload.collection_revision === "number") {
          const revision = payload.collection_revision;
          setVaultRevision((current) =>
            revision > current ? revision : current,
          );
        }
      } catch {
        // Ignore malformed event payloads; the next valid revision will resync.
      }
    };
    events.addEventListener("vault-collection-revision", onCollectionRevision);

    return () => {
      events.removeEventListener(
        "vault-collection-revision",
        onCollectionRevision,
      );
      events.close();
    };
  }, []);

  useEffect(() => {
    if (vaultRevision === 0) {
      return;
    }

    void loadTree();
    void loadModifiedNotes();
  }, [loadModifiedNotes, loadTree, vaultRevision]);

  const folderPaths = useMemo(() => collectFolderPaths(tree), [tree]);
  const noteCandidates = useMemo(() => flattenNoteCandidates(tree), [tree]);

  return {
    tree,
    loadingTree,
    treeError,
    treePartial,
    modifiedNotes,
    modifiedNotesPartial,
    modifiedNotesMissingVaults,
    vaultRevision,
    folderPaths,
    noteCandidates,
    loadTree,
    loadModifiedNotes,
  };
}

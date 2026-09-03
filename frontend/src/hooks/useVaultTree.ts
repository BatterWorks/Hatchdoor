import { useCallback, useEffect, useMemo, useState } from "react";

import { apiFetch } from "../api/api";
import { useVaultCollection } from "../vaults";
import { readErrorMessage } from "../api/apiError";
import { collectFolderPaths } from "../lib/folderPaths";
import { flattenNoteCandidates } from "../lib/noteCandidates";
import { isExplorerTreeEqual } from "../lib/stateCompare";
import { missingVaultNames } from "../lib/vaultParticipants";
import { attributeVaultTree } from "../lib/vaultTrees";
import type {
  ExplorerFolder,
  ModifiedNote,
  VaultReadProjection,
  VaultId,
  VaultScope,
  VaultTree,
  WireVaultTree,
} from "../types";

/** Merges every participating Vault's tree into the one `ExplorerFolder`
 * shape narrowed-scope (and single-Vault-instance) explorer rendering uses,
 * unchanged — byte-identical to today. The per-Vault accordion under `all`
 * (#142) renders each Vault's own tree from `vaultTrees` instead, so this
 * merge is never asked to stand in for that grouping. */
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
 * scope: initial load, and reload whenever the collection client's revision
 * moves. The `vault-collection-revision` subscription itself belongs to the
 * collection client (#198) — one stream for the whole app, so the tree and
 * every other surface invalidate on the same event rather than each opening
 * their own. Also derives the folder-path and note-candidate lists the shell
 * and dialogs consume.
 */
export function useVaultTree(scope: VaultScope) {
  const [tree, setTree] = useState<ExplorerFolder | null>(null);
  const [vaultTrees, setVaultTrees] = useState<VaultTree[]>([]);
  const [loadingTree, setLoadingTree] = useState(true);
  const [treeError, setTreeError] = useState<string | null>(null);
  const [treePartial, setTreePartial] = useState(false);
  const [modifiedNotes, setModifiedNotes] = useState<ModifiedNote[]>([]);
  const [modifiedNotesPartial, setModifiedNotesPartial] = useState(false);
  const [modifiedNotesMissingVaults, setModifiedNotesMissingVaults] = useState<
    string[]
  >([]);
  const { revision: vaultRevision } = useVaultCollection();

  const loadTree = useCallback(async () => {
    setTreeError(null);
    try {
      const res = await apiFetch(
        `/api/v1/vaults/${encodeURIComponent(scope)}/tree`,
      );
      if (!res.ok) {
        throw new Error(await readErrorMessage(res, "Failed loading tree"));
      }
      const projection = (await res.json()) as VaultReadProjection<
        WireVaultTree[]
      >;
      // Notes arrive without a vault ID; the tree they hang from carries it
      // (#192). Stamping them here is the last point at which the grouping is
      // still intact — everything below merges or flattens the trees.
      const trees = projection.data.map(attributeVaultTree);
      const nextTree = mergeVaultTrees(trees);
      setTree((prev) =>
        isExplorerTreeEqual(prev, nextTree) ? prev : nextTree,
      );
      setVaultTrees(trees);
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
      setModifiedNotesMissingVaults(missingVaultNames(projection.participants));
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
    if (vaultRevision === 0) {
      return;
    }

    void loadTree();
    void loadModifiedNotes();
  }, [loadModifiedNotes, loadTree, vaultRevision]);

  // Folder lists stay separated by Vault. Flattening the merged tree instead
  // produced one list in which "Projects" could mean a different Vault's
  // folder than the one the writer was looking at, while the target Vault was
  // being decided somewhere else entirely.
  const folderPathsByVault = useMemo(() => {
    const byVault: Record<VaultId, string[]> = {};
    for (const vaultTree of vaultTrees) {
      byVault[vaultTree.vault_id] = collectFolderPaths(vaultTree.tree);
    }
    return byVault;
  }, [vaultTrees]);
  const noteCandidates = useMemo(() => flattenNoteCandidates(tree), [tree]);

  return {
    tree,
    vaultTrees,
    loadingTree,
    treeError,
    treePartial,
    modifiedNotes,
    modifiedNotesPartial,
    modifiedNotesMissingVaults,
    vaultRevision,
    folderPathsByVault,
    noteCandidates,
    loadTree,
    loadModifiedNotes,
  };
}

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

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

/** How many changed notes the `/recent` read asks for: the API's ceiling,
 * newest first across every Vault in scope with no per-Vault share (#341).
 * Deliberately more than Changed on disk shows, because the server returns no
 * total and the rows past the panel's own limit are what make its
 * "and N more" line and its count true. */
const CHANGED_NOTES_FETCH_LIMIT = 25;

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

type ScopeRead = { scope: VaultScope; controller: AbortController };

/** The signal a read for `scope` must carry, or `null` when `scope` has
 * already been superseded and the read should not start at all. */
function scopeReadSignal(
  ref: { current: ScopeRead | null },
  scope: VaultScope,
): AbortSignal | null {
  if (ref.current === null) {
    // Before the scope effect has run there is nothing to supersede yet.
    ref.current = { scope, controller: new AbortController() };
  }
  return ref.current.scope === scope ? ref.current.controller.signal : null;
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
  const [treeMissingVaults, setTreeMissingVaults] = useState<string[]>([]);
  const [modifiedNotes, setModifiedNotes] = useState<ModifiedNote[]>([]);
  const [modifiedNotesPartial, setModifiedNotesPartial] = useState(false);
  const [modifiedNotesMissingVaults, setModifiedNotesMissingVaults] = useState<
    string[]
  >([]);
  const [modifiedNotesError, setModifiedNotesError] = useState<string | null>(
    null,
  );
  const { revision: vaultRevision } = useVaultCollection();
  // The collection revision the loaded tree reflects, taken from the
  // projection envelope rather than assumed. `loadInFlightRef` holds the load
  // that has not answered yet, because the revision the collection client
  // publishes on load arrives while the very first tree read is still open:
  // without waiting for it there is nothing to compare against, and the tree
  // and the recent list are each fetched a second time on every page load.
  const loadedRevisionRef = useRef<number | null>(null);
  const loadInFlightRef = useRef<Promise<void> | null>(null);
  // Every read made for the current scope carries this controller's signal,
  // whoever started it (the scope effect, a revision bump, a Retry), and a
  // scope change aborts it. A wide `all` read is the slow one, so without
  // this it routinely answered after the narrowed read that replaced it and
  // put every Vault's folders under the narrowed Vault's header (#334). The
  // scope is kept beside it so a read started from a closure over the old
  // scope (the recent read queued behind a superseded tree read) never
  // borrows the new scope's signal.
  const scopeReadRef = useRef<ScopeRead | null>(null);
  // Within one scope, two reads can still overlap (a revision bump while a
  // Retry is open). Only the newest one started may write.
  const treeRequestRef = useRef(0);
  const recentRequestRef = useRef(0);

  const loadTree = useCallback(async () => {
    const signal = scopeReadSignal(scopeReadRef, scope);
    if (signal === null) {
      return;
    }
    const request = ++treeRequestRef.current;
    const isCurrent = () =>
      !signal.aborted && request === treeRequestRef.current;
    setTreeError(null);
    try {
      const res = await apiFetch(
        `/api/v1/vaults/${encodeURIComponent(scope)}/tree`,
        { signal },
      );
      if (!res.ok) {
        throw new Error(await readErrorMessage(res, "Failed loading tree"));
      }
      const projection = (await res.json()) as VaultReadProjection<
        WireVaultTree[]
      >;
      if (!isCurrent()) {
        return;
      }
      // Notes arrive without a vault ID; the tree they hang from carries it
      // (#192). Stamping them here is the last point at which the grouping is
      // still intact — everything below merges or flattens the trees.
      loadedRevisionRef.current = projection.collection_revision;
      const trees = projection.data.map(attributeVaultTree);
      const nextTree = mergeVaultTrees(trees);
      setTree((prev) =>
        isExplorerTreeEqual(prev, nextTree) ? prev : nextTree,
      );
      setVaultTrees(trees);
      setTreePartial(projection.partial);
      setTreeMissingVaults(missingVaultNames(projection.participants));
    } catch (err) {
      if (!isCurrent()) {
        return;
      }
      setTreeError(
        err instanceof Error ? err.message : "Unknown tree loading error",
      );
    }
  }, [scope]);

  const loadModifiedNotes = useCallback(async () => {
    const signal = scopeReadSignal(scopeReadRef, scope);
    if (signal === null) {
      return;
    }
    const request = ++recentRequestRef.current;
    const isCurrent = () =>
      !signal.aborted && request === recentRequestRef.current;
    try {
      const params = new URLSearchParams({
        limit: String(CHANGED_NOTES_FETCH_LIMIT),
      });
      const res = await apiFetch(
        `/api/v1/vaults/${encodeURIComponent(scope)}/recent?${params.toString()}`,
        { signal },
      );
      if (!res.ok) {
        throw new Error(
          await readErrorMessage(res, "Failed loading modified notes"),
        );
      }
      const projection = (await res.json()) as VaultReadProjection<
        ModifiedNote[]
      >;
      if (!isCurrent()) {
        return;
      }
      setModifiedNotes(projection.data);
      setModifiedNotesPartial(projection.partial);
      setModifiedNotesMissingVaults(missingVaultNames(projection.participants));
      setModifiedNotesError(null);
    } catch (err) {
      if (!isCurrent()) {
        return;
      }
      // A read that never happened is not a quiet collection: keep it apart
      // from the empty answer so the panel can say it failed.
      setModifiedNotes([]);
      setModifiedNotesPartial(false);
      setModifiedNotesMissingVaults([]);
      setModifiedNotesError(
        err instanceof Error ? err.message : "Failed loading modified notes",
      );
    }
  }, [scope]);

  const loadTreeAndRecent = useCallback(async () => {
    const running = (async () => {
      await loadTree();
      await loadModifiedNotes();
    })();
    loadInFlightRef.current = running;
    try {
      await running;
    } finally {
      if (loadInFlightRef.current === running) {
        loadInFlightRef.current = null;
      }
    }
  }, [loadModifiedNotes, loadTree]);

  useEffect(() => {
    const controller = new AbortController();
    scopeReadRef.current = { scope, controller };
    loadedRevisionRef.current = null;
    void (async () => {
      setLoadingTree(true);
      await loadTreeAndRecent();
      if (!controller.signal.aborted) {
        setLoadingTree(false);
      }
    })();
    return () => controller.abort();
  }, [loadTreeAndRecent, scope]);

  useEffect(() => {
    if (vaultRevision === null) {
      return;
    }

    void (async () => {
      // A read already open may be about to answer at exactly this revision.
      await loadInFlightRef.current;
      if (loadedRevisionRef.current === vaultRevision) {
        return;
      }
      await loadTreeAndRecent();
    })();
  }, [loadTreeAndRecent, vaultRevision]);

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
    treeMissingVaults,
    modifiedNotes,
    modifiedNotesPartial,
    modifiedNotesMissingVaults,
    modifiedNotesError,
    vaultRevision,
    folderPathsByVault,
    noteCandidates,
    loadTree,
    loadModifiedNotes,
  };
}

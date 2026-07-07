import { useCallback, useEffect, useMemo, useState } from "react";

import { apiFetch, withAccessToken } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import { collectFolderPaths } from "../lib/folderPaths";
import { flattenNoteCandidates } from "../lib/noteCandidates";
import { isExplorerTreeEqual } from "../lib/stateCompare";
import type {
  ExplorerFolder,
  ModifiedNote,
  RecentlyModifiedResponse,
} from "../types";

/**
 * Owns the vault explorer tree and its live-refresh machinery: initial load,
 * the SSE `vault-revision` subscription that bumps `vaultRevision`, and reload
 * on revision change. Also derives the folder-path and note-candidate lists the
 * shell and dialogs consume.
 */
export function useVaultTree() {
  const [tree, setTree] = useState<ExplorerFolder | null>(null);
  const [loadingTree, setLoadingTree] = useState(true);
  const [treeError, setTreeError] = useState<string | null>(null);
  const [modifiedNotes, setModifiedNotes] = useState<ModifiedNote[]>([]);
  const [vaultRevision, setVaultRevision] = useState(0);

  const loadTree = useCallback(async () => {
    setTreeError(null);
    try {
      const res = await apiFetch("/api/tree");
      if (!res.ok) {
        throw new Error(await readErrorMessage(res, "Failed loading tree"));
      }
      const nextTree = (await res.json()) as ExplorerFolder;
      setTree((prev) =>
        isExplorerTreeEqual(prev, nextTree) ? prev : nextTree,
      );
    } catch (err) {
      setTreeError(
        err instanceof Error ? err.message : "Unknown tree loading error",
      );
    }
  }, []);

  const loadModifiedNotes = useCallback(async () => {
    try {
      const params = new URLSearchParams({ limit: "5" });
      const res = await apiFetch(`/api/recently-modified?${params.toString()}`);
      if (!res.ok) {
        throw new Error(
          await readErrorMessage(res, "Failed loading modified notes"),
        );
      }
      const json = (await res.json()) as RecentlyModifiedResponse;
      setModifiedNotes(json.notes.slice(0, 5));
    } catch {
      setModifiedNotes([]);
    }
  }, []);

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

    const events = new EventSource(withAccessToken("/api/vault-events"));
    const onVaultRevision = (event: MessageEvent<string>) => {
      try {
        const payload = JSON.parse(event.data) as { revision?: unknown };
        if (typeof payload.revision === "number") {
          const revision = payload.revision;
          setVaultRevision((current) =>
            revision > current ? revision : current,
          );
        }
      } catch {
        // Ignore malformed event payloads; the next valid revision will resync.
      }
    };
    events.addEventListener("vault-revision", onVaultRevision);

    return () => {
      events.removeEventListener("vault-revision", onVaultRevision);
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

  const refreshVault = useCallback(async () => {
    try {
      await apiFetch("/api/refresh", { method: "POST" });
    } catch {
      // Fall back to tree refresh even if force refresh endpoint fails.
    }
    await loadTree();
    await loadModifiedNotes();
  }, [loadModifiedNotes, loadTree]);

  const treeIsStale = Boolean(tree && treeError);
  const folderPaths = useMemo(() => collectFolderPaths(tree), [tree]);
  const noteCandidates = useMemo(() => flattenNoteCandidates(tree), [tree]);

  return {
    tree,
    loadingTree,
    treeError,
    treeIsStale,
    modifiedNotes,
    vaultRevision,
    folderPaths,
    noteCandidates,
    loadTree,
    loadModifiedNotes,
    refreshVault,
  };
}

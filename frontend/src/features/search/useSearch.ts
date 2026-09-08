import { useCallback, useEffect, useRef, useState } from "react";

import { apiFetch } from "../../api/api";
import { readErrorMessage } from "../../api/apiError";
import { missingVaultNames } from "../../lib/vaultParticipants";
import type {
  VaultId,
  VaultParticipant,
  VaultReadProjection,
} from "../../types";
import type { SearchResponse, SearchResult } from "./types";

/**
 * Command-palette search state: open/query/mode plus the debounced fetch and
 * focus-management effects. `setSearchOpen` is exposed so the shell's global
 * keyboard shortcuts can open the dialog.
 *
 * The read is always collection-wide. Narrowing the browsing scope used to
 * narrow this fetch as well, which pinned search to one Vault with nothing on
 * screen saying so: the dialog's facet rail was hidden at exactly that scope,
 * so a query matching only elsewhere came back looking like a query matching
 * nowhere. Search now asks every enabled Vault every time and hands the whole
 * answer to the dialog, whose own Vault filter (#144) opens on the browsing
 * scope and can be widened in place. Both stay lenses over the answer;
 * neither is the browsing scope itself (#119).
 */
export function useSearch() {
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchIncludeContent, setSearchIncludeContent] = useState(false);
  const [searchResults, setSearchResults] = useState<SearchResult[]>([]);
  const [searchPartial, setSearchPartial] = useState(false);
  const [searchMissingVaultNames, setSearchMissingVaultNames] = useState<
    string[]
  >([]);
  const [searchParticipants, setSearchParticipants] = useState<
    VaultParticipant[]
  >([]);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  // Pre-fills the dialog's own Vault filter from where a tag tap happened
  // (#144). Read once by the dialog on open and cleared the moment it
  // closes, so a later plain open never inherits a stale preselection.
  const [searchInitialVaultFilter, setSearchInitialVaultFilter] = useState<
    VaultId | undefined
  >(undefined);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const prevFocusRef = useRef<Element | null>(null);

  const openSearchForTag = useCallback((tag: string, vaultId: VaultId) => {
    setSearchQuery(`#${tag}`);
    setSearchIncludeContent(true);
    setSearchInitialVaultFilter(vaultId);
    setSearchOpen(true);
  }, []);

  useEffect(() => {
    if (!searchOpen) {
      setSearchInitialVaultFilter(undefined);
    }
  }, [searchOpen]);

  useEffect(() => {
    if (searchOpen) {
      prevFocusRef.current = document.activeElement;
      const id = window.setTimeout(() => searchInputRef.current?.focus(), 0);
      return () => window.clearTimeout(id);
    } else {
      if (prevFocusRef.current instanceof HTMLElement) {
        prevFocusRef.current.focus();
      }
      prevFocusRef.current = null;
    }
  }, [searchOpen]);

  useEffect(() => {
    let cancelled = false;

    if (!searchOpen) {
      return;
    }

    const query = searchQuery.trim();
    if (query.length < 2) {
      setSearchResults([]);
      setSearchPartial(false);
      setSearchMissingVaultNames([]);
      setSearchParticipants([]);
      setSearchLoading(false);
      setSearchError(null);
      return;
    }

    const id = window.setTimeout(() => {
      void (async () => {
        setSearchLoading(true);
        setSearchError(null);
        try {
          const params = new URLSearchParams({
            q: query,
            mode: searchIncludeContent ? "keyword" : "semantic",
            // The ceiling `clamp_search_limit` allows. One collection-wide
            // budget now covers every Vault where a narrowed scope once had
            // 30 rows to itself, so ask for every row on offer.
            limit: "50",
            per_note_cap: "2",
          });
          const res = await apiFetch(
            `/api/v1/vaults/all/search?${params.toString()}`,
          );
          if (!res.ok) {
            throw new Error(await readErrorMessage(res, "Search failed"));
          }
          const projection =
            (await res.json()) as VaultReadProjection<SearchResponse>;
          if (!cancelled) {
            setSearchResults(projection.data.results);
            setSearchPartial(projection.partial);
            setSearchMissingVaultNames(
              missingVaultNames(projection.participants),
            );
            setSearchParticipants(projection.participants);
          }
        } catch (error) {
          if (!cancelled) {
            setSearchResults([]);
            setSearchPartial(false);
            setSearchMissingVaultNames([]);
            setSearchParticipants([]);
            setSearchError(
              error instanceof Error ? error.message : "Unknown search error",
            );
          }
        } finally {
          if (!cancelled) {
            setSearchLoading(false);
          }
        }
      })();
    }, 150);

    return () => {
      cancelled = true;
      window.clearTimeout(id);
    };
  }, [searchIncludeContent, searchOpen, searchQuery]);

  return {
    searchOpen,
    setSearchOpen,
    searchQuery,
    setSearchQuery,
    searchIncludeContent,
    setSearchIncludeContent,
    searchResults,
    searchPartial,
    searchMissingVaultNames,
    searchParticipants,
    searchInitialVaultFilter,
    searchLoading,
    searchError,
    searchInputRef,
    openSearchForTag,
  };
}

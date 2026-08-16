import { useRef, useState, type ReactNode, type RefObject } from "react";

import {
  StateBlock,
  UiButton,
  UiPanel,
  VaultPrefix,
} from "../../components/ui";
import {
  describeMissingVaults,
  describeNotSearchableVaults,
  notSearchableVaultNames,
} from "../../lib/vaultParticipants";
import type { StartupStatus } from "../../startup/useStartupStatus";
import type {
  VaultId,
  VaultParticipant,
  VaultScope,
  VaultSummary,
} from "../../types";
import type { SearchResult, SearchSelection } from "./types";

type NoteGroup = {
  vault_id: string;
  note_slug: string;
  note_title: string;
  note_path: string;
  chunks: SearchResult[];
};

/** One row of the dialog's own Vault filter (#144) — never the browsing
 * scope. `count: null` means the Vault did not answer this search at all;
 * `0` means it answered with nothing. The two read differently but both
 * keep their row rather than disappearing, per #116's "a missing facet
 * would be an absence; a named one is a fact." */
type FacetRow = { vaultId: VaultId; label: string; count: number | null };

/** Vault-management order, restricted to Vaults that actually participated
 * in this read — never re-sorted by count or condition (#117). */
function buildFacetRows(
  vaults: VaultSummary[],
  participants: VaultParticipant[],
  groups: NoteGroup[],
): FacetRow[] {
  const participantById = new Map(
    participants.map((participant) => [participant.vault_id, participant]),
  );
  const countByVault = new Map<VaultId, number>();
  for (const group of groups) {
    countByVault.set(
      group.vault_id,
      (countByVault.get(group.vault_id) ?? 0) + 1,
    );
  }
  return vaults
    .filter((vault) => participantById.has(vault.vault_id))
    .map((vault) => {
      const answered = participantById.get(vault.vault_id)?.state === "fresh";
      return {
        vaultId: vault.vault_id,
        label: vault.name,
        count: answered ? (countByVault.get(vault.vault_id) ?? 0) : null,
      };
    });
}

const EMPTY_EXPANDED_SLUGS = new Set<string>();

/** Groups by `(vault_id, note_slug)` — a slug is only unique within its own
 * Vault, and duplicate slugs across Vaults must stay distinct groups (#137;
 * full provenance display is #115). */
function groupKey(result: SearchResult): string {
  return `${result.vault_id}:${result.note_slug}`;
}

function groupResults(results: SearchResult[]): NoteGroup[] {
  const map = new Map<string, NoteGroup>();
  for (const r of results) {
    const key = groupKey(r);
    if (!map.has(key)) {
      map.set(key, {
        vault_id: r.vault_id,
        note_slug: r.note_slug,
        note_title: r.note_title,
        note_path: r.note_path,
        chunks: [],
      });
    }
    map.get(key)!.chunks.push(r);
  }
  return Array.from(map.values());
}

function stripMarkdown(raw: string): string {
  return raw
    .replace(/^#{1,6}\s+.*/gm, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/\*([^*]+)\*/g, "$1")
    .replace(/__([^_]+)__/g, "$1")
    .replace(/_([^_]+)_/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/^\s*[-*+]\s+/gm, "")
    .replace(/^\s*\d+\.\s+/gm, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function stripSnippet(raw: string): string {
  const stripped = stripMarkdown(raw);
  if (stripped.length <= 200) return stripped;
  const cut = stripped.slice(0, 200);
  const lastSpace = cut.lastIndexOf(" ");
  return (lastSpace > 150 ? cut.slice(0, lastSpace) : cut) + "…";
}

export function SearchDialog({
  query,
  includeContent,
  loading,
  error,
  results,
  partial,
  missingVaultNames,
  participants,
  initialVaultFilter,
  vaults,
  scope,
  inputRef,
  startupStatus,
  onRetryModelSetup,
  onClose,
  onQueryChange,
  onIncludeContentChange,
  onSelect,
}: {
  query: string;
  includeContent: boolean;
  loading: boolean;
  error: string | null;
  results: SearchResult[];
  /** Whether at least one Vault this search asked did not answer fresh
   * (#141). Never a banner, never a change to ranking. */
  partial: boolean;
  missingVaultNames: string[];
  /** Feeds the dialog's own Vault filter (#144) — never the browsing scope. */
  participants: VaultParticipant[];
  /** Pre-selects a facet from a tag tap; read once, on mount only, since the
   * dialog unmounts on close and remounts fresh on open. */
  initialVaultFilter: VaultId | undefined;
  vaults: VaultSummary[];
  scope: VaultScope;
  inputRef: RefObject<HTMLInputElement | null>;
  /** The shrunk startup gate's own state (#150): a first index in flight or
   * a failed model download replace the result area with a dedicated block
   * instead of blocking the whole app. Typing stays live either way. */
  startupStatus: StartupStatus | null;
  onRetryModelSetup: () => void;
  onClose: () => void;
  onQueryChange: (value: string) => void;
  onIncludeContentChange: (value: boolean) => void;
  onSelect: (selection: SearchSelection) => void;
}) {
  const trimmedQuery = query.trim();
  const startupWorkInFlight =
    startupStatus?.state === "scanning" || startupStatus?.state === "indexing";
  const startupPercent =
    startupStatus?.state === "indexing" ? startupStatus.percent : null;
  const startupFailed = startupStatus?.state === "failed";
  // Provenance only where results can actually span Vaults (#140).
  const showVaultPrefix = scope === "all" && vaults.length > 1;
  const vaultName = (vaultId: string) =>
    vaults.find((vault) => vault.vault_id === vaultId)?.name ?? vaultId;
  const resultsListRef = useRef<HTMLUListElement | null>(null);
  const resultsKey = [
    trimmedQuery,
    ...results.map((result) => `${groupKey(result)}:${result.chunk_id}`),
  ].join("|");
  const [expandedState, setExpandedState] = useState<{
    resultsKey: string;
    slugs: Set<string>;
  }>({ resultsKey: "", slugs: new Set() });
  const expandedSlugs =
    expandedState.resultsKey === resultsKey
      ? expandedState.slugs
      : EMPTY_EXPANDED_SLUGS;

  function toggleExpanded(slug: string) {
    setExpandedState((prev) => {
      const next = new Set(
        prev.resultsKey === resultsKey ? prev.slugs : EMPTY_EXPANDED_SLUGS,
      );
      if (next.has(slug)) {
        next.delete(slug);
      } else {
        next.add(slug);
      }
      return { resultsKey, slugs: next };
    });
  }

  const groups = groupResults(results);

  // The dialog's own filter (#144) — a lens over the answer in front of you,
  // never the browsing scope. Local state: it dies when the dialog closes,
  // because App.tsx only mounts <SearchDialog> while searchOpen is true, so
  // every open starts fresh unless a tag tap pre-selected a Vault.
  const [vaultFilter, setVaultFilter] = useState<VaultId | "all">(
    () => initialVaultFilter ?? "all",
  );
  const facetRows = buildFacetRows(vaults, participants, groups);
  // Two different reasons a semantic search came back partial, told apart
  // because they ask different things of the reader: a Vault that did not
  // answer may need attention, while one still building search only needs
  // time. Both are named when both apply.
  const pendingSearchNames = notSearchableVaultNames(participants);
  const partialSentence = [
    missingVaultNames.length > 0
      ? describeMissingVaults(missingVaultNames)
      : null,
    pendingSearchNames.length > 0
      ? describeNotSearchableVaults(pendingSearchNames)
      : null,
  ]
    .filter(Boolean)
    .join(" ");
  const showFacetRail = scope === "all" && vaults.length > 1;
  const showScopeField = vaults.length > 1;
  const visibleGroups =
    vaultFilter === "all"
      ? groups
      : groups.filter((group) => group.vault_id === vaultFilter);
  const filteredToEmpty =
    vaultFilter !== "all" && groups.length > 0 && visibleGroups.length === 0;
  const filterLabel =
    vaultFilter === "all"
      ? null
      : (facetRows.find((row) => row.vaultId === vaultFilter)?.label ??
        vaultName(vaultFilter));

  return (
    <div
      className="search-overlay"
      role="dialog"
      aria-modal="true"
      aria-label="Search notes"
      onClick={onClose}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          onClose();
        }
      }}
    >
      <UiPanel
        className={`search-panel${showFacetRail ? " search-panel--faceted" : ""}`}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key !== "Tab") return;
          const panel = event.currentTarget;
          const focusable = Array.from(
            panel.querySelectorAll<HTMLElement>(
              "button:not([disabled]), input:not([disabled]), select:not([disabled])",
            ),
          );
          if (focusable.length === 0) return;
          const first = focusable[0];
          const last = focusable[focusable.length - 1];
          if (event.shiftKey) {
            if (document.activeElement === first) {
              event.preventDefault();
              last.focus();
            }
          } else {
            if (document.activeElement === last) {
              event.preventDefault();
              first.focus();
            }
          }
        }}
      >
        <header className="search-header">
          <h2>Search</h2>
          <UiButton className="close-note" onClick={onClose}>
            Close
          </UiButton>
        </header>

        <input
          ref={inputRef}
          className="search-input"
          placeholder="Search notes…"
          autoFocus
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown" && results.length > 0) {
              event.preventDefault();
              resultsListRef.current
                ?.querySelector<HTMLButtonElement>("button")
                ?.focus();
            }
          }}
        />

        {/* Desktop: the documented dialog's own Mode toggle, unchanged.
            Hidden below 920px, where the field strip below carries Mode
            instead — same filter, same semantics, different form (#119,
            #144). */}
        <label className="search-toggle">
          <input
            type="checkbox"
            checked={includeContent}
            onChange={(event) => onIncludeContentChange(event.target.checked)}
          />
          Keyword mode
        </label>

        {/* Phone: Scope beside Mode, in one field strip under the input
            (#119, #144). Scope only where there is anything to filter
            across; it stays even when the sidebar's browsing scope is
            narrowed, because the panel covers the Scope zone here and the
            current scope must stay visible some other way. */}
        <div className="search-field-strip">
          {showScopeField ? (
            <div className="field">
              <label className="field-label" htmlFor="search-scope-field">
                Scope
              </label>
              <select
                id="search-scope-field"
                className="field-input"
                value={vaultFilter}
                onChange={(event) => setVaultFilter(event.target.value)}
              >
                <option value="all">All results</option>
                {facetRows.map((row) => (
                  <option
                    key={row.vaultId}
                    value={row.vaultId}
                    disabled={row.count === null}
                  >
                    {row.count === null
                      ? `${row.label} (no answer)`
                      : row.label}
                  </option>
                ))}
              </select>
            </div>
          ) : null}
          <div className="field">
            <label className="field-label" htmlFor="search-mode-field">
              Mode
            </label>
            <select
              id="search-mode-field"
              className="field-input"
              value={includeContent ? "keyword" : "semantic"}
              onChange={(event) =>
                onIncludeContentChange(event.target.value === "keyword")
              }
            >
              <option value="semantic">Semantic</option>
              <option value="keyword">Keyword</option>
            </select>
          </div>
        </div>

        {startupWorkInFlight ? (
          // The shrunk startup gate (#150) no longer blocks the app for a
          // first index; this dialog still opens normally and stays typable
          // — it just can't answer yet, worded as work in flight and
          // carrying the same percentage the Scope zone shows.
          <StateBlock
            title="Could Not Load"
            description={
              startupPercent === null
                ? "Building the search index. Results will appear once it's ready."
                : `Building the search index (${startupPercent}%). Results will appear once it's ready.`
            }
          />
        ) : startupFailed ? (
          <StateBlock
            tone="error"
            title="Could Not Load"
            description={
              (startupStatus?.state === "failed" && startupStatus.message) ||
              "The search model could not be downloaded or loaded."
            }
            actionLabel="Retry setup"
            onAction={onRetryModelSetup}
          />
        ) : (
          <>
            {loading ? <p>Searching…</p> : null}
            {error ? <p className="error">{error}</p> : null}
            {!loading &&
            !error &&
            trimmedQuery.length >= 2 &&
            results.length === 0 ? (
              partial ? (
                // Nothing usable: the documented error block replaces the
                // empty state entirely. "No matching notes" would be a lie
                // when some Vaults never answered (#141).
                <StateBlock
                  tone="error"
                  title="Nothing Found"
                  description={partialSentence}
                />
              ) : (
                <p>No matching notes.</p>
              )
            ) : null}
          </>
        )}

        <div className="search-body">
          {/* Desktop: the facet rail, a narrow column beside the results
              (#119, #144). Absent when narrowed or at one enabled Vault —
              there is nothing to filter across, and the Scope zone is on
              screen behind the overlay saying what the scope is. */}
          {showFacetRail ? (
            <div className="search-facet-rail" aria-label="Filter by Vault">
              <button
                type="button"
                className={`search-facet-row${vaultFilter === "all" ? " is-selected" : ""}`}
                onClick={() => setVaultFilter("all")}
              >
                <span className="search-facet-label">All results</span>
                <span className="side-count">{groups.length}</span>
              </button>
              {facetRows.map((row) => (
                <button
                  key={row.vaultId}
                  type="button"
                  className={`search-facet-row${vaultFilter === row.vaultId ? " is-selected" : ""}`}
                  aria-disabled={row.count === null}
                  onClick={() => {
                    if (row.count !== null) {
                      setVaultFilter(row.vaultId);
                    }
                  }}
                >
                  <span className="search-facet-label">{row.label}</span>
                  {row.count === null ? (
                    <span className="vault-slot-condition vault-tier-error">
                      no answer
                    </span>
                  ) : (
                    <span className="side-count">{row.count}</span>
                  )}
                </button>
              ))}
            </div>
          ) : null}

          <div className="search-main">
            <ul
              ref={resultsListRef}
              className="search-results"
              onKeyDown={(event) => {
                if (event.key !== "ArrowDown" && event.key !== "ArrowUp")
                  return;
                event.preventDefault();
                const list = resultsListRef.current;
                if (!list) return;
                const buttons = Array.from(
                  list.querySelectorAll<HTMLButtonElement>("button"),
                );
                const idx = buttons.indexOf(
                  document.activeElement as HTMLButtonElement,
                );
                if (event.key === "ArrowDown") {
                  (buttons[idx + 1] ?? buttons[0])?.focus();
                } else {
                  if (idx <= 0) {
                    inputRef.current?.focus();
                  } else {
                    buttons[idx - 1].focus();
                  }
                }
              }}
            >
              {visibleGroups.map((group) => {
                const [first, ...rest] = group.chunks;
                const key = groupKey(first);
                const isExpanded = expandedSlugs.has(key);
                const hiddenCount = rest.length;

                return (
                  <li key={key} className="search-group">
                    <button
                      type="button"
                      className="search-result search-result--primary"
                      onClick={() =>
                        onSelect({
                          vaultId: first.vault_id,
                          slug: first.note_slug,
                          query: trimmedQuery,
                          matchKind: first.heading_path ?? "",
                        })
                      }
                    >
                      <div className="result-title">
                        {highlightMatches(group.note_title, trimmedQuery)}
                      </div>
                      <div className="result-path">
                        {showVaultPrefix ? (
                          <VaultPrefix name={vaultName(group.vault_id)} />
                        ) : null}
                        <span className="result-path-text">
                          {highlightMatches(
                            `${group.note_path}.md`,
                            trimmedQuery,
                          )}
                        </span>
                      </div>
                      {first.heading_path ? (
                        <div className="result-breadcrumb">
                          {first.heading_path}
                        </div>
                      ) : null}
                      <p className="result-snippet">
                        {highlightMatches(
                          stripSnippet(first.content),
                          trimmedQuery,
                        )}
                      </p>
                    </button>

                    {isExpanded
                      ? rest.map((chunk) => (
                          <button
                            key={chunk.chunk_id}
                            type="button"
                            className="search-result search-result--chunk"
                            onClick={() =>
                              onSelect({
                                vaultId: chunk.vault_id,
                                slug: chunk.note_slug,
                                query: trimmedQuery,
                                matchKind: chunk.heading_path ?? "",
                              })
                            }
                          >
                            {chunk.heading_path ? (
                              <div className="result-breadcrumb">
                                {chunk.heading_path}
                              </div>
                            ) : null}
                            <p className="result-snippet">
                              {highlightMatches(
                                stripSnippet(chunk.content),
                                trimmedQuery,
                              )}
                            </p>
                          </button>
                        ))
                      : null}

                    {hiddenCount > 0 ? (
                      <button
                        type="button"
                        className="search-group-toggle"
                        onClick={() => toggleExpanded(key)}
                      >
                        {isExpanded
                          ? "Show less"
                          : `${hiddenCount} more section${hiddenCount > 1 ? "s" : ""}`}
                      </button>
                    ) : null}
                  </li>
                );
              })}
            </ul>
            {/* The filter narrowed the view to nothing, distinct from #141's
            "nothing usable at all" — the Vault answered, it just has no
            matches. Not a banner, not an error: the facet's own "0" already
            said this before the click. */}
            {filteredToEmpty ? (
              <p className="search-facet-empty">No results in {filterLabel}.</p>
            ) : null}
            {/* Ranking is unchanged by partiality; this trailing line below the
            last row is the only thing that changes (#141). */}
            {partial && results.length > 0 ? (
              <p className="search-partial">{partialSentence}</p>
            ) : null}
          </div>
        </div>
      </UiPanel>
    </div>
  );
}

function highlightMatches(text: string, query: string): ReactNode {
  if (!query) {
    return text;
  }

  const escaped = escapeRegExp(query);
  const regex = new RegExp(`(${escaped})`, "ig");
  const parts = text.split(regex);

  if (parts.length <= 1) {
    return text;
  }

  const queryLower = query.toLowerCase();
  return parts.map((part, index) =>
    part.toLowerCase() === queryLower ? (
      <mark key={index} className="search-match">
        {part}
      </mark>
    ) : (
      <span key={index}>{part}</span>
    ),
  );
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

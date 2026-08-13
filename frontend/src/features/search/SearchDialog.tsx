import { useRef, useState, type ReactNode, type RefObject } from "react";

import { StateBlock, UiButton, UiPanel, VaultPrefix } from "../../components/ui";
import { describeMissingVaults } from "../../lib/vaultParticipants";
import type { VaultScope, VaultSummary } from "../../types";
import type { SearchResult, SearchSelection } from "./types";

type NoteGroup = {
  vault_id: string;
  note_slug: string;
  note_title: string;
  note_path: string;
  chunks: SearchResult[];
};

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
  vaults,
  scope,
  inputRef,
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
  vaults: VaultSummary[];
  scope: VaultScope;
  inputRef: RefObject<HTMLInputElement | null>;
  onClose: () => void;
  onQueryChange: (value: string) => void;
  onIncludeContentChange: (value: boolean) => void;
  onSelect: (selection: SearchSelection) => void;
}) {
  const trimmedQuery = query.trim();
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
        className="search-panel"
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key !== "Tab") return;
          const panel = event.currentTarget;
          const focusable = Array.from(
            panel.querySelectorAll<HTMLElement>(
              "button:not([disabled]), input:not([disabled])",
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

        <label className="search-toggle">
          <input
            type="checkbox"
            checked={includeContent}
            onChange={(event) => onIncludeContentChange(event.target.checked)}
          />
          Keyword mode
        </label>

        {loading ? <p>Searching…</p> : null}
        {error ? <p className="error">{error}</p> : null}
        {!loading && !error && trimmedQuery.length >= 2 && results.length === 0 ? (
          partial ? (
            // Nothing usable: the documented error block replaces the empty
            // state entirely. "No matching notes" would be a lie when some
            // Vaults never answered (#141).
            <StateBlock
              tone="error"
              title="Nothing Found"
              description={describeMissingVaults(missingVaultNames)}
            />
          ) : (
            <p>No matching notes.</p>
          )
        ) : null}

        <ul
          ref={resultsListRef}
          className="search-results"
          onKeyDown={(event) => {
            if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
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
          {groups.map((group) => {
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
        {/* Ranking is unchanged by partiality; this trailing line below the
            last row is the only thing that changes (#141). */}
        {partial && results.length > 0 ? (
          <p className="search-partial">
            {describeMissingVaults(missingVaultNames)}
          </p>
        ) : null}
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

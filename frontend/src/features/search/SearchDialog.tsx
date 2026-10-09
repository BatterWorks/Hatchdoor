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
 * scope. A `number` is how many notes that Vault put in this answer, `0`
 * included: it answered with nothing. `"no-answer"` is a Vault that was asked
 * and did not answer. `"unasked"` is every Vault before a search has run at
 * all, which has no count to give and no fault to report. All three keep
 * their row rather than disappearing, per #116's "a missing facet would be an
 * absence; a named one is a fact." */
type FacetRow = {
  vaultId: VaultId;
  label: string;
  count: number | "no-answer" | "unasked";
};

/** Vault-management order, never re-sorted by count or condition (#117).
 * Once a search has run the rows are the Vaults that participated in it;
 * before one has, they are simply every Vault, so the rail is a selector from
 * the moment the dialog opens rather than a column that fills in later. */
function buildFacetRows(
  vaults: VaultSummary[],
  participants: VaultParticipant[],
  groups: NoteGroup[],
  searchAnswered: boolean,
): FacetRow[] {
  if (!searchAnswered) {
    return vaults.map((vault) => ({
      vaultId: vault.vault_id,
      label: vault.name,
      count: "unasked" as const,
    }));
  }
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
        count: answered
          ? (countByVault.get(vault.vault_id) ?? 0)
          : ("no-answer" as const),
      };
    });
}

/** Whether `element` takes part in layout, checked against its computed
 * style up to `boundary` rather than its box, so the answer is the same in
 * a browser and in a DOM without layout. */
function isRendered(element: HTMLElement, boundary: HTMLElement): boolean {
  for (
    let node: HTMLElement | null = element;
    node && node !== boundary.parentElement;
    node = node.parentElement
  ) {
    const style = window.getComputedStyle(node);
    if (style.display === "none" || style.visibility === "hidden") {
      return false;
    }
  }
  return true;
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
  return (
    raw
      // Frontmatter at the top of a chunk, and any fenced block whole: a
      // snippet that showed a Mermaid diagram's source read as noise (#530).
      // An unterminated fence is cut from its opening to the end.
      .replace(/^---\r?\n[\s\S]*?\r?\n---(?:\r?\n|$)/, "")
      .replace(/```[\s\S]*?(?:```|$)/g, "")
      .replace(/^#{1,6}\s+.*/gm, "")
      // A wikilink reads by its alias where it has one, else its target.
      .replace(/!?\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_, target, alias) =>
        String(alias ?? target),
      )
      .replace(/\*\*([^*]+)\*\*/g, "$1")
      .replace(/\*([^*]+)\*/g, "$1")
      .replace(/__([^_]+)__/g, "$1")
      .replace(/_([^_]+)_/g, "$1")
      .replace(/`([^`]+)`/g, "$1")
      .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
      .replace(/^\s*[-*+]\s+/gm, "")
      .replace(/^\s*\d+\.\s+/gm, "")
      .replace(/\n{3,}/g, "\n\n")
      .trim()
  );
}

function stripSnippet(raw: string): string {
  const stripped = stripMarkdown(raw);
  if (stripped.length <= 200) return stripped;
  const cut = stripped.slice(0, 200);
  const lastSpace = cut.lastIndexOf(" ");
  return (lastSpace > 150 ? cut.slice(0, lastSpace) : cut) + "…";
}

/** What a demo visitor reads when search's model is missing or failed: no
 * operator diagnostic and nothing to act on (#339), the same posture
 * `vaultSlotLogic.ts` takes for Vault conditions in demo mode (#152). */
const DEMO_SEARCH_UNAVAILABLE = "Search is unavailable on this demo right now.";

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
  demoMode = false,
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
  /** Pre-selects a facet from a tag tap, overriding the browsing scope's own
   * pre-selection; read once, on mount only, since the dialog unmounts on
   * close and remounts fresh on open. */
  initialVaultFilter: VaultId | undefined;
  vaults: VaultSummary[];
  /** The browsing scope, which the Vault filter opens on and the reader can
   * then widen or move. It no longer narrows the search itself — the fetch is
   * always collection-wide (see `useSearch`). */
  scope: VaultScope;
  inputRef: RefObject<HTMLInputElement | null>;
  /** The shrunk startup gate's own state (#150): a first index in flight or
   * a failed model download replace the result area with a dedicated block
   * instead of blocking the whole app. Typing stays live either way. */
  startupStatus: StartupStatus | null;
  onRetryModelSetup: () => void;
  /** A public demo (#339): model-setup trouble is described in a visitor's
   * words, never the server's operator diagnostic, and offers no retry — the
   * server 404s `/api/model/retry` in demo mode, so the button could never
   * do anything. */
  demoMode?: boolean;
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
  const startupDownloading = startupStatus?.state === "downloading";
  const startupDownloadPercent =
    startupStatus?.state === "downloading" &&
    startupStatus.percent !== undefined
      ? startupStatus.percent
      : null;
  const startupTermsRequired = startupStatus?.state === "terms_required";
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
  // every open starts fresh.
  //
  // It opens on the browsing scope so a reader who narrowed the sidebar sees
  // what they expected to see, and the rail beside it shows the Vaults they
  // did not narrow to, with counts, one click away. A tag tap wins over the
  // scope: it names the Vault the tag was read in.
  //
  // Both are filtered through the enabled Vaults first. `useVaultScope` only
  // reconciles the browsing scope once discovery has answered (#335), and a
  // tag tap can name any Vault, so a Vault disabled since it was last browsed
  // can still arrive here as a stale id. Seeding on that would open the dialog
  // filtered to a Vault with no row to click, with nothing selected and the
  // raw id rendered as a name. All results is the honest fallback.
  const [chosenVaultFilter, setVaultFilter] = useState<VaultId | "all">(() => {
    const preferred = initialVaultFilter ?? scope;
    return vaults.some((vault) => vault.vault_id === preferred)
      ? preferred
      : "all";
  });
  // The seed above runs once, but the collection can lose a Vault while the
  // dialog is open. Read the choice through the live collection on every
  // render, so a departed Vault never leaves the control reading "All
  // results" while it still suppresses every result (#334).
  const vaultFilter: VaultId | "all" =
    chosenVaultFilter === "all" ||
    vaults.some((vault) => vault.vault_id === chosenVaultFilter)
      ? chosenVaultFilter
      : "all";
  // No Vault has been asked yet — the query is still too short, or the first
  // answer has not landed. Counts would all read `0`, which is a claim about
  // the collection rather than about this search.
  const searchAnswered = participants.length > 0;
  const facetRows = buildFacetRows(
    vaults,
    participants,
    groups,
    searchAnswered,
  );
  // Two different reasons a semantic search came back partial, told apart
  // because they ask different things of the reader: a Vault that did not
  // answer may need attention, while one whose notes semantic search could
  // not reach is answered by Keyword search. Both are named when both apply.
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
  // One condition, both shapes: the rail on desktop and the phone's Scope
  // field are the same filter, so they appear and disappear together.
  // Shown wherever there is more than one Vault to tell apart, narrowed
  // browsing scope included. Hiding it at a narrowed scope was the whole bug:
  // the one state where the reader most needs to know their search is pinned
  // was the one state that said nothing.
  const showVaultFilter = vaults.length > 1;
  // Provenance only where the visible rows can actually span Vaults (#140).
  const showVaultPrefix = vaultFilter === "all" && vaults.length > 1;
  const visibleGroups =
    vaultFilter === "all"
      ? groups
      : groups.filter((group) => group.vault_id === vaultFilter);
  const filteredRow =
    vaultFilter === "all"
      ? null
      : (facetRows.find((row) => row.vaultId === vaultFilter) ?? null);
  // The browsing scope can seed the filter onto a Vault that then fails to
  // answer, which is the one case where the filter's empty list is not a fact
  // about the Vault's contents. "No results in Beta" would be #141's exact
  // lie; the row's own `no answer` and the partial sentence already say what
  // actually happened, so say nothing more here.
  const filteredToNoAnswer = filteredRow?.count === "no-answer";
  const filteredToEmpty =
    vaultFilter !== "all" &&
    !filteredToNoAnswer &&
    groups.length > 0 &&
    visibleGroups.length === 0;
  const filterLabel =
    vaultFilter === "all"
      ? null
      : (filteredRow?.label ?? vaultName(vaultFilter));

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
        className={`search-panel${showVaultFilter ? " search-panel--faceted" : ""}`}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key !== "Tab") return;
          const panel = event.currentTarget;
          // Only controls that are actually rendered: the phone field strip
          // is always in the DOM but `display: none` on desktop, and a hidden
          // last stop meant the wrap never fired and Tab left the modal
          // (#334).
          const focusable = Array.from(
            panel.querySelectorAll<HTMLElement>(
              "button:not([disabled]), input:not([disabled]), select:not([disabled])",
            ),
          ).filter((element) => isRendered(element, panel));
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

        {/* One Mode control at every width (#530): a two-way segmented
            choice, the same control Settings uses for a Vault's Git
            behaviour. It replaced a desktop checkbox and a phone select that
            said the same thing in two shapes. */}
        <div className="search-mode-row">
          <span className="search-mode-label" id="search-mode-label">
            Mode
          </span>
          <div
            className="search-mode"
            role="group"
            aria-labelledby="search-mode-label"
          >
            <button
              type="button"
              aria-pressed={!includeContent}
              onClick={() => onIncludeContentChange(false)}
            >
              Semantic
            </button>
            <button
              type="button"
              aria-pressed={includeContent}
              onClick={() => onIncludeContentChange(true)}
            >
              Keyword
            </button>
          </div>
        </div>

        {/* Phone: Scope as a field under the input (#119, #144). The rail
            has no room as a column here, so it takes this shape instead —
            same filter, same semantics. It opens on the browsing scope like
            the rail does, and like the rail it is the filter from the first
            change onwards, never the browsing scope itself. */}
        {showVaultFilter ? (
          <div className="search-field-strip">
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
                    disabled={row.count === "no-answer"}
                  >
                    {row.count === "no-answer"
                      ? `${row.label} (no answer)`
                      : row.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
        ) : null}

        {/* Work in flight is progress, not failure (#530): one quiet line
            under the controls, never a block that outranks results. The
            shrunk startup gate (#150) no longer blocks the app for a first
            index, and Keyword search answers from the text index long
            before the vectors land, so results and this line coexist. In
            semantic mode with nothing to show, "No matching notes" stays
            suppressed below: nothing has been searched yet. */}
        {startupWorkInFlight ? (
          <SearchProgress
            percent={startupPercent}
            keyword={includeContent}
            lead={
              startupStatus?.state === "scanning"
                ? "Search is still scanning your notes."
                : startupPercent === null
                  ? "Semantic search is still indexing."
                  : `Semantic search is still indexing, ${startupPercent}% done.`
            }
          />
        ) : startupDownloading ? (
          // A model download after the gate has stepped aside, most often
          // the one "Retry setup" starts (#339). Semantic search cannot
          // answer until it lands; Keyword still can.
          <SearchProgress
            percent={startupDownloadPercent}
            keyword={includeContent}
            lead={
              startupDownloadPercent === null
                ? "Downloading the search model."
                : `Downloading the search model (${startupDownloadPercent}%).`
            }
          />
        ) : null}

        {/* A model that is missing or failed is a genuine failure and keeps
            its block, unless Keyword results are on screen, where it shrinks
            to the same quiet line so it never sits above a live answer. */}
        {startupTermsRequired || startupFailed ? (
          results.length > 0 ? (
            <p className="search-progress" role="status">
              {demoMode
                ? DEMO_SEARCH_UNAVAILABLE
                : startupTermsRequired
                  ? "Semantic search is waiting for a search model to be chosen. Keyword search answers now."
                  : "The search model could not be loaded, so only Keyword search answers."}
            </p>
          ) : startupTermsRequired ? (
            <StateBlock
              title="Could Not Load"
              description={
                demoMode
                  ? DEMO_SEARCH_UNAVAILABLE
                  : "Search is waiting for a search model to be chosen. Reload the page to choose one."
              }
            />
          ) : demoMode ? (
            <StateBlock
              title="Could Not Load"
              description={DEMO_SEARCH_UNAVAILABLE}
            />
          ) : (
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
          )
        ) : null}

        {startupTermsRequired || startupFailed ? null : (
          <>
            {loading ? <p>Searching…</p> : null}
            {error ? <p className="error">{error}</p> : null}
            {!loading &&
            !error &&
            trimmedQuery.length >= 2 &&
            results.length === 0 &&
            // Semantic search has not answered anything yet while the model
            // is still arriving; only Keyword can claim "no matches" then.
            (includeContent || !(startupWorkInFlight || startupDownloading)) ? (
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
              (#119, #144). Absent only at one enabled Vault, where there is
              nothing to filter across. */}
          {showVaultFilter ? (
            <div className="search-facet-rail" aria-label="Filter by Vault">
              <button
                type="button"
                className={`search-facet-row${vaultFilter === "all" ? " is-selected" : ""}`}
                onClick={() => setVaultFilter("all")}
              >
                <span className="search-facet-label">All results</span>
                {searchAnswered ? (
                  <span className="side-count">{groups.length}</span>
                ) : null}
              </button>
              {facetRows.map((row) => (
                <button
                  key={row.vaultId}
                  type="button"
                  className={`search-facet-row${vaultFilter === row.vaultId ? " is-selected" : ""}`}
                  aria-disabled={row.count === "no-answer"}
                  onClick={() => {
                    if (row.count !== "no-answer") {
                      setVaultFilter(row.vaultId);
                    }
                  }}
                >
                  <span className="search-facet-label">{row.label}</span>
                  {row.count === "no-answer" ? (
                    // Amber, not red: the sidebar already carries the
                    // Vault's own condition in its tier; inside a filter the
                    // word is a fact about this answer, not an alarm (#530).
                    <span className="vault-slot-condition vault-tier-warn">
                      no answer
                    </span>
                  ) : row.count === "unasked" ? null : (
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
                      {/* A chunk under the note's own H1 has a heading path
                          equal to the title; repeating it said nothing
                          (#530). */}
                      {first.heading_path &&
                      first.heading_path !== group.note_title ? (
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

/** Work in flight, worded as progress (#530): a bar for the percentage and
 * one sentence saying which mode already answers. Scanning builds the text
 * index itself, so only an index past scanning can promise Keyword. */
function SearchProgress({
  lead,
  percent,
  keyword,
}: {
  lead: string;
  percent: number | null;
  keyword: boolean;
}) {
  const scanning = lead.startsWith("Search is still scanning");
  return (
    <p className="search-progress" role="status">
      <span className="search-progress-bar" aria-hidden="true">
        <span
          className="search-progress-fill"
          style={{ width: `${percent ?? 0}%` }}
        />
      </span>
      <span>
        {lead}{" "}
        {scanning
          ? "Results fill in as it goes."
          : keyword
            ? "Keyword search answers now."
            : "Keyword search answers now; switch to it for exact words."}
      </span>
    </p>
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

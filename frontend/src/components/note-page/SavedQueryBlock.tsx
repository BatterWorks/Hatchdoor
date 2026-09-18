import { useContext, useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";

import type { VaultId } from "../../types";
import { CodeBlock } from "./RendererComponents";
import {
  SavedQueryContext,
  baseFenceLines,
  formatCell,
  linkText,
  markerNoticesFor,
  matchResult,
  nextSort,
  sortRows,
  type SavedQueryColumn,
  type SavedQueryRow,
  type SavedQueryState,
  type SortState,
} from "./savedQueries";

export function SavedQueryProvider({
  state,
  vaultId,
  markdown,
  children,
}: {
  state: SavedQueryState;
  vaultId: VaultId;
  /** The Markdown being rendered, whose line numbers the blocks report. */
  markdown: string;
  children: ReactNode;
}) {
  const fenceLines = useMemo(() => baseFenceLines(markdown), [markdown]);
  const value = useMemo(
    () => ({ state, vaultId, fenceLines }),
    [state, vaultId, fenceLines],
  );
  return (
    <SavedQueryContext.Provider value={value}>
      {children}
    </SavedQueryContext.Provider>
  );
}

/**
 * What a `base` block renders as. Outside a provider, which is the editor's
 * preview, the definition shows as the code it is: the preview draws unsaved
 * text, and a saved query is only evaluated from the note on disk.
 */
export function SavedQueryBlock({
  source,
  line,
}: {
  source: string;
  /** The line the block's opening fence sits on, when the renderer knows it. */
  line?: number;
}) {
  const context = useContext(SavedQueryContext);
  if (!context) {
    return <CodeBlock language="base" content={source} />;
  }

  const { state, vaultId, fenceLines } = context;
  if (state.status === "error") {
    return (
      <SavedQueryFrame>
        <p className="saved-query-note">
          Saved query results could not be loaded: {state.message}
        </p>
      </SavedQueryFrame>
    );
  }

  const result =
    state.status === "none"
      ? undefined
      : matchResult(state.results, source, line, fenceLines);

  if (!result || state.status === "none") {
    return (
      <SavedQueryFrame>
        <p className="saved-query-note">
          {state.status === "loading"
            ? "Evaluating saved query…"
            : "This saved query has not been evaluated."}
        </p>
      </SavedQueryFrame>
    );
  }

  const notices = markerNoticesFor(
    state.markerProblems,
    state.results.indexOf(result),
  );

  if (result.status === "refused") {
    return (
      <SavedQueryFrame notices={notices}>
        <p className="saved-query-note" role="status">
          <strong>Not evaluated.</strong> {result.message}
        </p>
      </SavedQueryFrame>
    );
  }

  if (result.status === "stopped") {
    return (
      <SavedQueryFrame notices={notices}>
        <p className="saved-query-note" role="status">
          <strong>Stopped.</strong> {result.message}
        </p>
      </SavedQueryFrame>
    );
  }

  const ignored = (result.ignored ?? []).map((gap) => gap.message);

  if (result.status === "empty") {
    return (
      <SavedQueryFrame
        caption={result.view_name}
        notices={[...ignored, ...notices]}
      >
        <p className="saved-query-note" role="status">
          <strong>No matches.</strong> This saved query was read and checked
          against every note in this Vault, and none qualifies right now.
        </p>
      </SavedQueryFrame>
    );
  }

  return (
    <SavedQueryFrame
      caption={result.view_name}
      notices={[...ignored, ...notices]}
    >
      <SortableRows
        columns={result.columns}
        rows={result.rows}
        vaultId={vaultId}
      />
      {result.truncated ? (
        <p className="saved-query-note" role="status">
          {result.truncated.reason === "definition_limit"
            ? `Showing the first ${result.truncated.shown} notes, the limit this saved query sets.`
            : `Truncated: showing the first ${result.truncated.shown} notes. Hatchdoor shows no more than that from one saved query, and more qualify.`}
        </p>
      ) : null}
    </SavedQueryFrame>
  );
}

/**
 * The table itself. A click on a heading re-sorts the rows on screen and
 * nowhere else: the choice lives in this component's state, so it is never
 * written to the note and a reload forgets it, and each table sorts on its own.
 */
function SortableRows({
  columns,
  rows,
  vaultId,
}: {
  columns: SavedQueryColumn[];
  rows: SavedQueryRow[];
  vaultId: VaultId;
}) {
  const [sort, setSort] = useState<SortState | null>(null);
  const sorted = useMemo(() => sortRows(rows, sort), [rows, sort]);
  const linkColumn = Math.max(
    columns.findIndex(
      (column) => column.id === "file.name" || column.id === "file.basename",
    ),
    0,
  );

  return (
    <table>
      <thead>
        <tr>
          {columns.map((column, index) => {
            const direction =
              sort?.column === index ? sort.direction : undefined;
            return (
              <th key={column.id} aria-sort={direction ?? "none"}>
                <button
                  type="button"
                  className="saved-query-sort"
                  onClick={() => setSort((current) => nextSort(current, index))}
                >
                  {column.label}
                  <span aria-hidden="true">
                    {direction === "ascending"
                      ? "↑"
                      : direction === "descending"
                        ? "↓"
                        : ""}
                  </span>
                </button>
              </th>
            );
          })}
        </tr>
      </thead>
      <tbody>
        {sorted.map((row) => (
          <tr key={`${row.vault_id}:${row.slug}`}>
            {columns.map((column, index) => (
              <td key={column.id}>
                {index === linkColumn ? (
                  <Link
                    to={`/v/${encodeURIComponent(vaultId)}/n/${encodeURIComponent(row.slug)}`}
                  >
                    {linkText(column.id, row.cells[index], row.title)}
                  </Link>
                ) : (
                  formatCell(row.cells[index])
                )}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/**
 * Where a `hatchdoor-query` marker with no `base` block after it sits. It
 * shows the server's notice about that marker and nothing else; outside a
 * provider, or before the server has answered, it shows nothing.
 */
export function OrphanedMarkerNotice({ name }: { name?: string }) {
  const context = useContext(SavedQueryContext);
  const state = context?.state;
  if (!state || (state.status !== "ready" && state.status !== "loading")) {
    return null;
  }
  const problem = state.markerProblems.find(
    (candidate) =>
      candidate.problem === "orphaned" && candidate.name === (name ?? ""),
  );
  if (!problem) {
    return null;
  }
  return (
    <p className="saved-query-orphan" role="status">
      {problem.message}
    </p>
  );
}

function SavedQueryFrame({
  caption,
  notices,
  children,
}: {
  caption?: string;
  /** What the server set aside without changing the rows: an ignored
   * presentation instruction, or a problem with the block's name. */
  notices?: string[];
  children: ReactNode;
}) {
  return (
    <div className="table-wrap saved-query">
      <div className="saved-query-head">
        <span>{caption ?? "Saved query"}</span>
      </div>
      {children}
      {notices?.map((notice) => (
        <p key={notice} className="saved-query-note">
          {notice}
        </p>
      ))}
    </div>
  );
}

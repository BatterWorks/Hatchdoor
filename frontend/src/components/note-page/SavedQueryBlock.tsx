import { useContext, useMemo, type ReactNode } from "react";
import { Link } from "react-router-dom";

import type { VaultId } from "../../types";
import { CodeBlock } from "./RendererComponents";
import {
  SavedQueryContext,
  baseFenceLines,
  formatCell,
  linkText,
  matchResult,
  type SavedQueryState,
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

  if (!result) {
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

  if (result.status === "refused") {
    return (
      <SavedQueryFrame notices={result.notices}>
        <p className="saved-query-note" role="status">
          Not evaluated. {result.message}
        </p>
      </SavedQueryFrame>
    );
  }

  if (result.status === "stopped") {
    return (
      <SavedQueryFrame notices={result.notices}>
        <p className="saved-query-note" role="status">
          Stopped. {result.message}
        </p>
      </SavedQueryFrame>
    );
  }

  const linkColumn = Math.max(
    result.columns.findIndex(
      (column) => column.id === "file.name" || column.id === "file.basename",
    ),
    0,
  );

  return (
    <SavedQueryFrame caption={result.view_name} notices={result.notices}>
      {result.rows.length === 0 ? (
        <p className="saved-query-note">No notes match this saved query.</p>
      ) : (
        <table>
          <thead>
            <tr>
              {result.columns.map((column) => (
                <th key={column.id}>{column.label}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {result.rows.map((row) => (
              <tr key={`${row.vault_id}:${row.slug}`}>
                {result.columns.map((column, index) => (
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
      )}
      {result.truncated ? (
        <p className="saved-query-note">
          {result.truncated.reason === "definition_limit"
            ? `Showing the first ${result.truncated.shown} notes, the limit this saved query sets.`
            : `Showing the first ${result.truncated.shown} notes. Hatchdoor shows no more than that from one saved query.`}
        </p>
      ) : null}
    </SavedQueryFrame>
  );
}

function SavedQueryFrame({
  caption,
  notices,
  children,
}: {
  caption?: string;
  /** What the server set aside without changing the rows, such as a name. */
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

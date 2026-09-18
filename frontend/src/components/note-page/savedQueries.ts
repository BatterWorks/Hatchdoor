import { createContext, useEffect, useRef, useState } from "react";

import { apiFetch } from "../../api/api";
import type { VaultId, VaultReadProjection } from "../../types";

// Saved queries (#275, ADR-21): a note's fenced `base` blocks, evaluated by the
// server against the note's own Vault on every read. The note read returns the
// Markdown and nothing else, so the rows arrive through a separate request and
// are drawn in place of each block. Nothing here is ever written back.

export type SavedQueryColumn = { id: string; label: string };

export type SavedQueryRow = {
  vault_id: VaultId;
  title: string;
  slug: string;
  relative_path: string;
  cells: unknown[];
};

export type SavedQueryTruncation = {
  reason: "definition_limit" | "ceiling";
  shown: number;
};

/** A presentation instruction the server set aside. The rows are complete
 * and correct without it. */
export type SavedQueryIgnored = { instruction: string; message: string };

// Refused, empty and populated are separate statuses (#276, ADR-21), so a
// broken definition can never reach the page looking like an empty answer.
// `populated` always carries at least one row.
export type SavedQueryResult = {
  name?: string;
  source: string;
} & (
  | {
      status: "populated";
      view_name?: string;
      columns: SavedQueryColumn[];
      rows: SavedQueryRow[];
      truncated?: SavedQueryTruncation;
      ignored?: SavedQueryIgnored[];
    }
  | {
      status: "empty";
      view_name?: string;
      columns: SavedQueryColumn[];
      ignored?: SavedQueryIgnored[];
    }
  | { status: "refused"; construct: string; message: string }
  | { status: "stopped"; message: string }
);

/** Something wrong with a `hatchdoor-query` marker. It belongs to the note, not
 * to one saved query, and never changes a row. */
export type SavedQueryMarkerProblem =
  | { problem: "orphaned"; name: string; line: number; message: string }
  | { problem: "unusable_name"; name: string; query: number; message: string }
  | {
      problem: "duplicate_name";
      name: string;
      queries: number[];
      message: string;
    };

export type SavedQueriesResponse = {
  vault_id: VaultId;
  slug: string;
  queries: SavedQueryResult[];
  marker_problems: SavedQueryMarkerProblem[];
};

export type SavedQueryState =
  | { status: "none" }
  | {
      status: "loading";
      results: SavedQueryResult[];
      markerProblems: SavedQueryMarkerProblem[];
    }
  | {
      status: "ready";
      results: SavedQueryResult[];
      markerProblems: SavedQueryMarkerProblem[];
    }
  | { status: "error"; message: string };

const QUERY_MARKER = /^<!--\s*hatchdoor-query:([^\n]*)-->$/;

type MarkdownTreeNode = {
  type: string;
  value?: string;
  lang?: string | null;
  position?: unknown;
  data?: Record<string, unknown>;
  children?: MarkdownTreeNode[];
};

/** The element an orphaned marker renders as, so the renderer can put the
 * server's notice where the marker sits. */
export const ORPHANED_MARKER_ELEMENT = "hatchdoor-orphaned-marker";

/**
 * A remark plugin for `<!-- hatchdoor-query: name -->` markers. A marker that
 * names the `base` block after it is dropped: it is an identifier, never
 * content. A marker with no block after it names nothing, so it becomes an
 * {@link ORPHANED_MARKER_ELEMENT} carrying the name as written, where the
 * server's notice about it is drawn (#276). Every other piece of raw HTML
 * renders exactly as it did before. Every other node keeps its source
 * position, so inline editing still addresses the right lines.
 */
export function remarkHideQueryMarkers() {
  const visit = (node: MarkdownTreeNode) => {
    if (!node.children) {
      return;
    }
    const children: MarkdownTreeNode[] = [];
    node.children.forEach((child, index) => {
      const marker =
        child.type === "html"
          ? QUERY_MARKER.exec((child.value ?? "").trim())
          : null;
      if (!marker) {
        children.push(child);
        return;
      }
      const next = node.children?.[index + 1];
      if (next?.type === "code" && next.lang === "base") {
        return;
      }
      children.push({
        type: "hatchdoorOrphanedMarker",
        position: child.position,
        data: {
          hName: ORPHANED_MARKER_ELEMENT,
          hProperties: { dataName: marker[1].trim() },
        },
      });
    });
    node.children = children;
    node.children.forEach(visit);
  };
  return (tree: MarkdownTreeNode) => visit(tree);
}

const BASE_FENCE = /^ {0,3}(?:`{3,}|~{3,})[ \t]*base(?:[ \t]|$)/m;
const MARKER_LINE = /^[ \t]*<!--\s*hatchdoor-query:/m;

/** Whether a note's Markdown holds anything the server would report on: a
 * `base` block, or a name marker that may be missing its block. Saves a
 * request on every note that has neither, which is nearly all of them. */
export function hasSavedQueryMarkup(markdown: string): boolean {
  return BASE_FENCE.test(markdown) || MARKER_LINE.test(markdown);
}

/** The key a block's text and a result's `source` are matched on. The renderer
 * hands over the block's text with its final newline removed, and the server
 * returns it as written, so both are folded the same way before comparing. */
export function savedQueryKey(source: string): string {
  return source
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .replace(/^\n+|\n+$/g, "");
}

/**
 * Fetch the note's evaluated saved queries whenever its content changes or the
 * collection moves. A result is recomputed on every read, so a filter comparing
 * against the current time answers afresh each time the note is opened.
 *
 * While a refetch is in flight the previous results stay on screen rather than
 * collapsing every table to a loading line.
 */
export function useSavedQueries(
  notePath: string,
  markdown: string | undefined,
  contentHash: string | undefined,
  refreshKey: unknown,
): SavedQueryState {
  const [state, setState] = useState<SavedQueryState>({ status: "none" });
  const requestRef = useRef(0);
  const wanted = markdown !== undefined && hasSavedQueryMarkup(markdown);

  useEffect(() => {
    const request = ++requestRef.current;
    if (!wanted) {
      setState({ status: "none" });
      return;
    }
    setState((previous) =>
      previous.status === "ready" || previous.status === "loading"
        ? { ...previous, status: "loading" }
        : { status: "loading", results: [], markerProblems: [] },
    );
    void (async () => {
      try {
        const response = await apiFetch(`${notePath}/saved-queries`);
        if (!response.ok) {
          const body = (await response.json().catch(() => null)) as {
            message?: string;
          } | null;
          throw new Error(
            body?.message ?? `Saved queries failed (${response.status})`,
          );
        }
        const json =
          (await response.json()) as VaultReadProjection<SavedQueriesResponse>;
        if (request !== requestRef.current) return;
        setState({
          status: "ready",
          results: json.data.queries,
          markerProblems: json.data.marker_problems ?? [],
        });
      } catch (error) {
        if (request !== requestRef.current) return;
        setState({
          status: "error",
          message:
            error instanceof Error
              ? error.message
              : "Saved queries could not be loaded",
        });
      }
    })();
  }, [notePath, wanted, contentHash, refreshKey]);

  return state;
}

/**
 * The 1-based line of every `base` block's opening fence, in document order,
 * read with the rules the server uses to find them: a fence indented at most
 * three spaces, closed by a run of the same character at least as long, with
 * anything inside another fence skipped. The line a block renders at gives its
 * position among them, which is what tells two identical blocks apart.
 */
export function baseFenceLines(markdown: string): number[] {
  const lines: number[] = [];
  let open: { marker: string; length: number } | null = null;
  markdown.split("\n").forEach((raw, index) => {
    const line = raw.replace(/\r$/, "");
    const fence = /^( {0,3})(`{3,}|~{3,})(.*)$/.exec(line);
    if (open) {
      const closing = /^ *(`{3,}|~{3,})\s*$/.exec(line);
      if (
        closing &&
        closing[1][0] === open.marker &&
        closing[1].length >= open.length
      ) {
        open = null;
      }
      return;
    }
    if (!fence) {
      return;
    }
    open = { marker: fence[2][0], length: fence[2].length };
    if (fence[3].trim().split(/\s+/)[0] === "base") {
      lines.push(index + 1);
    }
  });
  return lines;
}

export type SavedQueryContextValue = {
  state: SavedQueryState;
  vaultId: VaultId;
  fenceLines: number[];
};

export const SavedQueryContext = createContext<SavedQueryContextValue | null>(
  null,
);

/**
 * The result for one block: the one at the block's position, provided its
 * source agrees, otherwise the first whose source matches. Position alone could
 * pair a block with the wrong result if the two sides ever counted blocks
 * differently; source alone would hand two identical blocks the same result,
 * when the second may have been refused for reusing a name.
 */
export function matchResult(
  results: SavedQueryResult[],
  source: string,
  line: number | undefined,
  fenceLines: number[],
): SavedQueryResult | undefined {
  const key = savedQueryKey(source);
  const position = line === undefined ? -1 : fenceLines.indexOf(line);
  const positioned = position >= 0 ? results[position] : undefined;
  if (positioned && savedQueryKey(positioned.source) === key) {
    return positioned;
  }
  return results.find((candidate) => savedQueryKey(candidate.source) === key);
}

/** The link text for a row: the file's name without its `.md`, the way a note
 * is named everywhere else in the app, or the title when the link sits on a
 * cell that is empty. */
export function linkText(
  columnId: string,
  cell: unknown,
  title: string,
): string {
  const text = formatCell(cell);
  if (columnId === "file.name" && text.endsWith(".md")) {
    return text.slice(0, -".md".length);
  }
  return text || title;
}

/** One property value as a reader would write it. A property the note does not
 * carry arrives as `null` and shows as an empty cell. */
export function formatCell(value: unknown): string {
  if (value === null || value === undefined) {
    return "";
  }
  if (Array.isArray(value)) {
    return value.map(formatCell).join(", ");
  }
  if (typeof value === "object") {
    return JSON.stringify(value);
  }
  return String(value);
}

/** The marker notices that belong under one saved query's table: its own
 * unusable name, and any name it shares with another saved query. */
export function markerNoticesFor(
  problems: SavedQueryMarkerProblem[],
  query: number,
): string[] {
  return problems
    .filter(
      (problem) =>
        (problem.problem === "unusable_name" && problem.query === query) ||
        (problem.problem === "duplicate_name" &&
          problem.queries.includes(query)),
    )
    .map((problem) => problem.message);
}

export type SortState = {
  column: number;
  direction: "ascending" | "descending";
};

/** The next sort after a click on `column`: ascending, then descending, then
 * back to the server's order. A click on another column starts it ascending. */
export function nextSort(
  current: SortState | null,
  column: number,
): SortState | null {
  if (!current || current.column !== column) {
    return { column, direction: "ascending" };
  }
  return current.direction === "ascending"
    ? { column, direction: "descending" }
    : null;
}

/**
 * Rows re-sorted for the screen alone. Nothing here is written anywhere: the
 * definition cannot carry a sort (ADR-21), so a reader's sort is viewer state
 * that a reload forgets, the way Obsidian treats it. Empty cells go last in
 * either direction, and ties keep the server's order.
 */
export function sortRows(
  rows: SavedQueryRow[],
  sort: SortState | null,
): SavedQueryRow[] {
  if (!sort) {
    return rows;
  }
  const sign = sort.direction === "ascending" ? 1 : -1;
  return rows
    .map((row, index) => ({ row, index }))
    .sort((left, right) => {
      const a = left.row.cells[sort.column];
      const b = right.row.cells[sort.column];
      const aEmpty = formatCell(a) === "";
      const bEmpty = formatCell(b) === "";
      if (aEmpty || bEmpty) {
        return aEmpty === bEmpty ? left.index - right.index : aEmpty ? 1 : -1;
      }
      const order =
        typeof a === "number" && typeof b === "number"
          ? a - b
          : formatCell(a).localeCompare(formatCell(b), undefined, {
              numeric: true,
              sensitivity: "base",
            });
      return order === 0 ? left.index - right.index : sign * order;
    })
    .map(({ row }) => row);
}

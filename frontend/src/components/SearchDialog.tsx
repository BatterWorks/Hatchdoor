import { useEffect, useRef, useState, type ReactNode, type RefObject } from "react";

import type { SearchResult, SearchSelection } from "../types";
import { UiButton, UiPanel } from "./ui";

type NoteGroup = {
  note_slug: string;
  note_title: string;
  note_path: string;
  chunks: SearchResult[];
};

function groupResults(results: SearchResult[]): NoteGroup[] {
  const map = new Map<string, NoteGroup>();
  for (const r of results) {
    if (!map.has(r.note_slug)) {
      map.set(r.note_slug, {
        note_slug: r.note_slug,
        note_title: r.note_title,
        note_path: r.note_path,
        chunks: [],
      });
    }
    map.get(r.note_slug)!.chunks.push(r);
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
  inputRef: RefObject<HTMLInputElement | null>;
  onClose: () => void;
  onQueryChange: (value: string) => void;
  onIncludeContentChange: (value: boolean) => void;
  onSelect: (selection: SearchSelection) => void;
}) {
  const trimmedQuery = query.trim();
  const resultsListRef = useRef<HTMLUListElement | null>(null);
  const [expandedSlugs, setExpandedSlugs] = useState<Set<string>>(new Set());

  useEffect(() => {
    setExpandedSlugs(new Set());
  }, [results]);

  function toggleExpanded(slug: string) {
    setExpandedSlugs((prev) => {
      const next = new Set(prev);
      if (next.has(slug)) {
        next.delete(slug);
      } else {
        next.add(slug);
      }
      return next;
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
          <p>No matching notes.</p>
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
            const isExpanded = expandedSlugs.has(group.note_slug);
            const hiddenCount = rest.length;

            return (
              <li key={group.note_slug} className="search-group">
                <button
                  type="button"
                  className="search-result search-result--primary"
                  onClick={() =>
                    onSelect({
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
                    {highlightMatches(`${group.note_path}.md`, trimmedQuery)}
                  </div>
                  {first.heading_path ? (
                    <div className="result-breadcrumb">{first.heading_path}</div>
                  ) : null}
                  <p className="result-snippet">
                    {highlightMatches(stripSnippet(first.content), trimmedQuery)}
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
                    onClick={() => toggleExpanded(group.note_slug)}
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

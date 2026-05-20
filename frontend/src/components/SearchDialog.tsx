import { useRef, type ReactNode, type RefObject } from "react";

import type { SearchResult, SearchSelection } from "../types";
import { UiButton, UiPanel } from "./ui";

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
          placeholder="Search notes (title, path, content)"
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
        {!loading &&
        !error &&
        query.trim().length >= 2 &&
        results.length === 0 ? (
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
          {results.map((result) => (
            <li key={`${result.note_slug}-${result.chunk_id}`}>
              <UiButton
                className="search-result"
                onClick={() =>
                  onSelect({
                    slug: result.note_slug,
                    query: trimmedQuery,
                    matchKind: result.heading_path ?? "",
                  })
                }
              >
                <div className="search-main">
                  <strong>
                    {highlightMatches(result.note_title, trimmedQuery)}
                  </strong>
                  <span>
                    {highlightMatches(
                      `${result.note_path}.md`,
                      trimmedQuery,
                    )}
                  </span>
                </div>
                {result.heading_path ? (
                  <span className="search-kind">{result.heading_path}</span>
                ) : null}
                <p className="search-snippet">
                  {highlightMatches(
                    result.content.length > 240
                      ? result.content.slice(0, 240) + "…"
                      : result.content,
                    trimmedQuery,
                  )}
                </p>
              </UiButton>
            </li>
          ))}
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

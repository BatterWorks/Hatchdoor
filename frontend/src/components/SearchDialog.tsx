import type { RefObject } from "react";

import type { SearchResult } from "../types";
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
  onSelect: (slug: string) => void;
}) {
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
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
        />

        <label className="search-toggle">
          <input
            type="checkbox"
            checked={includeContent}
            onChange={(event) => onIncludeContentChange(event.target.checked)}
          />
          Include content matches
        </label>

        {loading ? <p>Searching…</p> : null}
        {error ? <p className="error">{error}</p> : null}
        {!loading &&
        !error &&
        query.trim().length >= 2 &&
        results.length === 0 ? (
          <p>No matching notes.</p>
        ) : null}

        <ul className="search-results">
          {results.map((result) => (
            <li key={`${result.slug}-${result.match_kind}`}>
              <UiButton
                className="search-result"
                onClick={() => onSelect(result.slug)}
              >
                <div className="search-main">
                  <strong>{result.title}</strong>
                  <span>{result.relative_path}.md</span>
                </div>
                <span className="search-kind">{result.match_kind}</span>
                {result.snippet ? (
                  <p className="search-snippet">{result.snippet}</p>
                ) : null}
              </UiButton>
            </li>
          ))}
        </ul>
      </UiPanel>
    </div>
  );
}

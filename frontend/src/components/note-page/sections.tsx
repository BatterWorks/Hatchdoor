import { useLayoutEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";

import { normalizeTags, type FrontmatterValue } from "../../lib/markdown";
import type { NoteHeading } from "../../lib/noteHeadings";
import type { NoteLinks } from "../../types";
import { UiButton } from "../ui";
import { jumpToHeading } from "./dom";
import {
  buildContentWithFrontmatter,
  parseFrontmatterEntries,
  splitFrontmatter,
} from "./frontmatter";

export function NoteProperties({
  properties,
  content,
  editable = false,
  collapsed,
  onToggleCollapsed,
  onTagSelect,
  onChange,
}: {
  properties: Record<string, FrontmatterValue>;
  /** The whole note, needed to rewrite the frontmatter in place. */
  content?: string;
  editable?: boolean;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  onTagSelect: (tag: string) => void;
  onChange?: (next: string) => void;
}) {
  const [editingKey, setEditingKey] = useState<string | null>(null);

  const commitProperty = (key: string, value: string) => {
    setEditingKey(null);
    if (!content || !onChange) {
      return;
    }
    const { raw, body } = splitFrontmatter(content);
    if (raw === null) {
      return;
    }
    const parsed = parseFrontmatterEntries(raw);
    if (!parsed.editable) {
      return;
    }
    const next = parsed.entries.map((entry) =>
      entry.key === key ? { ...entry, value } : entry,
    );
    onChange(buildContentWithFrontmatter(next, body));
  };

  const canEdit = editable && !!content && !!onChange;
  const entries = Object.entries(properties);
  if (entries.length === 0) {
    return null;
  }

  return (
    <section className="note-properties" data-collapsed={collapsed}>
      {/*
       * The heading is the disclosure, which is what removed the separate
       * Show/Hide button. Button-inside-heading rather than heading-inside-
       * button: a heading is flow content and is not valid inside a button,
       * and this way the outline keeps its "Properties" entry.
       *
       * The caret is the same affordance as the sidebar folder rows, so a
       * collapsible thing reads the same way wherever it appears.
       */}
      <h3 className="note-properties-head">
        <button
          type="button"
          className="note-properties-toggle"
          data-open={!collapsed}
          onClick={onToggleCollapsed}
          aria-expanded={!collapsed}
          aria-controls="note-properties-grid"
        >
          <span className="note-properties-caret" aria-hidden="true" />
          Properties
        </button>
      </h3>

      {/* Rendered even when collapsed, and hidden: `aria-controls` pointing at
          an element that does not exist is worse than a hidden one. */}
      <dl
        id="note-properties-grid"
        className="note-properties-grid"
        hidden={collapsed}
      >
        {entries.map(([key, value]) => (
          <div key={key} className="note-property-row">
            <dt>{key}</dt>
            <dd>
              {editingKey === key ? (
                <PropertyInput
                  initialValue={
                    Array.isArray(value) ? value.join(", ") : String(value)
                  }
                  propertyName={key}
                  onCommit={(next) => commitProperty(key, next)}
                />
              ) : (
                <span
                  className={canEdit ? "note-property-value" : undefined}
                  onClick={canEdit ? () => setEditingKey(key) : undefined}
                >
                  {key === "tags" ? (
                    <TagChips
                      tags={normalizeTags(value)}
                      onSelect={
                        canEdit ? () => setEditingKey(key) : onTagSelect
                      }
                    />
                  ) : (
                    <PropertyValue value={value} />
                  )}
                </span>
              )}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

/**
 * Frontmatter values edit in place for the same reason blocks do: changing a
 * tag should not require dropping the whole note into source mode.
 */
function PropertyInput({
  initialValue,
  propertyName,
  onCommit,
}: {
  initialValue: string;
  propertyName: string;
  onCommit: (value: string) => void;
}) {
  const [value, setValue] = useState(initialValue);
  const ref = useRef<HTMLInputElement | null>(null);

  useLayoutEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);

  return (
    <input
      ref={ref}
      className="field-input note-property-input"
      aria-label={`Editing ${propertyName}`}
      value={value}
      onChange={(event) => setValue(event.target.value)}
      onBlur={() => onCommit(value)}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing) {
          return;
        }
        if (event.key === "Enter" || event.key === "Escape") {
          event.preventDefault();
          onCommit(value);
        }
      }}
    />
  );
}

function PropertyValue({ value }: { value: FrontmatterValue }) {
  if (Array.isArray(value)) {
    return <span>{value.join(", ")}</span>;
  }
  return <span>{value}</span>;
}

function TagChips({
  tags,
  onSelect,
}: {
  tags: string[];
  onSelect: (tag: string) => void;
}) {
  if (tags.length === 0) {
    return <span>None</span>;
  }

  return (
    <div className="tag-chip-list">
      {tags.map((tag) => (
        <button
          type="button"
          key={tag}
          className="tag-chip"
          onClick={() => onSelect(tag)}
          title={`Search tag: ${tag}`}
        >
          #{tag}
        </button>
      ))}
    </div>
  );
}

export function SearchHitNavigator({
  totalHits,
  activeHit,
  onSelect,
}: {
  totalHits: number;
  activeHit: number;
  onSelect: (index: number) => void;
}) {
  if (totalHits <= 0) {
    return null;
  }

  const canNavigate = totalHits > 1;
  const previous = () => {
    if (!canNavigate) {
      return;
    }
    onSelect((activeHit - 1 + totalHits) % totalHits);
  };
  const next = () => {
    if (!canNavigate) {
      return;
    }
    onSelect((activeHit + 1) % totalHits);
  };

  return (
    <div className="note-search-nav" aria-label="Search matches in note">
      <span>
        Match {Math.min(activeHit + 1, totalHits)} of {totalHits}
      </span>
      <div className="note-search-nav-actions">
        <UiButton
          className="close-note"
          onClick={previous}
          disabled={!canNavigate}
        >
          Prev
        </UiButton>
        <UiButton className="close-note" onClick={next} disabled={!canNavigate}>
          Next
        </UiButton>
      </div>
    </div>
  );
}

export function NoteLinksPanel({ links }: { links: NoteLinks | null }) {
  const outgoing = links?.outgoing ?? [];
  const backlinks = links?.backlinks ?? [];
  if (outgoing.length === 0 && backlinks.length === 0) {
    return null;
  }
  const totalLinks = outgoing.length + backlinks.length;

  return (
    <details className="note-links-panel" aria-label="Note links">
      <summary>
        <span>Links</span>
        <span className="note-links-count">{totalLinks}</span>
      </summary>
      <div className="note-links-body">
        <div className="note-links-grid">
          <NoteLinksList title="Outgoing" links={outgoing} />
          <NoteLinksList title="Backlinks" links={backlinks} />
        </div>
      </div>
    </details>
  );
}

function NoteLinksList({
  title,
  links,
}: {
  title: string;
  links: NoteLinks["outgoing"];
}) {
  return (
    <section className="note-links-list">
      <h4>{title}</h4>
      {links.length === 0 ? (
        <p className="note-links-empty">None</p>
      ) : (
        <ul>
          {links.map((link) => (
            <li key={`${title}-${link.slug}`}>
              <Link to={`/n/${link.slug}`} title={`${link.relative_path}.md`}>
                {link.title}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function NoteTocDesktop({ headings }: { headings: NoteHeading[] }) {
  if (headings.length === 0) {
    return null;
  }

  return (
    <nav className="note-toc note-toc-desktop" aria-label="Table of contents">
      <p className="note-toc-title">On this page</p>
      <ul>
        {headings.map((heading) => (
          <li key={heading.id}>
            <button
              type="button"
              className="note-toc-link"
              data-level={heading.level}
              onClick={() => jumpToHeading(heading.id)}
            >
              {heading.text}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}

export function NoteTocMobile({ headings }: { headings: NoteHeading[] }) {
  const detailsRef = useRef<HTMLDetailsElement>(null);

  if (headings.length === 0) {
    return null;
  }

  const jumpFromMobileToc = (id: string) => {
    detailsRef.current?.removeAttribute("open");
    window.requestAnimationFrame(() => jumpToHeading(id));
  };

  return (
    <details ref={detailsRef} className="note-toc note-toc-mobile">
      <summary>
        <span className="note-toc-mobile-label">On this page</span>
        <span className="note-toc-mobile-count">{headings.length}</span>
      </summary>
      <ul>
        {headings.map((heading) => (
          <li key={`mobile-${heading.id}`}>
            <button
              type="button"
              className="note-toc-link"
              data-level={heading.level}
              onClick={() => jumpFromMobileToc(heading.id)}
            >
              {heading.text}
            </button>
          </li>
        ))}
      </ul>
    </details>
  );
}

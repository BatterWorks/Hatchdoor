import { Link } from "react-router-dom";

import { normalizeTags, type FrontmatterValue } from "../../markdown";
import type { NoteHeading } from "../../noteHeadings";
import type { NoteLinks } from "../../types";
import { UiButton } from "../ui";
import { jumpToHeading } from "./dom";

export function NoteProperties({
  properties,
  collapsed,
  onToggleCollapsed,
  onTagSelect,
}: {
  properties: Record<string, FrontmatterValue>;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  onTagSelect: (tag: string) => void;
}) {
  const entries = Object.entries(properties);
  if (entries.length === 0) {
    return null;
  }

  return (
    <section className="note-properties" data-collapsed={collapsed}>
      <header className="note-properties-head">
        <h3>Properties</h3>
        <UiButton
          className="close-note"
          onClick={onToggleCollapsed}
          aria-expanded={!collapsed}
          aria-controls="note-properties-grid"
        >
          {collapsed ? "Show" : "Hide"}
        </UiButton>
      </header>

      {!collapsed ? (
        <dl id="note-properties-grid" className="note-properties-grid">
          {entries.map(([key, value]) => (
            <div key={key} className="note-property-row">
              <dt>{key}</dt>
              <dd>
                {key === "tags" ? (
                  <TagChips
                    tags={normalizeTags(value)}
                    onSelect={onTagSelect}
                  />
                ) : (
                  <PropertyValue value={value} />
                )}
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
    </section>
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
  if (headings.length === 0) {
    return null;
  }

  return (
    <details className="note-toc note-toc-mobile">
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
              onClick={() => jumpToHeading(heading.id)}
            >
              {heading.text}
            </button>
          </li>
        ))}
      </ul>
    </details>
  );
}

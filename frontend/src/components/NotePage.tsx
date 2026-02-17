import {
  Children,
  isValidElement,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createElement } from "react";
import ReactMarkdown from "react-markdown";
import { Link, useLocation, useParams } from "react-router-dom";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import {
  escapeMarkdownLabel,
  normalizeTags,
  parseFrontmatter,
  parseWikilinkTarget,
  type FrontmatterValue,
} from "../markdown";
import type {
  ActiveNoteMeta,
  MermaidApi,
  Note,
  NoteLinks,
  NoteLinksResponse,
  ResolveBatchResponse,
} from "../types";
import {
  applySearchHighlights,
  normalizeSearchQuery,
  setActiveSearchHit as setActiveSearchHitClass,
} from "../noteSearch";
import {
  assignHeadingId,
  extractMarkdownHeadings,
  type NoteHeading,
} from "../noteHeadings";
import { isNoteEqual, isNoteLinksEqual } from "../stateCompare";
import { NoteSkeleton, StateBlock, StatusBadge, UiButton } from "./ui";

export function NotePage({
  onActiveNoteChange,
  onTagSelect,
  propertiesCollapsedStorageKey,
}: {
  onActiveNoteChange: (meta: ActiveNoteMeta | null) => void;
  onTagSelect: (tag: string) => void;
  propertiesCollapsedStorageKey: string;
}) {
  const params = useParams<{ slug: string }>();
  const location = useLocation();
  const slug = params.slug ?? "";
  const [note, setNote] = useState<Note | null>(null);
  const [noteLinks, setNoteLinks] = useState<NoteLinks | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [propertiesCollapsed, setPropertiesCollapsed] = useState<boolean>(
    () => {
      return window.localStorage.getItem(propertiesCollapsedStorageKey) !== "0";
    },
  );
  const [searchHitCount, setSearchHitCount] = useState(0);
  const [activeSearchHit, setActiveSearchHit] = useState(0);
  const noteBodyRef = useRef<HTMLDivElement | null>(null);
  const searchHitsRef = useRef<HTMLSpanElement[]>([]);

  const loadNote = useCallback(
    async (hardReload: boolean) => {
      setError(null);
      if (hardReload) {
        setNote(null);
      }

      try {
        const res = await fetch(`/api/note/${encodeURIComponent(slug)}`);
        if (!res.ok) {
          throw new Error(`Failed loading note: ${res.status}`);
        }
        const json = (await res.json()) as { note: Note };
        setNote((prev) => (isNoteEqual(prev, json.note) ? prev : json.note));
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Unknown note loading error",
        );
      }
    },
    [slug],
  );

  const loadNoteLinks = useCallback(async () => {
    try {
      const res = await fetch(`/api/note/${encodeURIComponent(slug)}/links`);
      if (!res.ok) {
        throw new Error(`Failed loading note links: ${res.status}`);
      }
      const json = (await res.json()) as NoteLinksResponse;
      setNoteLinks((prev) =>
        isNoteLinksEqual(prev, json.links) ? prev : json.links,
      );
    } catch {
      setNoteLinks(null);
    }
  }, [slug]);

  useEffect(() => {
    void (async () => {
      setLoading(true);
      await loadNote(true);
      await loadNoteLinks();
      setLoading(false);
    })();
  }, [loadNote, loadNoteLinks]);

  useEffect(() => {
    const id = window.setInterval(() => {
      void loadNote(false);
      void loadNoteLinks();
    }, 10_000);

    return () => {
      window.clearInterval(id);
    };
  }, [loadNote, loadNoteLinks]);

  useEffect(() => {
    if (!note) {
      onActiveNoteChange(null);
      return;
    }

    onActiveNoteChange({
      title: note.title,
      slug: note.slug,
      relativePath: note.relative_path,
    });
  }, [note, onActiveNoteChange]);

  useEffect(() => {
    window.localStorage.setItem(
      propertiesCollapsedStorageKey,
      propertiesCollapsed ? "1" : "0",
    );
  }, [propertiesCollapsed, propertiesCollapsedStorageKey]);

  useEffect(() => {
    const onToggle = () => setPropertiesCollapsed((prev) => !prev);
    window.addEventListener("hatchdoor:toggle-note-properties", onToggle);
    return () =>
      window.removeEventListener("hatchdoor:toggle-note-properties", onToggle);
  }, []);

  const parsed = useMemo(() => parseFrontmatter(note?.content ?? ""), [note]);
  const markdown = useResolvedWikilinks(parsed.body, note?.relative_path ?? "");
  const searchQuery = useMemo(
    () => normalizeSearchQuery(new URLSearchParams(location.search).get("q")),
    [location.search],
  );
  const tocHeadings = useMemo(
    () => extractMarkdownHeadings(parsed.body),
    [parsed.body],
  );

  useEffect(() => {
    const root = noteBodyRef.current;
    if (!root) {
      return;
    }

    const hits = applySearchHighlights(root, searchQuery);
    searchHitsRef.current = hits;
    setSearchHitCount(hits.length);
    setActiveSearchHit(0);

    if (hits.length > 0) {
      setActiveSearchHitClass(hits, 0);
      scrollElementIntoView(hits[0], { block: "center", inline: "nearest" });
    }
  }, [searchQuery, markdown, note?.slug]);

  useEffect(() => {
    if (searchHitsRef.current.length === 0) {
      return;
    }
    setActiveSearchHitClass(searchHitsRef.current, activeSearchHit);
  }, [activeSearchHit]);

  if (loading) {
    return <NoteSkeleton />;
  }
  if (error && !note) {
    return (
      <StateBlock
        title="Note Unavailable"
        description={error}
        actionLabel="Retry"
        onAction={() => void loadNote(true)}
      />
    );
  }
  if (!note) {
    return (
      <StateBlock title="Not Found" description="This note no longer exists." />
    );
  }

  const headingCounts = new Map<string, number>();

  return (
    <div className="note-page-layout">
      <article className="note-content">
        <h2 className="note-page-title">{note.title}</h2>
        {error ? <StatusBadge tone="warn" text="Showing cached note" /> : null}
        {searchHitCount > 0 ? (
          <SearchHitNavigator
            totalHits={searchHitCount}
            activeHit={activeSearchHit}
            onSelect={(nextIndex) => {
              setActiveSearchHit(nextIndex);
              const target = searchHitsRef.current[nextIndex];
              scrollElementIntoView(target, {
                block: "center",
                inline: "nearest",
              });
            }}
          />
        ) : null}
        <NoteProperties
          properties={parsed.properties}
          collapsed={propertiesCollapsed}
          onToggleCollapsed={() => setPropertiesCollapsed((prev) => !prev)}
          onTagSelect={onTagSelect}
        />
        <NoteLinksPanel links={noteLinks} />
        <NoteTocMobile headings={tocHeadings} />
        <div ref={noteBodyRef} className="note-body">
          <ReactMarkdown
            remarkPlugins={[remarkGfm, remarkMath]}
            rehypePlugins={[rehypeKatex]}
            components={{
              pre(props) {
                const first = Children.toArray(props.children)[0];
                if (
                  isValidElement<{ className?: string }>(first) &&
                  first.type !== "code"
                ) {
                  return first;
                }
                return <pre>{props.children}</pre>;
              },
              code(props) {
                const { children, className } = props;
                const content = String(children).replace(/\n$/, "");
                const match = /language-(\w+)/.exec(className || "");

                if (match?.[1] === "mermaid") {
                  return <MermaidDiagram chart={content} />;
                }

                if (!match) {
                  return <code className={className}>{children}</code>;
                }

                return <CodeBlock language={match[1]} content={content} />;
              },
              a(props) {
                const { href, children } = props;
                if (
                  typeof href === "string" &&
                  href.startsWith("/__missing__/")
                ) {
                  const target = decodeURIComponent(
                    href.replace("/__missing__/", ""),
                  );
                  return (
                    <span className="broken-link" title={`Missing: ${target}`}>
                      {children}
                    </span>
                  );
                }
                if (isExternalHref(href)) {
                  return (
                    <a href={href} target="_blank" rel="noopener noreferrer">
                      {children}
                    </a>
                  );
                }
                return <a href={href}>{children}</a>;
              },
              img(props) {
                const source =
                  typeof props.src === "string"
                    ? resolveAssetHref(props.src, note.relative_path)
                    : props.src;
                return (
                  <img
                    src={source}
                    alt={props.alt ?? ""}
                    loading="lazy"
                    decoding="async"
                  />
                );
              },
              blockquote(props) {
                return <CalloutOrQuote>{props.children}</CalloutOrQuote>;
              },
              table(props) {
                return (
                  <div className="table-wrap">
                    <table>{props.children}</table>
                  </div>
                );
              },
              h1(props) {
                return renderHeading("h1", props.children, headingCounts);
              },
              h2(props) {
                return renderHeading("h2", props.children, headingCounts);
              },
              h3(props) {
                return renderHeading("h3", props.children, headingCounts);
              },
              h4(props) {
                return renderHeading("h4", props.children, headingCounts);
              },
              h5(props) {
                return renderHeading("h5", props.children, headingCounts);
              },
              h6(props) {
                return renderHeading("h6", props.children, headingCounts);
              },
            }}
          >
            {markdown}
          </ReactMarkdown>
        </div>
      </article>

      <NoteTocDesktop headings={tocHeadings} />
    </div>
  );
}

function NoteProperties({
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

function SearchHitNavigator({
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

function NoteLinksPanel({ links }: { links: NoteLinks | null }) {
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

function NoteTocDesktop({ headings }: { headings: NoteHeading[] }) {
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

function NoteTocMobile({ headings }: { headings: NoteHeading[] }) {
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

function jumpToHeading(id: string): void {
  const heading = document.getElementById(id);
  scrollElementIntoView(heading, { block: "start", inline: "nearest" });
}

function renderHeading(
  tag: "h1" | "h2" | "h3" | "h4" | "h5" | "h6",
  children: ReactNode,
  counts: Map<string, number>,
) {
  const text = flattenText(children).trim();
  const id = assignHeadingId(text, counts);
  return createElement(tag, { id }, children);
}

function scrollElementIntoView(
  element: Element | null,
  options: ScrollIntoViewOptions,
): void {
  if (!element) {
    return;
  }
  const maybeScrollable = element as Element & {
    scrollIntoView?: (options?: ScrollIntoViewOptions) => void;
  };
  if (typeof maybeScrollable.scrollIntoView === "function") {
    maybeScrollable.scrollIntoView(options);
  }
}

function isExternalHref(href: string | undefined): boolean {
  if (!href) {
    return false;
  }
  if (href.startsWith("/") || href.startsWith("#")) {
    return false;
  }

  try {
    const url = new URL(href, window.location.origin);
    return url.origin !== window.location.origin;
  } catch {
    return false;
  }
}

function useResolvedWikilinks(
  markdown: string,
  noteRelativePath: string,
): string {
  const [resolved, setResolved] = useState(markdown);

  useEffect(() => {
    let cancelled = false;

    if (!markdown) {
      queueMicrotask(() => setResolved(""));
      return;
    }

    void (async () => {
      const matches = [...markdown.matchAll(/(!?)\[\[([^\]]+)\]\]/g)];
      const rawTargets = matches
        .filter((m) => m[1] !== "!")
        .map((m) => parseWikilinkTarget(m[2]).target)
        .filter((target) => target.length > 0);
      const uniqueTargets = [...new Set(rawTargets)];

      const map = new Map<string, string | null>();

      if (uniqueTargets.length > 0) {
        try {
          const res = await fetch("/api/resolve-batch", {
            method: "POST",
            headers: {
              "Content-Type": "application/json",
            },
            body: JSON.stringify({ targets: uniqueTargets }),
          });

          if (res.ok) {
            const json = (await res.json()) as ResolveBatchResponse;
            for (const result of json.results) {
              map.set(result.target, result.slug);
            }
          }
        } catch {
          // Leave unresolved values as null fallback.
        }
      }

      const rewritten = markdown.replace(
        /(!?)\[\[([^\]]+)\]\]/g,
        (_whole, bang: string, body: string) => {
          const parsed = parseWikilinkTarget(body);

          if (bang === "!") {
            const source = resolveAssetHref(parsed.target, noteRelativePath);
            return `![${escapeMarkdownLabel(parsed.label)}](${source})`;
          }

          const slug = map.get(parsed.target) ?? null;
          if (slug) {
            return `[${escapeMarkdownLabel(parsed.label)}](/n/${slug})`;
          }
          return `[${escapeMarkdownLabel(parsed.label)}](/__missing__/${encodeURIComponent(parsed.target)})`;
        },
      );

      if (!cancelled) {
        setResolved(rewritten);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [markdown, noteRelativePath]);

  return resolved;
}

function resolveAssetHref(rawTarget: string, noteRelativePath: string): string {
  if (/^(https?:|data:|blob:)/i.test(rawTarget) || rawTarget.startsWith("/")) {
    return rawTarget;
  }

  const [pathPart, suffix] = splitPathSuffix(rawTarget);
  const noteDir = dirname(noteRelativePath);
  const normalized = normalizeRelativePath(noteDir, pathPart);

  if (!normalized) {
    return rawTarget;
  }

  const encoded = normalized.split("/").map(encodeURIComponent).join("/");
  return `/vault-assets/${encoded}${suffix}`;
}

function splitPathSuffix(input: string): [string, string] {
  const markerIndex = input.search(/[?#]/);
  if (markerIndex < 0) {
    return [input, ""];
  }
  return [input.slice(0, markerIndex), input.slice(markerIndex)];
}

function dirname(relativePath: string): string {
  const parts = relativePath.split("/").filter((part) => part.length > 0);
  parts.pop();
  return parts.join("/");
}

function normalizeRelativePath(baseDir: string, target: string): string {
  const targetParts = target.replace(/\\/g, "/").split("/");
  const initial = baseDir ? baseDir.split("/").filter(Boolean) : [];
  const stack = [...initial];

  for (const part of targetParts) {
    if (!part || part === ".") {
      continue;
    }
    if (part === "..") {
      if (stack.length > 0) {
        stack.pop();
      }
      continue;
    }
    stack.push(part);
  }

  return stack.join("/");
}

let mermaidModulePromise: Promise<MermaidApi> | null = null;
let mermaidInitialized = false;

async function getMermaidApi(): Promise<MermaidApi> {
  if (!mermaidModulePromise) {
    mermaidModulePromise = import("mermaid").then(
      (mod) => (mod as { default: MermaidApi }).default,
    );
  }

  const api = await mermaidModulePromise;
  if (!mermaidInitialized) {
    api.initialize({ startOnLoad: false, securityLevel: "strict" });
    mermaidInitialized = true;
  }

  return api;
}

function CalloutOrQuote({ children }: { children: ReactNode }) {
  const nodes = Children.toArray(children);
  const firstContentIndex = nodes.findIndex(
    (node) => !(typeof node === "string" && node.trim().length === 0),
  );

  if (firstContentIndex === -1) {
    return <blockquote>{children}</blockquote>;
  }

  const first = nodes[firstContentIndex];

  if (isValidElement<{ children?: ReactNode }>(first) && first.type === "p") {
    const firstText = flattenText(first.props.children).trim();
    const match = firstText.match(/^\[!([A-Za-z0-9_]+)\]([+-])?\s*(.*)$/);

    if (match) {
      const kind = match[1].toLowerCase();
      const fold = match[2] ?? null;
      const title = match[3] || kind[0].toUpperCase() + kind.slice(1);
      const bodyNodes = nodes
        .slice(firstContentIndex + 1)
        .filter(
          (node) => !(typeof node === "string" && node.trim().length === 0),
        );

      if (fold) {
        return (
          <details
            className={`callout callout-${kind} callout-collapsible`}
            open={fold === "+"}
          >
            <summary className="callout-title">{title}</summary>
            <div className="callout-body">{bodyNodes}</div>
          </details>
        );
      }

      return (
        <div className={`callout callout-${kind}`}>
          <div className="callout-title">{title}</div>
          <div className="callout-body">{bodyNodes}</div>
        </div>
      );
    }
  }

  return <blockquote>{children}</blockquote>;
}

function flattenText(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }
  if (!node) {
    return "";
  }
  if (Array.isArray(node)) {
    return node.map(flattenText).join("");
  }
  if (isValidElement<{ children?: ReactNode }>(node)) {
    return flattenText(node.props.children);
  }
  return "";
}

function CodeBlock({
  language,
  content,
}: {
  language: string;
  content: string;
}) {
  const [copied, setCopied] = useState(false);

  const onCopy = useCallback(async () => {
    const success = await copyText(content);
    if (success) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } else {
      setCopied(false);
    }
  }, [content]);

  return (
    <div className="code-block">
      <div className="code-block-head">
        <span className="code-lang">{language}</span>
        <UiButton className="close-note" onClick={() => void onCopy()}>
          {copied ? "Copied" : "Copy"}
        </UiButton>
      </div>
      <pre>
        <code>{content}</code>
      </pre>
    </div>
  );
}

async function copyText(value: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      return true;
    }
  } catch {
    // Fallback below for non-secure contexts / unsupported clipboard API.
  }

  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.top = "-1000px";
  textarea.style.left = "-1000px";
  document.body.appendChild(textarea);

  textarea.select();
  textarea.setSelectionRange(0, textarea.value.length);
  const copied = document.execCommand("copy");
  document.body.removeChild(textarea);
  return copied;
}

function MermaidDiagram({ chart }: { chart: string }) {
  const [svg, setSvg] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;

    void (async () => {
      try {
        const api = await getMermaidApi();
        const id = `m-${Math.random().toString(36).slice(2)}`;
        const { svg: rendered } = await api.render(id, chart);
        if (mounted) {
          setSvg(rendered);
          setError(null);
        }
      } catch (err) {
        if (mounted) {
          setError(
            err instanceof Error ? err.message : "Invalid mermaid diagram",
          );
        }
      }
    })();

    return () => {
      mounted = false;
    };
  }, [chart]);

  if (error) {
    return <pre className="error">Mermaid error: {error}</pre>;
  }

  return <div className="mermaid" dangerouslySetInnerHTML={{ __html: svg }} />;
}

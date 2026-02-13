import {
  Children,
  isValidElement,
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import ReactMarkdown from "react-markdown";
import { useParams } from "react-router-dom";
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
  ResolveBatchResponse,
} from "../types";
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
  const slug = params.slug ?? "";
  const [note, setNote] = useState<Note | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [propertiesCollapsed, setPropertiesCollapsed] = useState<boolean>(
    () => {
      return window.localStorage.getItem(propertiesCollapsedStorageKey) !== "0";
    },
  );

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
        setNote(json.note);
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Unknown note loading error",
        );
      }
    },
    [slug],
  );

  useEffect(() => {
    void (async () => {
      setLoading(true);
      await loadNote(true);
      setLoading(false);
    })();
  }, [loadNote]);

  useEffect(() => {
    const id = window.setInterval(() => {
      void loadNote(false);
    }, 10_000);

    return () => {
      window.clearInterval(id);
    };
  }, [loadNote]);

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

  const parsed = useMemo(() => parseFrontmatter(note?.content ?? ""), [note]);
  const markdown = useResolvedWikilinks(parsed.body, note?.relative_path ?? "");

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

  return (
    <article className="note-content">
      <h2>{note.title}</h2>
      {error ? <StatusBadge tone="warn" text="Showing cached note" /> : null}
      <NoteProperties
        properties={parsed.properties}
        collapsed={propertiesCollapsed}
        onToggleCollapsed={() => setPropertiesCollapsed((prev) => !prev)}
        onTagSelect={onTagSelect}
      />
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
            if (typeof href === "string" && href.startsWith("/__missing__/")) {
              const target = decodeURIComponent(
                href.replace("/__missing__/", ""),
              );
              return (
                <span className="broken-link" title={`Missing: ${target}`}>
                  {children}
                </span>
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
        }}
      >
        {markdown}
      </ReactMarkdown>
    </article>
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

import {
  Children,
  createElement,
  isValidElement,
  useCallback,
  useEffect,
  useState,
  type ReactNode,
} from "react";

import { assignHeadingId } from "../../noteHeadings";
import type { MermaidApi } from "../../types";
import { UiButton } from "../ui";
import { resolveAssetHref } from "./wikilinks";

export function createNoteMarkdownComponents(
  noteRelativePath: string,
  headingCounts: Map<string, number>,
) {
  return {
    pre(props: { children?: ReactNode }) {
      const first = Children.toArray(props.children)[0];
      if (
        isValidElement<{ className?: string }>(first) &&
        first.type !== "code"
      ) {
        return first;
      }
      return <pre>{props.children}</pre>;
    },
    code(props: { children?: ReactNode; className?: string }) {
      const { children, className } = props;
      const content = String(children ?? "").replace(/\n$/, "");
      const match = /language-(\w+)/.exec(className || "");

      if (match?.[1] === "mermaid") {
        return <MermaidDiagram chart={content} />;
      }

      if (!match) {
        return <code className={className}>{children}</code>;
      }

      return <CodeBlock language={match[1]} content={content} />;
    },
    a(props: { href?: string; children?: ReactNode }) {
      const { href, children } = props;
      if (typeof href === "string" && href.startsWith("/__missing__/")) {
        const target = decodeURIComponent(href.replace("/__missing__/", ""));
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
    img(props: { src?: string; alt?: string }) {
      const source =
        typeof props.src === "string"
          ? resolveAssetHref(props.src, noteRelativePath)
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
    blockquote(props: { children?: ReactNode }) {
      return <CalloutOrQuote>{props.children}</CalloutOrQuote>;
    },
    table(props: { children?: ReactNode }) {
      return (
        <div className="table-wrap">
          <table>{props.children}</table>
        </div>
      );
    },
    h1(props: { children?: ReactNode }) {
      return renderHeading("h1", props.children, headingCounts);
    },
    h2(props: { children?: ReactNode }) {
      return renderHeading("h2", props.children, headingCounts);
    },
    h3(props: { children?: ReactNode }) {
      return renderHeading("h3", props.children, headingCounts);
    },
    h4(props: { children?: ReactNode }) {
      return renderHeading("h4", props.children, headingCounts);
    },
    h5(props: { children?: ReactNode }) {
      return renderHeading("h5", props.children, headingCounts);
    },
    h6(props: { children?: ReactNode }) {
      return renderHeading("h6", props.children, headingCounts);
    },
  };
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
        .filter((node) => !(typeof node === "string" && node.trim().length === 0));

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
          setError(err instanceof Error ? err.message : "Invalid mermaid diagram");
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

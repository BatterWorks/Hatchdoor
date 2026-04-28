import {
  Children,
  isValidElement,
  useCallback,
  useEffect,
  useState,
  type ReactNode,
} from "react";

import type { MermaidApi } from "../../types";
import { UiButton } from "../ui";
import { flattenText } from "./text";

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

export function CalloutOrQuote({ children }: { children: ReactNode }) {
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

export function CodeBlock({
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

export function MermaidDiagram({ chart }: { chart: string }) {
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

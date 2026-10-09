import {
  Children,
  isValidElement,
  useCallback,
  useEffect,
  useState,
  type ReactNode,
  type ReactElement,
} from "react";

import type { MermaidApi } from "../../types";
import { copyText } from "../../lib/clipboard";
import { UiButton } from "../ui";
import { isParagraphElement, splitAtSoftBreaks } from "./paragraphs";
import { flattenText } from "./text";

let mermaidModulePromise: Promise<MermaidApi> | null = null;
const mermaidFontFamily = "Inter Tight, system-ui, sans-serif";

async function getMermaidApi(): Promise<MermaidApi> {
  if (!mermaidModulePromise) {
    mermaidModulePromise = import("mermaid").then(
      (mod) => (mod as { default: MermaidApi }).default,
    );
  }
  return mermaidModulePromise;
}

function isDarkMode(): boolean {
  const dataTheme = document.documentElement.getAttribute("data-theme");
  if (dataTheme === "dark") return true;
  if (dataTheme === "light") return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

async function waitForDocumentFonts(): Promise<void> {
  await document.fonts?.ready;
}

/**
 * The hairline that carries a callout's kind label to the right edge.
 *
 * It states the block's top boundary, which is why a callout needs no top
 * border. It is an element rather than a `::after` because .editable-block
 * already owns that pseudo-element for the edit-gutter mark, and the title is
 * an editable unit.
 */
function CalloutLeadRule() {
  return <span className="callout-lead-rule" aria-hidden="true" />;
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

  if (
    isValidElement<{ children?: ReactNode }>(first) &&
    isParagraphElement(first)
  ) {
    const firstText = flattenText(first.props.children).trim();
    const match = firstText.match(/^\[!([A-Za-z0-9_-]+)\]([+-])?[ \t]*(.*)$/m);

    if (match) {
      const kind = match[1].toLowerCase();
      const fold = match[2] ?? null;
      const attribution = match[3].trim();
      const title = attribution || kind[0].toUpperCase() + kind.slice(1);
      const bodyNodes = nodes
        .slice(firstContentIndex + 1)
        .filter(
          (node) => !(typeof node === "string" && node.trim().length === 0),
        );
      // A Markdown block quote with consecutive `>` lines is parsed as one
      // paragraph separated by a soft line break. Preserve the content after
      // that break as the callout body instead of treating it as the title.
      const pChildren = Children.toArray(
        (first as ReactElement<{ children?: ReactNode }>).props.children,
      );
      const nlIdx = pChildren.findIndex(
        (node) => typeof node === "string" && node.includes("\n"),
      );
      let inlineBody: ReactNode[] = [];
      if (nlIdx !== -1) {
        const pivot = pChildren[nlIdx] as string;
        const tail = pivot.slice(pivot.indexOf("\n") + 1);
        // Blank string children are dropped, but a newline is not blank here:
        // when a line holds nothing but an element (`> **bold**`) its soft
        // break arrives as a string child of its own, and discarding it would
        // fuse two source lines into one block, leaving the second
        // unaddressable and the first writing merged text over one line.
        inlineBody = [
          ...(tail ? [tail] : []),
          ...pChildren.slice(nlIdx + 1),
        ].filter(
          (node) =>
            !(
              typeof node === "string" &&
              node.trim() === "" &&
              !node.includes("\n")
            ),
        );
      }
      // A callout's lines are contiguous, so the title is the blockquote's
      // first line and the run of body text reconstructed from the same
      // paragraph starts on the next one. Both are rebuilt here rather than
      // passed through, so neither carries a usable position any more.
      // The callout's body lines were split from one paragraph, so each is
      // drawn as its own line rather than fused back together.
      const inlineLines = splitAtSoftBreaks(inlineBody);
      const allBody =
        inlineLines.length > 0
          ? [
              ...inlineLines.map((lineChildren, index) => (
                <p key={`inline-callout-${index}`} className="callout-line">
                  {lineChildren}
                </p>
              )),
              ...bodyNodes,
            ]
          : bodyNodes;

      if (kind === "quote" || kind === "cite") {
        return (
          <figure className="pullquote">
            <blockquote>{allBody}</blockquote>
            {attribution && <figcaption>{attribution}</figcaption>}
          </figure>
        );
      }

      if (fold) {
        return (
          <details
            className={`callout callout-${kind} callout-collapsible`}
            open={fold === "+"}
          >
            <summary className="callout-title">
              {title}
              <CalloutLeadRule />
            </summary>
            {allBody.length > 0 && (
              <div className="callout-body">{allBody}</div>
            )}
          </details>
        );
      }

      return (
        <div className={`callout callout-${kind}`}>
          <div className="callout-title">
            {title}
            <CalloutLeadRule />
          </div>
          {allBody.length > 0 && <div className="callout-body">{allBody}</div>}
        </div>
      );
    }
  }

  return <blockquote>{children}</blockquote>;
}

/** A list item; a task item keeps its class so the box lines up. */
export function ListItem({
  className,
  children,
}: {
  className?: string;
  children?: ReactNode;
}) {
  const liClass = className?.includes("task-list-item")
    ? "task-list-item"
    : undefined;
  return <li className={liClass}>{children}</li>;
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

export function MermaidDiagram({ chart }: { chart: string }) {
  const [svg, setSvg] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [dark, setDark] = useState<boolean>(() => isDarkMode());

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const observer = new MutationObserver(() => setDark(isDarkMode()));
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    const onMqChange = () => setDark(isDarkMode());
    mq.addEventListener("change", onMqChange);
    return () => {
      observer.disconnect();
      mq.removeEventListener("change", onMqChange);
    };
  }, []);

  useEffect(() => {
    let mounted = true;

    void (async () => {
      try {
        const api = await getMermaidApi();
        await waitForDocumentFonts();
        api.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: dark ? "dark" : "default",
          fontFamily: mermaidFontFamily,
          themeVariables: {
            fontFamily: mermaidFontFamily,
          },
        });
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
  }, [chart, dark]);

  if (error) {
    return <pre className="error">Mermaid error: {error}</pre>;
  }

  return <div className="mermaid" dangerouslySetInnerHTML={{ __html: svg }} />;
}

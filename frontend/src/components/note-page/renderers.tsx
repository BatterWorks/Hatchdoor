import { Children, createElement, isValidElement, type ReactNode } from "react";

import { assignHeadingId } from "../../noteHeadings";
import {
  CalloutOrQuote,
  CodeBlock,
  MermaidDiagram,
} from "./RendererComponents";
import { flattenText } from "./text";
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
    li(props: { children?: ReactNode; className?: string }) {
      const isTask = props.className?.includes("task-list-item") ?? false;
      return (
        <li className={isTask ? "task-list-item" : undefined}>
          {props.children}
        </li>
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

import { Children, createElement, isValidElement, type ReactNode } from "react";

import { slugifyHeading } from "../../lib/noteHeadings";
import {
  CalloutOrQuote,
  CodeBlock,
  MermaidDiagram,
} from "./RendererComponents";
import { markAsParagraph } from "./paragraphs";
import { EditableBlock } from "./EditableBlock";
import type { UnitType } from "./BlockInput";
import { PdfPreview } from "./PdfPreview";
import { flattenText } from "./text";
import { resolveAssetHref } from "./wikilinks";

export function createNoteMarkdownComponents(
  noteRelativePath: string,
  headingIdsBySourceLine: Map<number, string>,
  options: { editable?: boolean } = {},
) {
  const components = {
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
      if (typeof href === "string" && href.startsWith("/__archived__/")) {
        const slug = href.slice("/__archived__/".length);
        return (
          <a className="archived-link" href={`/n/${slug}`}>
            {children}
          </a>
        );
      }
      if (isExternalHref(href)) {
        return (
          <a href={href} target="_blank" rel="noopener noreferrer">
            {children}
          </a>
        );
      }
      if (typeof href === "string" && isPdfHref(href)) {
        const source = resolveAssetHref(href, noteRelativePath);
        const label = flattenText(children).trim() || "PDF";
        return (
          <a
            className="pdf-link"
            href={source}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={`${label} (PDF document, opens in a new tab)`}
          >
            {children}
            <span className="pdf-link-badge" aria-hidden="true">
              PDF
            </span>
            <span className="pdf-link-open" aria-hidden="true">
              ↗
            </span>
          </a>
        );
      }
      return <a href={href}>{children}</a>;
    },
    p: markAsParagraph(function NoteParagraph(props: {
      children?: ReactNode;
      node?: MarkdownElementNode;
    }) {
      // A lone PDF embed parses as a paragraph wrapping an image, but
      // PdfPreview renders block content. Leaving the paragraph produces
      // invalid nesting, which the browser resolves by splitting the paragraph
      // and detaching the preview from it. Decided from the source node,
      // because by the time children are React elements they carry the mapped
      // img component as their type, not PdfPreview.
      if (holdsOnlyPdfEmbed(props.node, noteRelativePath)) {
        return <>{props.children}</>;
      }
      return <p>{props.children}</p>;
    }),
    img(props: { src?: string; alt?: string }) {
      const source =
        typeof props.src === "string"
          ? resolveAssetHref(props.src, noteRelativePath)
          : props.src;
      if (typeof source === "string" && isPdfHref(source)) {
        return <PdfPreview src={source} label={props.alt ?? "PDF"} />;
      }
      return (
        <img
          src={source}
          alt={props.alt ?? ""}
          loading="lazy"
          decoding="async"
        />
      );
    },
    input(props: { type?: string; checked?: boolean; className?: string }) {
      // mdast-util-to-hast emits task checkboxes disabled, and a disabled input
      // fires no click events at all, so the toggle on the li would never be
      // reached. Enabling it also gives the checkbox a keyboard path: Space
      // fires a click, which bubbles to the same handler.
      if (props.type !== "checkbox") {
        return <input {...props} />;
      }
      return (
        <input
          type="checkbox"
          className={props.className}
          checked={props.checked ?? false}
          disabled={!options.editable}
          aria-label={options.editable ? "Toggle task" : undefined}
          onChange={() => {}}
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
    blockquote(props: { children?: ReactNode; node?: unknown }) {
      return (
        <CalloutOrQuote node={props.node}>{props.children}</CalloutOrQuote>
      );
    },
    table(props: { children?: ReactNode }) {
      return (
        <div className="table-wrap">
          <table>{props.children}</table>
        </div>
      );
    },
    h1(props: MarkdownHeadingProps) {
      return renderHeading(
        "h1",
        props.children,
        headingIdsBySourceLine,
        props.node,
      );
    },
    h2(props: MarkdownHeadingProps) {
      return renderHeading(
        "h2",
        props.children,
        headingIdsBySourceLine,
        props.node,
      );
    },
    h3(props: MarkdownHeadingProps) {
      return renderHeading(
        "h3",
        props.children,
        headingIdsBySourceLine,
        props.node,
      );
    },
    h4(props: MarkdownHeadingProps) {
      return renderHeading(
        "h4",
        props.children,
        headingIdsBySourceLine,
        props.node,
      );
    },
    h5(props: MarkdownHeadingProps) {
      return renderHeading(
        "h5",
        props.children,
        headingIdsBySourceLine,
        props.node,
      );
    },
    h6(props: MarkdownHeadingProps) {
      return renderHeading(
        "h6",
        props.children,
        headingIdsBySourceLine,
        props.node,
      );
    },
  };

  return options.editable ? withEditableBlocks(components) : components;
}

// Block-level entries get wrapped so each rendered block can be swapped for its
// own source lines. Inline entries (a, code, img) are deliberately absent: they
// belong to the block that contains them, not to a range of their own.
const EDITABLE_UNITS: Record<string, UnitType> = {
  p: "paragraph",
  h1: "heading",
  h2: "heading",
  h3: "heading",
  h4: "heading",
  h5: "heading",
  h6: "heading",
  li: "list item",
  // D27: the tr is the unit, not the td. mdast gives tr and td identical
  // ranges, and the delimiter row belongs to no node at all.
  tr: "table row",
  pre: "code block",
};

type ComponentMap = Record<string, (props: never) => ReactNode>;

function withEditableBlocks<T extends ComponentMap>(components: T): T {
  const wrapped = { ...components } as ComponentMap;

  for (const [tag, unitType] of Object.entries(EDITABLE_UNITS)) {
    const Original = components[tag];
    const Wrapped = (props: { node?: unknown; children?: ReactNode }) => (
      <EditableBlock node={props.node} unitType={unitType}>
        {Original
          ? (Original as (p: unknown) => ReactNode)(props)
          : createElement(tag, null, props.children)}
      </EditableBlock>
    );
    wrapped[tag] = Wrapped as (props: never) => ReactNode;
  }

  // The paragraph marker must survive wrapping, or callout detection stops
  // recognising its own first child.
  if (wrapped.p) {
    markAsParagraph(wrapped.p);
  }

  return wrapped as T;
}

type MarkdownElementNode = {
  children?: Array<{
    type?: string;
    tagName?: string;
    value?: string;
    properties?: { src?: unknown };
  }>;
};

function holdsOnlyPdfEmbed(
  node: MarkdownElementNode | undefined,
  noteRelativePath: string,
): boolean {
  const meaningful = (node?.children ?? []).filter(
    (child) => !(child.type === "text" && (child.value ?? "").trim() === ""),
  );

  if (meaningful.length !== 1) {
    return false;
  }

  const only = meaningful[0];
  if (only.tagName !== "img" || typeof only.properties?.src !== "string") {
    return false;
  }

  return isPdfHref(resolveAssetHref(only.properties.src, noteRelativePath));
}

function isPdfHref(href: string): boolean {
  return href.split(/[?#]/, 1)[0].toLowerCase().endsWith(".pdf");
}

type MarkdownHeadingProps = {
  children?: ReactNode;
  node?: { position?: { start?: { line?: number } } };
};

function renderHeading(
  tag: "h1" | "h2" | "h3" | "h4" | "h5" | "h6",
  children: ReactNode,
  headingIdsBySourceLine: Map<number, string>,
  node: MarkdownHeadingProps["node"],
) {
  const text = flattenText(children).trim();
  const id =
    headingIdsBySourceLine.get(node?.position?.start?.line ?? -1) ??
    slugifyHeading(text);
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

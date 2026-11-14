import { Children, createElement, isValidElement, type ReactNode } from "react";
import { Link } from "react-router-dom";

import { slugifyHeading } from "../../lib/noteHeadings";
import { isNoteRoutePath } from "../../lib/notePath";
import {
  CalloutOrQuote,
  CodeBlock,
  ListItem,
  MermaidDiagram,
} from "./RendererComponents";
import { NoteEmbed } from "./NoteEmbed";
import { isNoteEmbedElement, parseNoteEmbedHref } from "./noteEmbeds";
import { markAsParagraph } from "./paragraphs";
import { PdfPreview } from "./PdfPreview";
import { OrphanedMarkerNotice, SavedQueryBlock } from "./SavedQueryBlock";
import { ORPHANED_MARKER_ELEMENT } from "./savedQueries";
import { flattenText } from "./text";
import { resolveAssetHref } from "./wikilinks";
import type { VaultId } from "../../types";

/** Maps the anchor `remarkBlockIds` gave a block (#578) to its element id;
 * a block without one stays without. */
type BlockAnchor = (id: string | undefined) => string | undefined;

type MarkdownCodeProps = {
  children?: ReactNode;
  className?: string;
  node?: { position?: { start?: { line?: number } } };
};

/**
 * The renderers that never touch a Vault: code blocks, callouts, lists,
 * tables and headings. Both the note renderer and the manual renderer build
 * on these, so the manual looks exactly like a note.
 */
function createSharedMarkdownComponents(
  /** `null` draws headings without ids, for content embedded from another
   * note, whose anchors belong to that note and not to this page. */
  headingIdsBySourceLine: Map<number, string> | null,
  renderBaseBlock: (content: string, props: MarkdownCodeProps) => ReactNode,
  hiddenHeadingLine?: number,
  blockAnchor: BlockAnchor = (id) => id,
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
    code(props: MarkdownCodeProps) {
      const { children, className } = props;
      const content = String(children ?? "").replace(/\n$/, "");
      const match = /language-(\w+)/.exec(className || "");

      if (match?.[1] === "mermaid") {
        return <MermaidDiagram chart={content} />;
      }

      if (match?.[1] === "base") {
        return renderBaseBlock(content, props);
      }

      if (!match) {
        return <code className={className}>{children}</code>;
      }

      return <CodeBlock language={match[1]} content={content} />;
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
          disabled
          onChange={() => {}}
        />
      );
    },
    li(props: { children?: ReactNode; className?: string; id?: string }) {
      return (
        <ListItem className={props.className} id={blockAnchor(props.id)}>
          {props.children}
        </ListItem>
      );
    },
    blockquote(props: { children?: ReactNode; id?: string }) {
      return (
        <CalloutOrQuote id={blockAnchor(props.id)}>
          {props.children}
        </CalloutOrQuote>
      );
    },
    // A `hatchdoor-query` marker naming no block (#276): see
    // remarkHideQueryMarkers.
    [ORPHANED_MARKER_ELEMENT](props: { "data-name"?: string }) {
      return <OrphanedMarkerNotice name={props["data-name"]} />;
    },
    table(props: { children?: ReactNode; id?: string }) {
      return (
        <div className="table-wrap" id={blockAnchor(props.id)}>
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
        hiddenHeadingLine,
      );
    },
    h2(props: MarkdownHeadingProps) {
      return renderHeading(
        "h2",
        props.children,
        headingIdsBySourceLine,
        props.node,
        hiddenHeadingLine,
      );
    },
    h3(props: MarkdownHeadingProps) {
      return renderHeading(
        "h3",
        props.children,
        headingIdsBySourceLine,
        props.node,
        hiddenHeadingLine,
      );
    },
    h4(props: MarkdownHeadingProps) {
      return renderHeading(
        "h4",
        props.children,
        headingIdsBySourceLine,
        props.node,
        hiddenHeadingLine,
      );
    },
    h5(props: MarkdownHeadingProps) {
      return renderHeading(
        "h5",
        props.children,
        headingIdsBySourceLine,
        props.node,
        hiddenHeadingLine,
      );
    },
    h6(props: MarkdownHeadingProps) {
      return renderHeading(
        "h6",
        props.children,
        headingIdsBySourceLine,
        props.node,
        hiddenHeadingLine,
      );
    },
  };
}

export function createNoteMarkdownComponents(
  vaultId: VaultId,
  noteRelativePath: string,
  headingIdsBySourceLine: Map<number, string>,
  options: {
    /** The body line of a heading that only repeats the note's title (#530),
     * kept in the DOM but not drawn. */
    hiddenHeadingLine?: number;
    /** Content embedded from another note (#699): no element gets an id,
     * since its heading and block anchors are that note's. */
    embedded?: boolean;
  } = {},
) {
  const blockAnchor: BlockAnchor = options.embedded
    ? () => undefined
    : (id) => id;
  const components = {
    ...createSharedMarkdownComponents(
      options.embedded ? null : headingIdsBySourceLine,
      (content, props) => (
        <SavedQueryBlock
          source={content}
          line={props.node?.position?.start?.line}
        />
      ),
      options.hiddenHeadingLine,
      blockAnchor,
    ),
    a(props: { href?: string; children?: ReactNode }) {
      const { href, children } = props;
      const embed = parseNoteEmbedHref(href);
      if (embed) {
        return (
          <NoteEmbed vaultId={vaultId} target={embed}>
            {children}
          </NoteEmbed>
        );
      }
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
          <Link
            className="archived-link"
            to={`/v/${encodeURIComponent(vaultId)}/n/${slug}`}
          >
            {children}
          </Link>
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
        const source = resolveAssetHref(vaultId, href, noteRelativePath);
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
      // A link to another note is the same navigation the explorer performs,
      // so it goes through the router. A bare anchor would take the browser
      // out and back in, remounting the whole app and rebuilding every Vault
      // tree in the sidebar to land on a note the router already had.
      if (isNoteRouteHref(href)) {
        return <Link to={href}>{children}</Link>;
      }
      return <a href={href}>{children}</a>;
    },
    p: markAsParagraph(function NoteParagraph(props: {
      children?: ReactNode;
      id?: string;
      node?: MarkdownElementNode;
    }) {
      // A lone PDF embed parses as a paragraph wrapping an image, but
      // PdfPreview renders block content. Leaving the paragraph produces
      // invalid nesting, which the browser resolves by splitting the paragraph
      // and detaching the preview from it. Decided from the source node,
      // because by the time children are React elements they carry the mapped
      // img component as their type, not PdfPreview.
      if (holdsOnlyPdfEmbed(vaultId, props.node, noteRelativePath)) {
        return <>{props.children}</>;
      }
      const id = blockAnchor(props.id);
      // A note embed draws as a block (#699), so a paragraph holding one is
      // split around it, the text either side staying a paragraph.
      const runs = splitAroundNoteEmbeds(props.children);
      if (runs) {
        return id ? <div id={id}>{runs}</div> : <>{runs}</>;
      }
      return <p id={id}>{props.children}</p>;
    }),
    img(props: { src?: string; alt?: string }) {
      const source =
        typeof props.src === "string"
          ? resolveAssetHref(vaultId, props.src, noteRelativePath)
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
  };

  return components;
}

/**
 * Renderers for the bundled manual (ADR-38), which belongs to no Vault. A
 * `base` block shows as its source, since a saved query has no Vault to run
 * against, and nothing here calls a Vault endpoint. `renderLink` decides
 * where each link goes; the manual's own links stay inside Help.
 */
export function createManualMarkdownComponents(
  headingIdsBySourceLine: Map<number, string>,
  renderLink: (href: string | undefined, children: ReactNode) => ReactNode,
  /** Turns a block's anchor (`^id`, #578) into the id the caller scrolls to,
   * the way it already chose the heading ids. */
  blockAnchor: BlockAnchor = (id) => id,
) {
  return {
    ...createSharedMarkdownComponents(
      headingIdsBySourceLine,
      (content) => <CodeBlock language="base" content={content} />,
      undefined,
      blockAnchor,
    ),
    a(props: { href?: string; children?: ReactNode }) {
      return renderLink(props.href, props.children);
    },
    p: markAsParagraph(function ManualParagraph(props: {
      children?: ReactNode;
      id?: string;
    }) {
      return <p id={blockAnchor(props.id)}>{props.children}</p>;
    }),
    img(props: { src?: string; alt?: string }) {
      return (
        <img
          src={props.src}
          alt={props.alt ?? ""}
          loading="lazy"
          decoding="async"
        />
      );
    },
  };
}

/**
 * A paragraph's children with each note embed taken out of the paragraph:
 * the text before and after it become paragraphs of their own, and a run
 * holding nothing but white space is dropped. `null` when the paragraph
 * holds no embed.
 */
function splitAroundNoteEmbeds(children: ReactNode): ReactNode[] | null {
  const nodes = Children.toArray(children);
  if (!nodes.some(isNoteEmbedElement)) {
    return null;
  }
  const out: ReactNode[] = [];
  let run: ReactNode[] = [];
  const flush = () => {
    const blank = run.every(
      (node) => typeof node === "string" && node.trim() === "",
    );
    if (!blank) {
      out.push(<p key={`run-${out.length}`}>{run}</p>);
    }
    run = [];
  };
  for (const node of nodes) {
    if (isNoteEmbedElement(node)) {
      flush();
      out.push(node);
    } else {
      run.push(node);
    }
  }
  flush();
  return out;
}

// Block-level entries get wrapped so each rendered block can be swapped for its
// own source lines. Inline entries (a, code, img) are deliberately absent: they
// belong to the block that contains them, not to a range of their own.
type MarkdownElementNode = {
  children?: Array<{
    type?: string;
    tagName?: string;
    value?: string;
    properties?: { src?: unknown };
  }>;
};

function holdsOnlyPdfEmbed(
  vaultId: VaultId,
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

  return isPdfHref(
    resolveAssetHref(vaultId, only.properties.src, noteRelativePath),
  );
}

// Only the note route is the router's to handle. Asset URLs under /api, and
// in-page fragments, are the browser's, and routing them would either break
// the download or resolve the fragment as a path.
function isNoteRouteHref(href: string | undefined): href is string {
  if (typeof href !== "string") {
    return false;
  }
  return isNoteRoutePath(href.split(/[?#]/, 1)[0]);
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
  headingIdsBySourceLine: Map<number, string> | null,
  node: MarkdownHeadingProps["node"],
  hiddenHeadingLine?: number,
) {
  const text = flattenText(children).trim();
  const line = node?.position?.start?.line ?? -1;
  const id = headingIdsBySourceLine
    ? (headingIdsBySourceLine.get(line) ?? slugifyHeading(text))
    : undefined;
  // The note's own title, repeated as its first heading, stays in the DOM
  // (line-addressed editing counts on every block being there) but is not
  // drawn: the page already set the title above the body (#530).
  const hidden = hiddenHeadingLine !== undefined && line === hiddenHeadingLine;
  return createElement(
    tag,
    hidden ? { id, className: "note-heading-duplicate", hidden: true } : { id },
    children,
  );
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

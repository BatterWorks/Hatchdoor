// Note embeds in the Reading view (#699): `![[Note]]`, `![[Note#Heading]]`
// and `![[Note#^id]]` draw the other note's content, or the part of it the
// embed names. `rewriteWikilinks` turns each resolved note embed into a link
// to `/__embed__/…`, and the renderer draws that link as a `NoteEmbed`.

import { createContext, isValidElement, type ReactNode } from "react";
import type { List, ListItem, Parents, Root, RootContent } from "mdast";
import {
  fromMarkdown,
  type Extension as FromMarkdownExtension,
} from "mdast-util-from-markdown";
import type { Extension as MicromarkExtension } from "micromark-util-types";
import { unified, type Processor } from "unified";

import { NOTE_SYNTAX_PLUGINS } from "../../lib/markdownSyntax";
import { extractMarkdownHeadings } from "../../lib/noteHeadings";
import type { VaultId } from "../../types";

/** How deep embeds nest: a note's embed is level 1, an embed inside that 2. */
export const MAX_EMBED_DEPTH = 3;

const EMBED_PREFIX = "/__embed__/";

/** One note embed, as the rewritten Markdown carries it to the renderer. */
export type NoteEmbedTarget = {
  slug: string;
  archived: boolean;
  /** The anchor the embed names, without its `#`: a heading's slug, a
   * block's `^id`, or empty for the whole note. */
  anchor: string;
};

export function noteEmbedHref(target: NoteEmbedTarget): string {
  const query = target.archived ? "?archived" : "";
  const fragment = target.anchor ? `#${target.anchor}` : "";
  // Encoded so a slug with a space (an archived note's path) stays one
  // Markdown destination.
  const slug = target.slug.split("/").map(encodeURIComponent).join("/");
  return `${EMBED_PREFIX}${slug}${query}${fragment}`;
}

export function parseNoteEmbedHref(
  href: string | undefined,
): NoteEmbedTarget | null {
  if (typeof href !== "string" || !href.startsWith(EMBED_PREFIX)) {
    return null;
  }
  const rest = href.slice(EMBED_PREFIX.length);
  const hashIdx = rest.indexOf("#");
  const beforeHash = hashIdx < 0 ? rest : rest.slice(0, hashIdx);
  // The Markdown parser percent-encodes a destination, a block's `^` too.
  const encodedAnchor = hashIdx < 0 ? "" : rest.slice(hashIdx + 1);
  const queryIdx = beforeHash.indexOf("?");
  const encoded = queryIdx < 0 ? beforeHash : beforeHash.slice(0, queryIdx);
  let slug: string;
  let anchor: string;
  try {
    slug = decodeURIComponent(encoded);
    anchor = decodeURIComponent(encodedAnchor);
  } catch {
    return null;
  }
  const archived = queryIdx >= 0 && beforeHash.slice(queryIdx) === "?archived";
  return slug ? { slug, archived, anchor } : null;
}

/** Whether a rendered child is a note embed's link element, which draws as a
 * block and so cannot stay inside a `<p>`. */
export function isNoteEmbedElement(node: ReactNode): boolean {
  return (
    isValidElement<{ href?: unknown }>(node) &&
    typeof node.props.href === "string" &&
    node.props.href.startsWith(EMBED_PREFIX)
  );
}

/** Where an embed sits: its Vault, the notes it is inside of, outermost
 * first, and the collection revision its content was read at. */
export type NoteEmbedScope = {
  vaultId: VaultId;
  chain: string[];
  revision: number | null;
};

export const NoteEmbedContext = createContext<NoteEmbedScope | null>(null);

/** Whether an embed of `slug` from `vaultId` may draw its content here:
 * inside a page, in that page's Vault, not inside itself and not past
 * `MAX_EMBED_DEPTH`. */
export function mayEmbed(
  scope: NoteEmbedScope | null,
  vaultId: VaultId,
  slug: string,
): scope is NoteEmbedScope {
  return (
    scope !== null &&
    scope.vaultId === vaultId &&
    !scope.chain.includes(slug) &&
    scope.chain.length <= MAX_EMBED_DEPTH
  );
}

/** remark-parse, built on the parser the syntax plugins already extend, so a
 * tree read here is the tree the renderer reads. */
function remarkParseNote(this: Processor) {
  // remark-parse reads these two lists; the syntax plugins fill them.
  const data = this.data() as {
    micromarkExtensions?: MicromarkExtension[];
    fromMarkdownExtensions?: FromMarkdownExtension[];
  };
  this.parser = (document: string) =>
    fromMarkdown(document, {
      extensions: data.micromarkExtensions ?? [],
      mdastExtensions: data.fromMarkdownExtensions ?? [],
    });
}

function parseNoteTree(markdown: string): Root {
  const processor = unified().use(remarkParseNote).use(NOTE_SYNTAX_PLUGINS);
  return processor.runSync(processor.parse(markdown)) as Root;
}

/** The lines a heading's section spans, from the heading to the line before
 * the next heading of the same or a higher level. */
type LineRange = { start: number; end: number };

/** What part of an embedded note to draw. */
export type EmbedSelection =
  | { kind: "whole" }
  | { kind: "section"; lines: LineRange }
  | { kind: "block"; anchor: string };

/**
 * The part of `body` (the embedded note's text without its frontmatter) that
 * `anchor` names, or `null` when the note has no such heading or block.
 */
export function selectEmbed(
  body: string,
  anchor: string,
): EmbedSelection | null {
  if (!anchor) {
    return { kind: "whole" };
  }
  if (anchor.startsWith("^")) {
    return findBlock(parseNoteTree(body).children, anchor)
      ? { kind: "block", anchor }
      : null;
  }
  const headings = extractMarkdownHeadings(body);
  const index = headings.findIndex((heading) => heading.id === anchor);
  if (index < 0) {
    return null;
  }
  const heading = headings[index];
  const next = headings
    .slice(index + 1)
    .find((later) => later.level <= heading.level);
  return {
    kind: "section",
    lines: {
      start: heading.sourceLine,
      end: next ? next.sourceLine - 1 : Number.POSITIVE_INFINITY,
    },
  };
}

/**
 * The remark step that keeps only the selected part of the tree. It runs on
 * the whole note, so every node keeps its line in the file, which is what
 * the note's saved queries are matched by.
 */
export function remarkSelectEmbed(options: { selection: EmbedSelection }) {
  return (tree: Root) => {
    const { selection } = options;
    if (selection.kind === "whole") {
      return;
    }
    if (selection.kind === "section") {
      tree.children = tree.children.filter((node) => {
        const line = node.position?.start.line;
        return (
          isDefinition(node) ||
          (line !== undefined &&
            line >= selection.lines.start &&
            line <= selection.lines.end)
        );
      });
      return;
    }
    const found = findBlock(tree.children, selection.anchor);
    // Link reference and footnote definitions stay wherever they were
    // written, so a reference in the kept part still finds its target.
    tree.children = found ? [found, ...tree.children.filter(isDefinition)] : [];
  };
}

function isDefinition(node: RootContent): boolean {
  return node.type === "definition" || node.type === "footnoteDefinition";
}

/**
 * The rehype step that takes every id out of embedded content. Its heading,
 * block and footnote anchors are the other note's, and an id left on the page
 * could catch a jump meant for the page's own heading or block.
 */
export function rehypeDropIds() {
  return (tree: HastNode) => {
    if (tree.properties) {
      delete tree.properties.id;
    }
    tree.children?.forEach((child) => rehypeDropIds()(child));
  };
}

/** The part of a rehype node this step reads. */
type HastNode = {
  properties?: Record<string, unknown>;
  children?: HastNode[];
};

/** The block carrying `anchor`, as a top-level node: a list item comes back
 * inside a list of its own kind, so it still draws as one. */
function findBlock(
  nodes: RootContent[],
  anchor: string,
  parent?: Parents,
): RootContent | null {
  for (const node of nodes) {
    const id = (node.data as { hProperties?: { id?: unknown } } | undefined)
      ?.hProperties?.id;
    if (id === anchor) {
      return node.type === "listItem" ? wrapListItem(node, parent) : node;
    }
    if ("children" in node) {
      const inner = findBlock(node.children as RootContent[], anchor, node);
      if (inner) {
        return inner;
      }
    }
  }
  return null;
}

function wrapListItem(item: ListItem, parent: Parents | undefined): List {
  const list = parent?.type === "list" ? parent : undefined;
  const ordered = list?.ordered ?? false;
  const start =
    ordered && list ? (list.start ?? 1) + list.children.indexOf(item) : null;
  return {
    type: "list",
    ordered,
    start,
    spread: false,
    children: [item],
  };
}

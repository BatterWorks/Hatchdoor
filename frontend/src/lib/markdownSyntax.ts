// The Markdown syntax every rendered view of a note reads (#572): the Reading
// view, the source-mode preview and the Help reader. They used to keep three
// copies of this list, and a syntax the editor's buttons write went missing
// from all three unnoticed.

import type { Nodes, Paragraph, Parents, Root, RootContent } from "mdast";
import {
  fromMarkdown,
  type Extension as FromMarkdownExtension,
} from "mdast-util-from-markdown";
import { splice } from "micromark-util-chunked";
import { classifyCharacter } from "micromark-util-classify-character";
import { resolveAll } from "micromark-util-resolve-all";
import type {
  Code,
  Event,
  Extension,
  State,
  Token,
  TokenizeContext,
  Tokenizer,
} from "micromark-util-types";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import type { Processor } from "unified";

import { blockAnchorId } from "./noteHeadings";

declare module "mdast" {
  interface Highlight extends Parent {
    type: "highlight";
    children: PhrasingContent[];
  }
  interface PhrasingContentMap {
    highlight: Highlight;
  }
  interface RootContentMap {
    highlight: Highlight;
  }
}

declare module "micromark-util-types" {
  interface TokenTypeMap {
    highlight: "highlight";
    highlightSequence: "highlightSequence";
    highlightSequenceTemporary: "highlightSequenceTemporary";
    highlightText: "highlightText";
  }
}

const EQUALS = 61;
/** What `classifyCharacter` returns for punctuation (1 is whitespace). */
const PUNCTUATION = 2;

/**
 * Obsidian's `==highlight==`, read the way the live editor reads it
 * (`highlightMarkdown` in `@atomic-editor/editor`), so both views of a note
 * agree: a run of exactly two `=`, opening and closing by the flanking rules
 * emphasis uses, closed within its paragraph. Being a parser construct and not a
 * text search, it leaves code, math and escaped signs alone for free. Built
 * like GFM strikethrough, which is the same shape with `~`.
 */
const highlightConstruct = {
  name: "highlight",
  tokenize: tokenizeHighlight,
  resolveAll: resolveAllHighlight,
};

const highlightSyntax: Extension = {
  text: { [EQUALS]: highlightConstruct },
  insideSpan: { null: [highlightConstruct] },
  attentionMarkers: { null: [EQUALS] },
};

function tokenizeHighlight(
  this: TokenizeContext,
  effects: Parameters<Tokenizer>[0],
  ok: State,
  nok: State,
): State {
  const previous = this.previous;
  let size = 0;

  const start: State = (code: Code) => {
    // The middle of a longer run is not the start of one, and the editor
    // counts an escaped sign as part of the run too.
    if (previous === EQUALS) return nok(code);
    effects.enter("highlightSequenceTemporary");
    return more(code);
  };

  const more: State = (code: Code) => {
    if (code === EQUALS) {
      if (size === 2) return nok(code);
      effects.consume(code);
      size++;
      return more;
    }
    if (size < 2) return nok(code);
    const token = effects.exit("highlightSequenceTemporary");
    const before = classifyCharacter(previous);
    const after = classifyCharacter(code);
    token._open = !after || (after === PUNCTUATION && Boolean(before));
    token._close = !before || (before === PUNCTUATION && Boolean(after));
    return ok(code);
  };

  return start;
}

function resolveAllHighlight(
  events: Event[],
  context: TokenizeContext,
): Event[] {
  let index = -1;

  while (++index < events.length) {
    const closer = events[index][1];
    if (
      events[index][0] !== "enter" ||
      closer.type !== "highlightSequenceTemporary" ||
      !closer._close
    ) {
      continue;
    }
    let open = index;
    while (open--) {
      const opener = events[open][1];
      if (
        events[open][0] !== "exit" ||
        opener.type !== "highlightSequenceTemporary" ||
        !opener._open
      ) {
        continue;
      }
      opener.type = "highlightSequence";
      closer.type = "highlightSequence";
      const highlight: Token = {
        type: "highlight",
        start: { ...opener.start },
        end: { ...closer.end },
      };
      const text: Token = {
        type: "highlightText",
        start: { ...opener.end },
        end: { ...closer.start },
      };
      const next: Event[] = [
        ["enter", highlight, context],
        ["enter", opener, context],
        ["exit", opener, context],
        ["enter", text, context],
      ];
      const insideSpan = context.parser.constructs.insideSpan.null;
      if (insideSpan) {
        splice(
          next,
          next.length,
          0,
          resolveAll(insideSpan, events.slice(open + 1, index), context),
        );
      }
      splice(next, next.length, 0, [
        ["exit", text, context],
        ["enter", closer, context],
        ["exit", closer, context],
        ["exit", highlight, context],
      ]);
      splice(events, open - 1, index - open + 3, next);
      index = open + next.length - 2;
      break;
    }
  }

  // Unpaired signs are text.
  for (const [, token] of events) {
    if (token.type === "highlightSequenceTemporary") token.type = "data";
  }
  return events;
}

const highlightFromMarkdown: FromMarkdownExtension = {
  // Keeps the line break of a highlight that runs onto the next line.
  canContainEols: ["highlight"],
  enter: {
    highlight(token) {
      // `hName` makes it a `<mark>` in the rendered page.
      this.enter(
        {
          type: "highlight",
          children: [],
          data: {
            hName: "mark",
            hProperties: { className: ["note-highlight"] },
          },
        },
        token,
      );
    },
  },
  exit: {
    highlight(token) {
      this.exit(token);
    },
  },
};

export function remarkHighlight(this: Processor) {
  // remark-parse reads these two lists; its types declaring them are not ours.
  const data = this.data() as {
    micromarkExtensions?: Extension[];
    fromMarkdownExtensions?: FromMarkdownExtension[];
  };
  (data.micromarkExtensions ??= []).push(highlightSyntax);
  (data.fromMarkdownExtensions ??= []).push(highlightFromMarkdown);
}

/** A block id closing a block: ` ^id` after its text, or `^id` alone. */
const BLOCK_ID_AT_END = /(^|[ \t\n])\^([a-zA-Z0-9-]+)[ \t]*$/;
/** A block id closing a line that is not the block's last. */
const BLOCK_ID_AT_LINE_END = / \^[a-zA-Z0-9-]+(?=\n)/g;

type BlockIdCarrier = Extract<RootContent, { type: "paragraph" | "heading" }>;

/**
 * Obsidian's block ids (#578): a `^id` closing a paragraph, a list item or a
 * quote, or standing on a line of its own under a quote or a table. The text
 * is taken out of the tree and the block is given the anchor `^id`
 * (`blockAnchorId`), which is where a link to `[[Note#^id]]` lands. Working on
 * the parsed tree leaves a `^id` in code or math alone and changes no line of
 * the text, so a rendered block still maps to its lines in the file.
 *
 * The anchor goes on the list item when the id closes the item's own line and
 * on the quote when it closes the quote, since a callout rebuilds its
 * paragraphs. A heading has an anchor already and only loses the text.
 */
export function remarkBlockIds() {
  return (tree: Root) => {
    anchorBlockIds(tree);
  };
}

function anchorBlockIds(parent: Parents): void {
  const siblings = parent.children as RootContent[];
  for (let index = 0; index < siblings.length; index += 1) {
    const node = siblings[index];
    if (node.type !== "paragraph" && node.type !== "heading") {
      if ("children" in node) {
        anchorBlockIds(node);
      }
      continue;
    }

    hideLineEndBlockIds(node);
    const found = closingBlockId(node);
    if (!found) {
      continue;
    }
    if (node.type === "heading") {
      // A heading that is nothing but `^word` is a heading, not an id.
      if (!found.alone) {
        found.strip();
      }
      continue;
    }
    if (!found.alone) {
      found.strip();
      setBlockAnchor(blockIdTarget(parent, node), found.id);
      continue;
    }
    // An id on a line of its own names the block above it. With no such block
    // it stays as written, the way a stray `^word` always has.
    const above = siblings[index - 1];
    if (
      above &&
      (above.type === "paragraph" ||
        above.type === "blockquote" ||
        above.type === "table")
    ) {
      setBlockAnchor(above, found.id);
      siblings.splice(index, 1);
      index -= 1;
    }
  }
}

function hideLineEndBlockIds(node: BlockIdCarrier): void {
  for (const child of node.children) {
    if (child.type === "text") {
      child.value = child.value.replace(BLOCK_ID_AT_LINE_END, "");
    }
  }
}

function closingBlockId(
  node: BlockIdCarrier,
): { id: string; alone: boolean; strip: () => void } | null {
  const last = node.children.at(-1);
  if (!last || last.type !== "text") {
    return null;
  }
  const match = BLOCK_ID_AT_END.exec(last.value);
  if (!match) {
    return null;
  }
  const alone = node.children.length === 1 && match.index === 0 && !match[1];
  // With no space before it the caret belongs to the word it touches.
  if (!match[1] && !alone) {
    return null;
  }
  return {
    id: match[2],
    alone,
    strip: () => {
      last.value = last.value.slice(0, match.index);
    },
  };
}

function blockIdTarget(parent: Parents, paragraph: Paragraph): RootContent {
  if (parent.type === "listItem" && parent.children[0] === paragraph) {
    return parent;
  }
  if (parent.type === "blockquote" && parent.children.at(-1) === paragraph) {
    return parent;
  }
  return paragraph;
}

function setBlockAnchor(node: RootContent, id: string): void {
  const data = (node.data ??= {}) as {
    hProperties?: Record<string, unknown>;
  };
  const properties = (data.hProperties ??= {});
  // A block names one place: the first id written on it.
  properties.id ??= blockAnchorId(id);
}

/** The syntax plugins shared by every view that renders note Markdown. */
export const NOTE_SYNTAX_PLUGINS = [
  remarkGfm,
  remarkMath,
  remarkHighlight,
  remarkBlockIds,
];

/** Plain text with each `==highlight==`'s signs dropped, for a search
 * snippet, which shows the words and draws no highlight. It parses the text
 * with the same construct the page renders with, so code, escapes and stray
 * signs keep theirs exactly where the page keeps them. */
export function stripHighlights(text: string): string {
  if (!text.includes("==")) return text;
  const tree = fromMarkdown(text, {
    extensions: [highlightSyntax],
    mdastExtensions: [highlightFromMarkdown],
  });
  const signs: number[] = [];
  const collect = (node: Nodes) => {
    if (node.type === "highlight" && node.position) {
      signs.push(node.position.start.offset!, node.position.end.offset! - 2);
    }
    if ("children" in node) node.children.forEach(collect);
  };
  collect(tree);
  let out = text;
  for (const at of signs.sort((a, b) => b - a)) {
    out = out.slice(0, at) + out.slice(at + 2);
  }
  return out;
}

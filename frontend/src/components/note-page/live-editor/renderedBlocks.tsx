// The blocks the reading view renders and the editor used to leave as
// source (#544): a `mermaid` fence as its diagram, a `base` fence as the
// saved-query table the server evaluated, `$$` and `$` math through KaTeX,
// and `![[file.pdf]]` as the PDF preview. Each is a widget in place of its
// source lines while the caret is elsewhere, and the source again the moment
// the caret lands on them, so nothing rendered is ever more than a caret
// move from its text. A PDF embed keeps its line and draws under it, the
// way an image does, since the line is one path the typist may want to fix.
//
// A state field, like the image widgets: CodeMirror takes block widgets
// from fields only. The whole document is scanned on each change and each
// caret move; the blocks are few per note and the scan is a line walk.

import type { ReactNode } from "react";
import katex from "katex";

import { ensureSyntaxTree } from "@codemirror/language";
import {
  RangeSetBuilder,
  type EditorState,
  type Extension,
  type Line,
} from "@codemirror/state";
import { Decoration, WidgetType, type DecorationSet } from "@codemirror/view";

import { MermaidDiagram } from "../RendererComponents";
import { PdfPreview } from "../PdfPreview";
import { SavedQueryBlock } from "../SavedQueryBlock";
import { caretAwareField, caretTouches } from "./focusState";
import { assetsResolved } from "./imageWidgets";
import type { PortalRegistry } from "./widgetPortals";

export type RenderedBlocksConfig = {
  portals: PortalRegistry;
  /** An attachment path as written, turned into something the browser can load. */
  resolveAssetSrc: (raw: string) => string;
};

const FENCE_OPEN = /^ {0,3}(`{3,}|~{3,})[ \t]*([^\s`]*)/;
const FENCE_CLOSE = /^ {0,3}(`{3,}|~{3,})[ \t]*$/;
const MATH_LINE = /^\s*\$\$(.*?)\$\$\s*$/;
const MATH_OPEN = /^\s*\$\$(.*)$/;
const MATH_CLOSE = /^(.*?)\$\$\s*$/;
// One `$` pair on a line: not escaped, not a `$$`, content that neither
// starts nor ends with a space (so "$5 and $6" stays prose), no digit
// straight after the closing one.
const MATH_INLINE =
  /(?<![\\$\w])\$(?![\s$])((?:\\.|[^$\\\n])*?)(?<!\s)\$(?![\w$])/g;
const PDF_EMBED = /!\[\[([^\]|#]+\.pdf)(#[^\]|]*)?(?:\|([^\]]*))?\]\]/gi;

/** Whether an embed target names a PDF; a `?`/`#` suffix addresses the viewer. */
export function isPdfTarget(target: string): boolean {
  return /\.pdf(?:[?#].*)?$/i.test(target);
}

/** Interactive content inside a widget is the widget's, not the caret's. */
function interactive(event: Event): boolean {
  const target = event.target;
  return (
    target instanceof Element &&
    target.closest("a, button, input, select, textarea, canvas") !== null
  );
}

/** A widget whose content is a React component drawn through a portal. */
abstract class PortalWidget extends WidgetType {
  readonly portals: PortalRegistry;
  readonly className: string;

  constructor(portals: PortalRegistry, className: string) {
    super();
    this.portals = portals;
    this.className = className;
  }

  abstract content(): ReactNode;

  toDOM() {
    const host = document.createElement("div");
    host.className = `live-editor-block ${this.className}`;
    host.dataset.testid = this.className;
    this.portals.mount(host, this.content());
    return host;
  }

  destroy(dom: HTMLElement) {
    this.portals.unmount(dom);
  }

  ignoreEvent(event: Event) {
    return interactive(event);
  }
}

class MermaidWidget extends PortalWidget {
  readonly chart: string;

  constructor(portals: PortalRegistry, chart: string) {
    super(portals, "live-editor-mermaid");
    this.chart = chart;
  }

  eq(other: MermaidWidget) {
    return other.chart === this.chart;
  }

  content() {
    return <MermaidDiagram chart={this.chart} />;
  }
}

class SavedQueryWidget extends PortalWidget {
  readonly source: string;
  readonly line: number;

  constructor(portals: PortalRegistry, source: string, line: number) {
    super(portals, "live-editor-saved-query");
    this.source = source;
    this.line = line;
  }

  eq(other: SavedQueryWidget) {
    return other.source === this.source && other.line === this.line;
  }

  content() {
    return <SavedQueryBlock source={this.source} line={this.line} />;
  }
}

class PdfWidget extends PortalWidget {
  readonly src: string;
  readonly label: string;

  constructor(portals: PortalRegistry, src: string, label: string) {
    super(portals, "live-editor-pdf");
    this.src = src;
    this.label = label;
  }

  eq(other: PdfWidget) {
    return other.src === this.src && other.label === this.label;
  }

  content() {
    return <PdfPreview src={this.src} label={this.label} />;
  }
}

/**
 * KaTeX, the way rehype-katex drives it in the reading view: strict first,
 * and on an error a second pass that draws the source in KaTeX's own error
 * colour rather than leaving a gap.
 */
function renderMath(tex: string, displayMode: boolean): string {
  try {
    return katex.renderToString(tex, { displayMode, throwOnError: true });
  } catch {
    return katex.renderToString(tex, {
      displayMode,
      throwOnError: false,
      strict: "ignore",
    });
  }
}

class MathWidget extends WidgetType {
  readonly tex: string;
  readonly display: boolean;

  constructor(tex: string, display: boolean) {
    super();
    this.tex = tex;
    this.display = display;
  }

  eq(other: MathWidget) {
    return other.tex === this.tex && other.display === this.display;
  }

  toDOM() {
    const host = document.createElement(this.display ? "div" : "span");
    host.className = this.display
      ? "live-editor-block live-editor-math"
      : "live-editor-math-inline";
    host.dataset.testid = this.display
      ? "live-editor-math"
      : "live-editor-math-inline";
    host.innerHTML = renderMath(this.tex, this.display);
    return host;
  }

  ignoreEvent() {
    return false;
  }
}

type RenderedBlock = {
  from: number;
  to: number;
  decoration: Decoration;
  /** Whether a caret in `[from, to]` turns the widget back into source. */
  reveals: boolean;
};

/** The backtick spans on a line, inside which nothing is math or an embed. */
function codeSpans(text: string): Array<[number, number]> {
  const spans: Array<[number, number]> = [];
  for (const match of text.matchAll(/(`+)[^`]*?\1/g)) {
    const from = match.index ?? 0;
    spans.push([from, from + match[0].length]);
  }
  return spans;
}

function insideCode(spans: Array<[number, number]>, at: number): boolean {
  return spans.some(([from, to]) => at >= from && at < to);
}

/** Which lines sit inside a fenced code block, by the editor's own parse. */
function fencedLines(state: EditorState): {
  fenced: Set<number>;
  fences: Array<{ first: Line; last: Line; lang: string }>;
} {
  const fenced = new Set<number>();
  const fences: Array<{ first: Line; last: Line; lang: string }> = [];
  const tree = ensureSyntaxTree(state, state.doc.length, 50);
  tree?.iterate({
    enter: (node) => {
      if (node.name !== "FencedCode") {
        return;
      }
      const first = state.doc.lineAt(node.from);
      const last = state.doc.lineAt(node.to);
      for (let n = first.number; n <= last.number; n += 1) {
        fenced.add(n);
      }
      const lang = FENCE_OPEN.exec(first.text)?.[2] ?? "";
      fences.push({ first, last, lang: lang.toLowerCase() });
      return false;
    },
  });
  return { fenced, fences };
}

function blocksIn(
  state: EditorState,
  config: RenderedBlocksConfig,
): RenderedBlock[] {
  const found: RenderedBlock[] = [];
  const { fenced, fences } = fencedLines(state);

  for (const { first, last, lang } of fences) {
    if (lang !== "mermaid" && lang !== "base") {
      continue;
    }
    // The body is what sits between the fences; an unclosed fence runs to
    // the end of the document and is not a block yet.
    if (last.number === first.number || !FENCE_CLOSE.test(last.text)) {
      continue;
    }
    const body = state.sliceDoc(
      first.to + 1,
      Math.max(first.to + 1, last.from - 1),
    );
    const widget =
      lang === "mermaid"
        ? new MermaidWidget(config.portals, body)
        : new SavedQueryWidget(config.portals, body, first.number);
    found.push({
      from: first.from,
      to: last.to,
      decoration: Decoration.replace({ widget, block: true }),
      reveals: true,
    });
  }

  let n = 1;
  while (n <= state.doc.lines) {
    const line = state.doc.line(n);
    n += 1;
    if (fenced.has(line.number)) {
      continue;
    }
    const whole = MATH_LINE.exec(line.text);
    if (whole) {
      if (whole[1].trim()) {
        found.push({
          from: line.from,
          to: line.to,
          decoration: Decoration.replace({
            widget: new MathWidget(whole[1], true),
            block: true,
          }),
          reveals: true,
        });
      }
      continue;
    }
    const open = MATH_OPEN.exec(line.text);
    if (open) {
      const lines = [open[1]];
      let closed: Line | null = null;
      for (let m = line.number + 1; m <= state.doc.lines; m += 1) {
        const candidate = state.doc.line(m);
        if (fenced.has(m)) {
          break;
        }
        const close = MATH_CLOSE.exec(candidate.text);
        if (close) {
          lines.push(close[1]);
          closed = candidate;
          break;
        }
        lines.push(candidate.text);
      }
      if (closed) {
        found.push({
          from: line.from,
          to: closed.to,
          decoration: Decoration.replace({
            widget: new MathWidget(lines.join("\n"), true),
            block: true,
          }),
          reveals: true,
        });
        n = closed.number + 1;
      }
      continue;
    }
    const spans = codeSpans(line.text);
    for (const match of line.text.matchAll(MATH_INLINE)) {
      if (insideCode(spans, match.index ?? 0)) {
        continue;
      }
      const from = line.from + (match.index ?? 0);
      found.push({
        from,
        to: from + match[0].length,
        decoration: Decoration.replace({
          widget: new MathWidget(match[1], false),
        }),
        reveals: true,
      });
    }
    for (const match of line.text.matchAll(PDF_EMBED)) {
      if (insideCode(spans, match.index ?? 0)) {
        continue;
      }
      const target = match[1].trim();
      const label = match[3]?.trim() || target.split("/").pop() || "PDF";
      found.push({
        from: line.to,
        to: line.to,
        decoration: Decoration.widget({
          widget: new PdfWidget(
            config.portals,
            config.resolveAssetSrc(target + (match[2] ?? "")),
            label,
          ),
          block: true,
          side: 1,
        }),
        reveals: false,
      });
    }
  }

  return found.sort((a, b) => a.from - b.from || a.to - b.to);
}

function decorate(
  state: EditorState,
  blocks: RenderedBlock[],
  focused: boolean,
): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const block of blocks) {
    if (block.reveals && focused && caretTouches(state, block.from, block.to)) {
      continue;
    }
    builder.add(block.from, block.to, block.decoration);
  }
  return builder.finish();
}

export function renderedBlocks(config: RenderedBlocksConfig): Extension {
  return caretAwareField<RenderedBlock[]>({
    find: (state) => blocksIn(state, config),
    decorate,
    refindOn: assetsResolved,
  });
}

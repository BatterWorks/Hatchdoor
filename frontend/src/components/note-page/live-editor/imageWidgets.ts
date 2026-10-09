// Draws each image below the line that embeds it. The source line stays as
// written, so a Vault-relative path keeps pointing at the file; the widget is
// where the path is turned into the asset route the browser can load.
//
// Covers `![alt](path)` and Obsidian's `![[path]]`. Other embeds (PDFs,
// notes) stay as source here and render in the reading view.
//
// A state field rather than a view plugin: CodeMirror only accepts block
// widgets from a field, so the whole document is scanned on each change
// rather than the viewport. Images are rare enough per note for that to be
// cheap.

import { ensureSyntaxTree } from "@codemirror/language";
import {
  RangeSetBuilder,
  StateField,
  type EditorState,
  type Extension,
} from "@codemirror/state";
import {
  Decoration,
  EditorView,
  WidgetType,
  type DecorationSet,
} from "@codemirror/view";

const IMAGE_FILE = /\.(png|jpe?g|gif|webp|svg|avif|bmp)$/i;
const WIKI_EMBED = /!\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|[^\]]*)?\]\]/g;
const MARKDOWN_IMAGE = /^!\[([^\]]*)\]\(\s*<?([^)\s>]+)>?/;

class ImageWidget extends WidgetType {
  readonly src: string;
  readonly alt: string;

  constructor(src: string, alt: string) {
    super();
    this.src = src;
    this.alt = alt;
  }

  eq(other: ImageWidget) {
    return other.src === this.src && other.alt === this.alt;
  }

  toDOM() {
    const figure = document.createElement("div");
    figure.className = "live-editor-image";
    const img = document.createElement("img");
    img.src = this.src;
    img.alt = this.alt;
    img.loading = "lazy";
    figure.appendChild(img);
    return figure;
  }

  ignoreEvent() {
    return false;
  }
}

function imagesIn(
  state: EditorState,
  resolve: (raw: string) => string,
): DecorationSet {
  const found: Array<{ at: number; src: string; alt: string }> = [];
  const tree = ensureSyntaxTree(state, state.doc.length, 50);
  tree?.iterate({
    enter: (node) => {
      if (node.name !== "Image") {
        return;
      }
      const parsed = MARKDOWN_IMAGE.exec(state.sliceDoc(node.from, node.to));
      if (parsed) {
        found.push({
          at: state.doc.lineAt(node.to).to,
          src: resolve(parsed[2]),
          alt: parsed[1],
        });
      }
    },
  });
  for (let n = 1; n <= state.doc.lines; n += 1) {
    const line = state.doc.line(n);
    for (const match of line.text.matchAll(WIKI_EMBED)) {
      const target = match[1].trim();
      if (IMAGE_FILE.test(target)) {
        found.push({ at: line.to, src: resolve(target), alt: target });
      }
    }
  }
  found.sort((a, b) => a.at - b.at);
  const builder = new RangeSetBuilder<Decoration>();
  for (const image of found) {
    builder.add(
      image.at,
      image.at,
      Decoration.widget({
        widget: new ImageWidget(image.src, image.alt),
        block: true,
        side: 1,
      }),
    );
  }
  return builder.finish();
}

export function imageWidgets(resolve: (raw: string) => string): Extension {
  return StateField.define<DecorationSet>({
    create: (state) => imagesIn(state, resolve),
    update: (value, tr) =>
      tr.docChanged ? imagesIn(tr.state, resolve) : value,
    provide: (field) => EditorView.decorations.from(field),
  });
}

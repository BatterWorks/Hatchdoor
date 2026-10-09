// One CodeMirror 6 editor over the whole note body, Obsidian Live Preview
// style (#540, #541). Markdown syntax shows on the caret line and renders
// everywhere else; the document is the file's text and nothing re-serializes
// it.
//
// The view is uncontrolled: it owns the text between commits and reports
// every change up. `value` is reconciled only when it differs from the
// document, which is how an undo, a conflict resolution or a reload reaches
// an open editor without resetting the caret on every keystroke.

import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";

import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import {
  markdown,
  markdownKeymap,
  markdownLanguage,
} from "@codemirror/lang-markdown";
import { indentOnInput } from "@codemirror/language";
import { search, searchKeymap } from "@codemirror/search";
import {
  EditorSelection,
  EditorState,
  Prec,
  Transaction,
} from "@codemirror/state";
import {
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightSpecialChars,
  keymap,
} from "@codemirror/view";
import {
  atomicEditorTheme,
  atomicMarkdownSyntax,
  autoCloseCodeFence,
  extendEmphasisPair,
  highlightMarkdown,
  inlinePreview,
  startAsteriskList,
  tables,
  wikiLinks,
} from "@atomic-editor/editor";
import "@atomic-editor/editor/styles.css";

import type { ExplorerNote } from "../../../types";
import type { UploadedAttachment } from "../../NoteEditor";
import { attachmentRejection } from "../attachmentDrop";
import { callouts } from "./callouts";
import { markKeymap } from "./commands";
import { focusTracking } from "./focusState";
import { assetsResolved, imageWidgets, isImageTarget } from "./imageWidgets";
import { KeyboardBar } from "./KeyboardBar";
import { completionMenus, selectionToolbar } from "./menus";
import { isPdfTarget, renderedBlocks } from "./renderedBlocks";
import {
  findSearchHits,
  searchHighlight,
  type SearchHit,
} from "./searchHighlight";
import { WidgetPortals } from "./WidgetPortals";
import { createPortalRegistry } from "./widgetPortals";

export type LiveEditorHandle = {
  focus: () => void;
  /** Leaves the editor, which commits whatever was typed. */
  blur: () => void;
  /** Scroll a 1-based body line to the top of the pane. */
  scrollToLine: (line: number) => void;
  /** Select and centre the n-th search hit. */
  scrollToHit: (index: number) => void;
};

export type LiveEditorProps = {
  /** The note body with `\n` line endings. */
  value: string;
  /** The `?q=` query to mark; empty for none. */
  searchQuery: string;
  /** Coarse pointer: keyboard bar instead of the floating toolbar. */
  touch: boolean;
  noteCandidates: ExplorerNote[];
  /** The link text `[[` completion writes for a chosen note (ADR-33). */
  formatNoteLink: (note: ExplorerNote) => string;
  /** A wikilink's target, as written, when it is clicked. */
  onOpenNote: (target: string) => void;
  /**
   * Where a wikilink points, for the resolved/missing styling; null when
   * unknown. Asked once per target as links come into view, so it may go to
   * the server (#544).
   */
  resolveNote: (
    target: string,
  ) => Promise<{ label: string; missing: boolean } | null>;
  /** An attachment path as written, turned into something the browser can load. */
  resolveAssetSrc: (raw: string) => string;
  /**
   * The body the page's asset resolution has settled for. Each change draws
   * every embed again through `resolveAssetSrc`, which by then knows the
   * server's answers (#544).
   */
  assetsResolvedFor?: string;
  /** Every change, for the idle flush and the draft. */
  onChange: (body: string) => void;
  /** Leaving the editor with changes: the document to save. */
  onCommit: (body: string) => void;
  onFocusChange?: (focused: boolean) => void;
  onSearchHits?: (count: number) => void;
  onUploadAttachment?: (file: File) => Promise<UploadedAttachment>;
  /** A rejected or failed upload, as a sentence for the notice strip. */
  onUploadNotice?: (message: string) => void;
  onUploadError?: (error: unknown) => void;
};

export const LiveEditor = forwardRef<LiveEditorHandle, LiveEditorProps>(
  function LiveEditor(props, ref) {
    const hostRef = useRef<HTMLDivElement | null>(null);
    const viewRef = useRef<EditorView | null>(null);
    const [focused, setFocused] = useState(false);
    // Handlers are rebuilt on every render of the page; the view is built
    // once. Reading them through a ref keeps the extensions bound to the
    // current ones without rebuilding the editor under the caret.
    const propsRef = useRef(props);
    propsRef.current = props;
    const committedRef = useRef(props.value);
    const applyingExternalRef = useRef(false);
    const hitsRef = useRef<SearchHit[]>([]);
    const highlightRef = useRef(searchHighlight());
    // The rendered blocks draw the reading view's components into their
    // widgets through this registry (#544).
    const portalsRef = useRef(createPortalRegistry());

    const reportHits = useCallback((doc: string, query: string) => {
      const hits = findSearchHits(doc, query);
      hitsRef.current = hits;
      propsRef.current.onSearchHits?.(hits.length);
    }, []);

    const insertAtCaret = useCallback(
      (view: EditorView, text: string, pos?: number) => {
        const at = pos ?? view.state.selection.main.head;
        const line = view.state.doc.lineAt(at);
        // An embed takes a line of its own, after whatever the caret sits in.
        const insert = line.length === 0 ? text : `\n${text}`;
        const from = line.length === 0 ? line.from : line.to;
        view.dispatch({
          changes: { from, insert },
          selection: { anchor: from + insert.length },
          userEvent: "input",
        });
      },
      [],
    );

    const uploadFile = useCallback(
      async (view: EditorView, file: File, pos?: number) => {
        const { onUploadAttachment, onUploadNotice, onUploadError } =
          propsRef.current;
        if (!onUploadAttachment) {
          return;
        }
        const rejection = attachmentRejection(file);
        if (rejection) {
          onUploadNotice?.(rejection);
          return;
        }
        try {
          const { embed } = await onUploadAttachment(file);
          if (viewRef.current === view) {
            insertAtCaret(view, embed, pos);
          }
        } catch (error) {
          onUploadError?.(error);
        }
      },
      [insertAtCaret],
    );

    useEffect(() => {
      const host = hostRef.current;
      if (!host) {
        return;
      }
      const openNote = (target: string) => propsRef.current.onOpenNote(target);
      const openExternal = (url: string) =>
        window.open(url, "_blank", "noopener,noreferrer");
      const view = new EditorView({
        parent: host,
        state: EditorState.create({
          doc: propsRef.current.value,
          extensions: [
            highlightSpecialChars(),
            history(),
            drawSelection(),
            dropCursor(),
            indentOnInput(),
            highlightActiveLine(),
            closeBrackets(),
            startAsteriskList,
            extendEmphasisPair,
            autoCloseCodeFence,
            EditorView.lineWrapping,
            search({ top: true }),
            // GitHub-flavored, to match the reading view's remark-gfm.
            markdown({ base: markdownLanguage, extensions: highlightMarkdown }),
            markdownLanguage.data.of({
              closeBrackets: {
                brackets: ["(", "[", "{", "'", '"', "*", "_", "`"],
              },
            }),
            atomicMarkdownSyntax,
            atomicEditorTheme,
            markKeymap,
            keymap.of([
              ...closeBracketsKeymap,
              ...historyKeymap,
              ...searchKeymap,
              ...markdownKeymap,
              ...defaultKeymap,
            ]),
            // Escape commits and leaves. Under autosave there is nothing to
            // cancel back to.
            Prec.highest(
              keymap.of([
                {
                  key: "Escape",
                  run: (v) => {
                    v.contentDOM.blur();
                    return true;
                  },
                },
              ]),
            ),
            tables({ onLinkClick: openExternal }),
            inlinePreview({ onLinkClick: openExternal }),
            wikiLinks({
              onOpen: openNote,
              // An attachment embed is drawn by its own widget under the
              // line; its target is a file, not a note to resolve or open.
              shouldResolve: (target) =>
                !isImageTarget(target) && !isPdfTarget(target),
              resolve: async (target) => {
                const resolved = await propsRef.current.resolveNote(target);
                return resolved
                  ? {
                      target,
                      label: resolved.label,
                      status: resolved.missing ? "missing" : "resolved",
                    }
                  : null;
              },
            }),
            focusTracking,
            imageWidgets((raw) => propsRef.current.resolveAssetSrc(raw)),
            renderedBlocks({
              portals: portalsRef.current,
              resolveAssetSrc: (raw) => propsRef.current.resolveAssetSrc(raw),
            }),
            callouts,
            completionMenus(
              () => propsRef.current.noteCandidates,
              (note) => propsRef.current.formatNoteLink(note),
            ),
            propsRef.current.touch ? [] : selectionToolbar,
            highlightRef.current.extension,
            EditorView.contentAttributes.of({
              "aria-label": "Note body",
              spellcheck: "true",
            }),
            EditorView.updateListener.of((update) => {
              if (!update.docChanged || applyingExternalRef.current) {
                return;
              }
              const doc = update.state.doc.toString();
              propsRef.current.onChange(doc);
              reportHits(doc, propsRef.current.searchQuery);
            }),
            EditorView.domEventHandlers({
              focus: () => {
                setFocused(true);
                propsRef.current.onFocusChange?.(true);
                return false;
              },
              blur: (_event, v) => {
                setFocused(false);
                propsRef.current.onFocusChange?.(false);
                const doc = v.state.doc.toString();
                if (doc !== committedRef.current) {
                  committedRef.current = doc;
                  propsRef.current.onCommit(doc);
                }
                return false;
              },
              paste: (event, v) => {
                const file = Array.from(event.clipboardData?.files ?? [])[0];
                if (!file) {
                  return false;
                }
                event.preventDefault();
                void uploadFile(v, file);
                return true;
              },
              drop: (event, v) => {
                const file = event.dataTransfer?.files[0];
                if (!file) {
                  return false;
                }
                event.preventDefault();
                const pos = v.posAtCoords({
                  x: event.clientX,
                  y: event.clientY,
                });
                void uploadFile(v, file, pos ?? undefined);
                return true;
              },
            }),
          ],
        }),
      });
      viewRef.current = view;
      highlightRef.current.reconfigure(view, propsRef.current.searchQuery);
      reportHits(propsRef.current.value, propsRef.current.searchQuery);
      return () => {
        view.destroy();
        viewRef.current = null;
      };
      // Mount-only: the document is seeded once and reconciled below.
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // An external document (undo of a save, a reload, a conflict resolution)
    // replaces the text in place, keeping the caret where it still fits.
    useEffect(() => {
      const view = viewRef.current;
      if (!view) {
        return;
      }
      const current = view.state.doc.toString();
      if (current === props.value) {
        committedRef.current = props.value;
        return;
      }
      applyingExternalRef.current = true;
      try {
        const head = Math.min(
          view.state.selection.main.head,
          props.value.length,
        );
        // Outside undo history: a Ctrl+Z straight after a reload or a conflict
        // resolution must not bring the replaced text back and autosave it.
        view.dispatch({
          changes: { from: 0, to: current.length, insert: props.value },
          selection: EditorSelection.cursor(head),
          userEvent: "external",
          annotations: Transaction.addToHistory.of(false),
        });
      } finally {
        applyingExternalRef.current = false;
      }
      committedRef.current = props.value;
      reportHits(props.value, props.searchQuery);
    }, [props.value, props.searchQuery, reportHits]);

    useEffect(() => {
      const view = viewRef.current;
      if (!view) {
        return;
      }
      highlightRef.current.reconfigure(view, props.searchQuery);
      reportHits(view.state.doc.toString(), props.searchQuery);
    }, [props.searchQuery, reportHits]);

    useEffect(() => {
      viewRef.current?.dispatch({ effects: assetsResolved.of() });
    }, [props.assetsResolvedFor]);

    useImperativeHandle(
      ref,
      () => ({
        focus: () => viewRef.current?.focus(),
        blur: () => viewRef.current?.contentDOM.blur(),
        scrollToLine: (line) => {
          const view = viewRef.current;
          if (!view) {
            return;
          }
          const clamped = Math.max(1, Math.min(line, view.state.doc.lines));
          const pos = view.state.doc.line(clamped).from;
          view.dispatch({
            effects: EditorView.scrollIntoView(pos, {
              y: "start",
              yMargin: 24,
            }),
          });
        },
        scrollToHit: (index) => {
          const view = viewRef.current;
          const hit = hitsRef.current[index];
          if (!view || !hit) {
            return;
          }
          view.dispatch({
            selection: EditorSelection.range(hit.from, hit.to),
            effects: EditorView.scrollIntoView(hit.from, { y: "center" }),
          });
        },
      }),
      [],
    );

    return (
      <div className="live-editor">
        {/* `atomic-cm-editor` is the class the library's stylesheet keys its
            custom properties and line styling on. */}
        <div ref={hostRef} className="live-editor-host atomic-cm-editor" />
        <WidgetPortals registry={portalsRef.current} />
        {props.touch && focused ? (
          <KeyboardBar getView={() => viewRef.current} />
        ) : null}
      </div>
    );
  },
);

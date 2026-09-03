# Frontend Assessment

Point-in-time record, 2026-08-30, taken on `development` at `f3c8d52` (after the Lean Hatchdoor programme merged). It answers four questions that came up while reviewing the frontend: what the stack is, how much of it is bespoke, whether the line counts are earned, and where the structural risk sits. Numbers are reproducible with the commands at the end.

## Verdict

The frontend is a deliberately narrow set of libraries (React, router, CodeMirror 6, the unified/remark pipeline, d3-force, two lazy-loaded embed renderers) with everything user-facing written by hand. The line counts are mostly earned by features the project chose not to outsource. The structural cost is concentration rather than volume: three components over 1,200 lines each hold most of the orchestration, and those are where changes get risky.

Do not replace the block editor with an off-the-shelf WYSIWYG editor. Do split the three large components when a task next touches them, along the seams named below.

## Stack

Build and tooling: Vite 8, TypeScript 6, React 19, `react-router-dom` 7, Vitest 4 with Testing Library under jsdom (70% line coverage gate in `frontend/vite.config.ts`), ESLint 10, Prettier. Shipped as a PWA through `vite-plugin-pwa` with an autoUpdate service worker; `/api`, `/vault-assets` and `/health` are denylisted from the SPA navigation fallback so server routes reach the network. Vendor chunks for react, markdown and KaTeX are split by hand in `vite.config.ts`.

Runtime dependencies, complete list from `frontend/package.json`:

| Concern | Library |
| --- | --- |
| Markdown rendering | `react-markdown`, `remark-gfm`, `remark-math`, `rehype-katex`, `katex` |
| Editing | CodeMirror 6 (`@codemirror/*`, `@lezer/markdown`, `@lezer/highlight`) |
| Graph physics | `d3-force` (layout only, not rendering) |
| Embeds | `mermaid`, `pdfjs-dist`, both dynamically imported |
| Routing | `react-router-dom` |

Absent by choice: no component library (MUI, shadcn, Radix, Chakra), no Tailwind or CSS-in-JS, no state library, no data-fetching library, no form library, no i18n.

## How custom it is

- Styling: ~6.3k lines of plain CSS under `frontend/src/styles/` and two feature-local files, 57 custom properties as design tokens, four typefaces from Google Fonts (Bricolage Grotesque display, Newsreader serif, Inter Tight sans, JetBrains Mono). The UI kit is one 138-line `components/ui.tsx` (`UiButton`, `UiPanel`, `UiToolbar`, `StatusBadge`, skeletons) plus a hand-rolled `icons.tsx`.
- State: React hooks, one `createContext` for the inline editor, and a `useSyncExternalStore` store for the Vault collection (`frontend/src/vaults/vaultCollectionStore.ts`). No global store.
- API client: a single `fetch` call site in `frontend/src/api/api.ts`, with `writeApi.ts` and `apiError.ts` layered on top.
- Editor: the block editor in `components/note-page/` (31 files) plus `NotePage.tsx` and the textarea-based `NoteEditor.tsx`. See the editor section below.
- Graph: `components/graph/GraphPage.tsx` draws to a raw `<canvas>` 2D context with its own hit-testing, pan, zoom and pinch; d3 supplies only the force layout.
- Domain logic: 18 dependency-free modules in `frontend/src/lib/` (block ops, edit history, caret and source maps, clipboard, image upload, note search, paths, drafts) carrying most of the test coverage.

## Size

Everything under `frontend/`, excluding `node_modules`, `dist`, `package-lock.json` (12.4k lines on its own) and binary icons:

| Category | Lines | Files |
| --- | ---: | ---: |
| Application TS/TSX (non-test) | 20,973 | 86 |
| Test files (`*.test.ts(x)`) | 18,657 | 70 |
| Test harness (`src/test/`) | 317 | 2 |
| CSS | 6,328 | 14 |
| Config and tooling | 451 | 11 |
| Total | ~46,700 | 183 |

Tests are 41% of the frontend, a 0.9:1 test-to-app ratio.

Of the 20,973 application lines, 17,219 are code, 2,319 are comments (11%) and 1,435 are blank.

Application lines by area:

| Area | Lines |
| --- | ---: |
| Note editor (`components/note-page/`) | 3,792 |
| Settings (`features/settings/`) | 3,206 |
| Pure domain lib (`lib/`) | 2,170 |
| App shell (`app/`: topbar, explorer pane, vault slots) | 1,859 |
| Graph (`components/graph/`) | 1,753 |
| `components/NotePage.tsx` | 1,232 |
| `App.tsx` | 1,214 |
| Hooks | 848 |
| Search (`features/search/`) | 821 |
| `components/NoteEditor.tsx` (textarea mode) | 571 |
| `components/StatsPage.tsx` | 550 |
| `components/NoteActionsDialog.tsx` | 496 |
| Vault collection store | 399 |
| `types.ts` | 397 |
| API client | 393 |
| `components/Explorer.tsx` | 349 |
| Startup gate | 345 |

## Are the line counts earned

### Application code

Yes, for the most part. With almost no library doing the work, 17k code lines buy: a block-based Markdown editor with per-block CodeMirror instances, caret and source mapping, wikilink autocomplete, autosave with drafts, conflict diffing, attachment drop, PDF, Mermaid and KaTeX embeds; a canvas force graph with its own interaction model; search; a multi-vault store with optimistic concurrency; a startup and auth gate; PWA support; 8 dialogs, 31 keyboard handlers and 173 `aria-*` attributes. A block editor and a graph view are each several thousand lines in any codebase. The comment share is healthy rather than padding.

Where the criticism does land:

1. Concentration. `NotePage.tsx`, `App.tsx` and `GraphPage.tsx` are each over 1,200 lines. See the next section.
2. Settings at 3.2k lines feels heavy for vault creation, git behaviour and drafts. `VaultSettingsIndex.tsx` alone is 1,046 lines with 23 `useState` calls.
3. `NoteEditor.tsx` (571 lines) sits next to the block editor. It is the full-source textarea mode, which is a legitimate second mode, not a leftover, but the two share wikilink autocomplete and frontmatter handling and should be checked for drift when either changes.

### CSS

The 6,328 lines are 816 rules at about 7.7 lines per rule, which is Prettier-formatted one-declaration-per-line CSS. Removing 765 blank and 289 comment lines leaves about 5.3k lines of declarations.

- 0 of 446 class selectors are unreferenced (heuristic: the class token appears nowhere in TS/TSX; dynamically composed names could hide a false negative).
- No dark-theme duplication: 5 theme-related hits in total.
- 104 rules (about 13%) sit under 36 `@media` blocks; that is the mobile support.
- The two deliberate choices that cost lines are the absence of a component library (dialogs, panels, toolbars, skeletons and badges are styled from scratch) and the editorial design (`font-variation-settings` appears 50 times). Tailwind would move these lines into `className` strings rather than remove them. A component library would save perhaps 500 to 800 lines and add a dependency and a generic look, which the Lean programme explicitly declined.

Per file:

| File | Lines | Rules |
| --- | ---: | ---: |
| `styles/note-content.css` | 1,297 | 160 |
| `features/settings/settings.css` | 892 | 121 |
| `styles/layout-explorer.css` | 698 | 87 |
| `styles/stats.css` | 566 | 78 |
| `styles/topbar.css` | 482 | 49 |
| `App.css` | 454 | 61 |
| `styles/graph.css` | 405 | 53 |
| `features/search/search.css` | 370 | 46 |
| `noteEnhancements.css` | 299 | 42 |
| `styles/base.css` | 243 | 16 |
| `styles/startup.css` | 228 | 40 |
| `styles/responsive.css` | 190 | 35 |
| `styles/ui-common.css` | 179 | 23 |
| `index.css` | 25 | 5 |

Where the criticism lands: `App.css` and `noteEnhancements.css` sit at `src/` root outside the `styles/` scheme and look like accretion. Repeated micro-patterns (`color: var(--muted)` 81 times, `text-transform: uppercase` 50 times, `font-variation-settings` 50 times) suggest an eyebrow-label utility class could fold a few hundred lines.

## How the size compares with other frontends

Measured 2026-08-30 with one counter on shallow clones of each project's frontend directory at its default branch. App lines are `.ts/.tsx/.js/.jsx/.vue/.svelte` excluding tests, `.d.ts`, `node_modules`, build output and obvious vendored code (SiYuan's `asset/` pdf.js viewer, 23.5k lines, and Excalidraw's generated woff2 bindings, 4k lines, were subtracted). Tests are files under `.test.`, `.spec.`, `__tests__`, `test(s)/`, `e2e/`, `fixtures/`. The counts are crude: Vue single-file components include template and style, and CSS-in-JS projects (Outline, Actual) carry their styling inside app lines.

Self-hosted note and knowledge apps:

| Frontend | Stack | App | Tests | CSS | Tests/app | Source of truth |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| Flatnotes `client/` | Vue, PrimeVue, Tailwind, Toast UI editor | 2.4k | 0 | 0.3k | 0.00 | `.md` files |
| Hatchdoor `frontend/` | React, no UI kit, CodeMirror 6 + react-markdown | 21.2k | 19.0k | 6.3k | 0.89 | `.md` files |
| HedgeDoc `frontend/` | Next/React, Bootstrap, CodeMirror 6 source + preview | 35.6k | 11.4k | 5.0k | 0.32 | markdown in DB |
| Memos `web/` | React, Tailwind, CodeMirror 6 | 40.3k | 14.8k | 0.5k | 0.37 | markdown in DB |
| SilverBullet `client/` | Preact, no UI kit, custom CodeMirror 6 live preview | 63.4k | 28.4k | 4.8k | 0.45 | `.md` files |
| Docmost `apps/client/` | React, Mantine, TipTap | 85.0k | 0.5k | 9.1k | 0.01 | JSON in DB |
| Outline `app/` + `shared/editor/` | React, Radix, styled-components, ProseMirror | 138.5k | 6.8k | 0 | 0.05 | ProseMirror doc in DB |
| Trilium `apps/client/` | Preact/jQuery, Bootstrap, CKEditor 5 fork | 127.5k | 83.9k | 34.1k | 0.66 | HTML in DB |
| SiYuan `app/src/` | Vanilla TS, custom Protyle editor | 185.8k | 21.7k | 18.5k | 0.12 | `.sy` JSON |

General open-source frontends, for scale:

| Frontend | Stack | App | Tests | CSS | Tests/app |
| --- | --- | ---: | ---: | ---: | ---: |
| Uptime Kuma `src/` | Vue, Bootstrap | 34.5k | 0 | 0.8k | 0.00 |
| Immich `web/` | Svelte, Tailwind | 60.2k | 6.2k | 0.2k | 0.10 |
| Excalidraw `packages/excalidraw/` | React, own components | 79.9k | 37.8k | 9.4k | 0.47 |
| Actual `packages/desktop-client/` | React, Emotion | 152.2k | 24.2k | 0 | 0.16 |

Editor layers only, same counter, to size the bespoke part of each project:

| Editor layer | Built on | App lines |
| --- | --- | ---: |
| Flatnotes | Toast UI, used as-is | ~0 |
| Hatchdoor `components/note-page/` (block-editor glue alone, including its `lib/` modules) | CodeMirror 6 + react-markdown | 3.8k (2.1k) |
| Memos `MemoEditor/` | CodeMirror 6 source mode | 5.9k |
| SilverBullet `client/codemirror/` (+ `markdown_parser/`) | CodeMirror 6 live preview | 7.9k (+1.7k) |
| HedgeDoc `components/editor-page/` | CodeMirror 6 source + split preview | 9.6k |
| Docmost `features/editor/` | TipTap | 17.9k |
| Trilium `widgets/type_widgets/` | CKEditor 5 fork | 34.2k |
| Outline `app/editor/` + `shared/editor/` | ProseMirror | 45.0k |
| SiYuan `protyle/` | Own block editor | 86.6k |

What the comparison says:

1. Hatchdoor's 21k application lines are the second smallest in the sample. Only Flatnotes is smaller, and it is a much simpler product (single folder, no graph, no multi-vault, an off-the-shelf WYSIWYG that rewrites files on save). The comparable note apps cluster between 35k and 190k; the general frontends between 35k and 150k.
2. The test ratio of 0.89 is the highest in the sample. The next are Trilium at 0.66 and SilverBullet and Excalidraw at about 0.45; Docmost and Outline ship with almost none. Of the 46.7k lines that read as "a lot", 19k is a testing investment most peers do not make. App plus CSS alone is 27.5k, below HedgeDoc (40.6k) and Memos (40.8k).
3. 6.3k lines of CSS is mid-pack. The near-zero CSS projects use Tailwind classes in templates or CSS-in-JS, so their styling is counted as app code, not absent.
4. Adopting an editor library does not make the editor layer small. Docmost carries 17.9k lines around TipTap, Outline 45k around ProseMirror, Trilium 34k around CKEditor. SilverBullet's CodeMirror live preview is 7.9k. Hatchdoor's entire `note-page/` is 3.8k and the block-editor glue 2.1k, the smallest bespoke editor layer in the sample apart from Flatnotes' zero.

## The note editor: why it is custom and whether it should stay so

There are two editors. `NoteEditor.tsx` is a plain `<textarea>` holding the whole file, with frontmatter fields and wikilink autocomplete. The block editor in `components/note-page/` keeps the note rendered and swaps one clicked block into a small CodeMirror instance; leaving the block turns it back into rendered text. Saves are whole-file `PUT` with an expected content hash (`frontend/src/api/writeApi.ts`, `updateNote`), so the backend never sees blocks. The block model exists only to decide which lines of the file to replace.

### Why custom

The constraint that forces it is that the file on disk is shared with Obsidian, git and MCP clients, and every one of them expects the bytes it did not touch to stay as they were. `frontend/src/lib/blockOps.ts` opens with the statement that an off-by-one there writes to the wrong line of the user's file; that is the whole design pressure in one sentence.

Off-the-shelf rich editors (TipTap/ProseMirror, Lexical, Slate, Milkdown) keep their own document model and re-serialise the entire file on save. That normalises list markers, blank lines and emphasis characters, and mangles or drops anything the model does not understand: Obsidian block IDs, callouts, aliased wikilinks, embeds. This is the round-trip problem, and it rules out the whole category.

What is bespoke is the glue, not the text engine:

- a source map from rendered blocks to file lines, with a runtime `linesMatch` check that every render-side transform preserved line counts (`frontend/src/lib/sourceMap.ts`);
- a caret map from a click in rendered text to an offset in the markdown, approximate by design (`frontend/src/lib/caretMap.ts`);
- Enter and Backspace split-and-merge rules that refuse to break a code fence or table row (`components/note-page/BlockInput.tsx`).

CodeMirror owns typing, IME composition, the mobile virtual keyboard, selection and per-block undo. The header of `BlockInput.tsx` explains why a textarea with a styled mirror layer was rejected: bold glyphs are wider than the regular ones the invisible text is measured in, so the caret drifts. `react-markdown` owns parsing and rendering.

### Should it stay custom

Yes, for as long as the rule "never rewrite lines the user did not edit" holds, and that rule is not negotiable while Obsidian and git share the files. Replacing the editor with a WYSIWYG library would look like a win until the first report that Hatchdoor reformatted a note.

The credible alternative is the Obsidian-style live preview: the whole note in a single CodeMirror instance with decorations hiding syntax away from the caret. It would remove the source map, the caret map and the split logic, and round-trip would be exact by construction. Packaged versions of this exist as of 2026, both young and single-maintainer: `@atomic-editor/editor` (React, created April 2026, 0.6.x, 135 stars, about 15k downloads a month, covers headings, emphasis, highlights, links, images, tables, task lists, code and wikilinks with async resolution) and `codemirror-live-markdown` (framework-free CM6 extensions, created January 2026, 0.5 alpha, last pushed March 2026, covers math but not wikilinks). Hatchdoor renders 13 Obsidian constructs; atomic-editor covers 7, so embeds, callouts, block IDs, math, Mermaid, PDF and footnotes would become hand-written CM6 widgets on top of it. Against the switch: it is a 0.x dependency at the centre of the product one month after the Lean programme, its own design notes list touch caret placement in hidden markup as a hard problem, and tables, Mermaid, KaTeX and PDFs render worse as CodeMirror decorations than as real HTML, which is where the current design is strongest. Neither library existed as a mature option; both are worth re-checking in a year.

There is also a boring option that the round-trip requirement permits on its own: no in-place editing at all. Keep the rendered read view, make edit mode a single whole-note CodeMirror instance in source mode (the highlighting and theme in `blockEditorSetup.ts` carry over), and delete the block editor: about 2.1k application lines (`BlockInput`, `EditableBlock`, `InlineEditorProvider`, `blockOps`, `sourceMap`, `caretMap`, `linePrefix`), about 3k test lines and the `linesMatch` invariant. The cost is that read and edit become two modes, and editing a long note on a phone means scrolling a raw markdown buffer. Whether that trade is right depends on whether browser editing is a convenience next to Obsidian or a headline feature; that product decision is not recorded anywhere and should become an ADR either way, since the block editor's rationale currently lives only in code comments that cite a decision list (D8, D9, D27, D28) not present in `docs/adr/`.

Tripwire: the block model depends on `parseFrontmatter`, `stripBlockIds` and `resolveWikilinks` keeping line counts identical, enforced at runtime by `linesMatch`. If a future feature needs a render-side transform that adds or removes lines, this design cracks. That is the moment to revisit the live-preview alternative, not before.

## The three large components

A "god component" is one React function doing every job on a screen, so any change means reading the whole thing to be sure nothing unrelated was knocked over. Three files fit the description.

### `App.tsx` (1,214 lines)

The real weight is `VaultWorkspace`, about 970 lines of one function holding 17 state values, 18 effects and consuming 13 hooks. By job:

- layout engine: drawer, sidebar width, mobile drawer position, on-screen keyboard height (`visualViewportHeight`), collapsed sections;
- router: every `<Route>` lives here;
- navigation memory: active note, recent notes, expanded folders, scroll restoration;
- dialog host: note actions, scope sheet, start-with-no-vaults, token prompt;
- cross-component messaging: `editRequestId` and `scopeFocusRequestId` are counters that increment so a distant child knows a shortcut fired, which is the parent acting as the only phone line between two children;
- odd jobs: online detection, screen-reader announcements, auth-required state.

Split shape: a responsive-layout hook, a navigation-memory hook, a keyboard-shortcut hook, and a dialogs host. Routes stay.

### `components/NotePage.tsx` (1,232 lines)

27 state values, 14 effects, 10 refs. By job:

- loading the note: note, links, loading, error;
- two editing sessions: the textarea mode (`isEditing`, `draftContent`, `editBaseHash`) and the block mode (`inlineDirty`, `activeUnit`, `activeRange`);
- autosave and drafts: draft notices, stale drafts, saving, demo-mode refusal;
- conflict handling: `conflict`, `conflictNote`, `noteChangedOnDisk`, `externalChange`;
- find-in-note from the URL query: hit count and active hit;
- small UI state: properties collapsed, touch edit hint, drop zone, tail armed.

The dangerous mix is editing mode, autosave and conflict handling, which interact and are all loose variables in one scope, so each effect has to be read against the others.

Split shape: one `useNoteSession` hook owning load, edit mode, autosave and conflict, leaving `NotePage` as the view; find-in-note as its own hook.

### `components/graph/GraphPage.tsx` (1,405 lines)

11 state values but 20 refs, which is the tell. React state covers the visible chrome (filters, counts, loading, island mode). The refs are a separate mutable world React does not manage: simulation nodes and links, camera transform, hover, selection, drag, pan, pinch, zoom animation, the `requestAnimationFrame` loop and theme colours read from CSS variables.

What is really in the file is a small engine: physics via d3-force, a canvas renderer, an input system for mouse, touch and pinch, a camera with fit-to-view animation, and the islands layout. It lives inside a React component because that is where it started. Its internals cannot be exercised under jsdom, which is why CodeGraph reports no covering tests for them.

Split shape: move the engine into a plain class with `mount(canvas)`, `setData`, `setTheme` and `destroy`, with no React inside. The component shrinks to a couple of hundred lines and the engine becomes testable without a browser. `graphSimulation.ts` already exists, so the seam is half cut. This is the highest-value split of the three.

### Why this is less bad than it sounds

The domain logic already lives in `frontend/src/lib/` as pure, tested functions. The three large files are mostly wiring. The risk is "changed the layout and broke conflict handling", not "the core logic is untestable".

## Reproducing the numbers

Run from `frontend/`.

Line counts by category:

```sh
find src -type f \( -name '*.ts' -o -name '*.tsx' \) ! -name '*.test.*' ! -path 'src/test/*' -print0 | xargs -0 cat | wc -l   # app
find src -type f -name '*.test.*' -print0 | xargs -0 cat | wc -l                                                        # tests
find src -name '*.css' -print0 | xargs -0 cat | wc -l                                                                    # css
```

Code, comment and blank split of the app lines:

```sh
find src -type f \( -name '*.ts' -o -name '*.tsx' \) ! -name '*.test.*' ! -path 'src/test/*' -print0 | xargs -0 cat | awk '/^[[:space:]]*$/{b++;next} /^[[:space:]]*(\/\/|\/\*|\*)/{c++;next} {k++} END{print "blank="b, "comment="c, "code="k}'
```

Hook call sites per component:

```sh
for f in src/App.tsx src/components/NotePage.tsx src/components/graph/GraphPage.tsx; do printf "%-40s useState=%s useEffect=%s useRef=%s\n" "$f" "$(grep -oE 'useState[<(]' $f | wc -l)" "$(grep -oE 'useEffect\(' $f | wc -l)" "$(grep -oE 'useRef[<(]' $f | wc -l)"; done
```

Unreferenced CSS classes (heuristic):

```sh
find src -name '*.css' -print0 | xargs -0 cat | grep -oE '\.[a-zA-Z_][a-zA-Z0-9_-]*' | sed 's/^\.//' | sort -u | while read -r c; do grep -rqF -- "$c" src --include='*.ts' --include='*.tsx' || echo "$c"; done
```

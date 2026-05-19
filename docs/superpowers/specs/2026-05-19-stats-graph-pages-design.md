# Stats & Graph Pages — Design Spec

**Date:** 2026-05-19
**Branch:** stats-integration

---

## Overview

Add two new pages to Hatchdoor — a Stats dashboard and an Obsidian-style Graph view — plus a minor update to the explorer sidebar to surface note count and navigation links to both pages.

---

## 1. Explorer Sidebar Changes

### Notes section label
The existing "Notes" list section in the explorer panel gets an inline total count:
```
Notes  142
```
The count comes from the existing tree data already loaded in `App.tsx`.

### Navigation links
Two new links appear at the top of the explorer panel, above the "Recently Viewed" section:

```
[ 📊 Stats ]  [ 🕸 Graph ]
```

Both are `<Link>` components routing to `/stats` and `/graph` respectively. They live inside `ExplorerPane.tsx`.

---

## 2. Stats Page (`/stats`)

### Route
New route added in `App.tsx`: `<Route path="/stats" element={<StatsPage />} />`

### New component
`frontend/src/components/StatsPage.tsx` — fetches from `/api/stats` on mount, renders all sections.

### Top counters
A row of large number cards:
- **Notes** — total note count
- **Words** — total word count across all notes
- **Tags** — count of distinct tags
- **Links** — total wikilinks in the vault
- **Images** — count of image embeds (`![` and `![[image` patterns in content)

### Sections

| Section | Content |
|---|---|
| Top Tags | Horizontal bar chart (pure CSS), sorted by note count descending, top 20 |
| Most Linked Notes | Ranked list with backlink count; each row is a `<Link>` to `/n/:slug` |
| Writing Activity | Bar chart of notes modified per month, last 6 months |
| Notes per Folder | Breakdown by top-level directory, note count per folder |
| Longest Notes | Top 5 by word count, clickable |
| Shortest Notes | Bottom 5 by word count (excluding empty), clickable |
| Orphan Notes | Notes with zero incoming and zero outgoing links, clickable list |
| Notes with No Tags | Notes missing all tags, clickable list |
| Modified This Week | Count + list of notes modified in last 7 days |
| Modified This Month | Count + list of notes modified in last 30 days |
| Link Balance | Two side-by-side counters: total outgoing links vs total backlinks |
| Average Word Count | Single number: mean words per note |
| Vault Size | Total size of all note files on disk (formatted as KB / MB) |

All charts are pure CSS — no charting library added.

### New backend endpoint: `GET /api/stats`

Returns a single JSON object computed from SQLite. All fields computed server-side.

**Response shape:**
```json
{
  "note_count": 142,
  "word_count": 84320,
  "tag_count": 38,
  "link_count": 317,
  "image_count": 54,
  "avg_word_count": 594,
  "vault_size_bytes": 2340000,
  "top_tags": [
    { "tag": "programming", "note_count": 24 }
  ],
  "most_linked": [
    { "title": "Index", "slug": "index", "backlink_count": 31 }
  ],
  "activity_by_month": [
    { "month": "2025-12", "modified_count": 8 }
  ],
  "notes_per_folder": [
    { "folder": "Projects", "note_count": 42 }
  ],
  "longest_notes": [
    { "title": "Big Note", "slug": "big-note", "word_count": 4200 }
  ],
  "shortest_notes": [
    { "title": "Stub", "slug": "stub", "word_count": 12 }
  ],
  "orphan_notes": [
    { "title": "Lost Note", "slug": "lost-note" }
  ],
  "no_tag_notes": [
    { "title": "Untagged", "slug": "untagged" }
  ],
  "modified_this_week": {
    "count": 7,
    "notes": [{ "title": "Recent", "slug": "recent" }]
  },
  "modified_this_month": {
    "count": 23,
    "notes": [{ "title": "Recent", "slug": "recent" }]
  },
  "total_outgoing_links": 317,
  "total_backlinks": 317
}
```

**Word count:** computed by splitting note content on whitespace server-side. YAML frontmatter is excluded (strip lines between leading `---` delimiters before counting).
**Image count:** count of `![` occurrences in note content — covers `![alt](url)` markdown images and `![[file.png]]` wikilink embeds. This is an approximation; non-image wikilink embeds using `![[` are rare and acceptable false positives.
**Modified lists cap:** `modified_this_week.notes` and `modified_this_month.notes` are capped at 20 entries each.
**Vault size:** sum of `size_bytes` from the `notes` table.

New Rust handler in `src/handlers/api.rs`. New response type in `src/api_types.rs`. New cache method `SqliteCache::vault_stats()` in `src/cache/queries.rs`.

---

## 3. Graph Page (`/graph`)

### Route
New route added in `App.tsx`: `<Route path="/graph" element={<GraphPage />} />`

### New component
`frontend/src/components/GraphPage.tsx` — fetches from `/api/graph` on mount, renders D3 force simulation inside a `<canvas>` or `<svg>` element.

### Library
**D3.js** (`d3-force`, `d3-zoom`, `d3-drag`) — installed as a dependency. No full D3 import; only the sub-packages needed are imported to keep bundle size minimal.

### New backend endpoint: `GET /api/graph`

Returns all nodes and all edges in one response (avoids N+1 per-note link fetches).

**Response shape:**
```json
{
  "nodes": [
    { "slug": "index", "title": "Index", "primary_tag": "programming", "backlink_count": 31 }
  ],
  "edges": [
    { "source": "index", "target": "projects" }
  ]
}
```

`primary_tag` is the first tag alphabetically for the note (or `null` if untagged). Tag-to-color mapping is computed deterministically on the frontend (hash tag name → hue in HSL).

New Rust handler in `src/handlers/api.rs`. New response types in `src/api_types.rs`. New cache method `SqliteCache::graph_data()` in `src/cache/queries.rs`.

### Rendering

- Canvas-based D3 force simulation for performance with large vaults
- Node radius: `base_radius + log(backlink_count + 1) * scale_factor`
- Node color: deterministic HSL from tag name hash; untagged nodes = muted grey
- Edge color: subtle, low-opacity lines
- Node labels: rendered on hover only (prevents clutter)

### Interactions

| Action | Behaviour |
|---|---|
| Hover node | Show note title tooltip, highlight direct edges |
| Click once | Highlight node + all directly connected nodes; dim everything else |
| Click again (same node) | Navigate to `/n/:slug` |
| Click canvas background | Clear highlight selection |
| Scroll | Zoom in/out (D3 zoom) |
| Drag canvas | Pan |
| Drag node | Reposition node (D3 drag), simulation re-stabilises |

### Tag filter

A chip list above the graph showing all tags. Selecting a chip dims nodes that do not have that tag. Multiple chips can be selected (union — show notes with any selected tag). Selecting no chips shows all nodes.

---

## 4. New Types in `frontend/src/types.ts`

```ts
export type TagStat = { tag: string; note_count: number };
export type NoteRef = { title: string; slug: string };
export type NoteWordRef = NoteRef & { word_count: number };
export type LinkedNoteRef = NoteRef & { backlink_count: number };
export type MonthActivity = { month: string; modified_count: number };
export type FolderStat = { folder: string; note_count: number };
export type NoteList = { count: number; notes: NoteRef[] };

export type VaultStats = {
  note_count: number;
  word_count: number;
  tag_count: number;
  link_count: number;
  image_count: number;
  avg_word_count: number;
  vault_size_bytes: number;
  total_outgoing_links: number;
  total_backlinks: number;
  top_tags: TagStat[];
  most_linked: LinkedNoteRef[];
  activity_by_month: MonthActivity[];
  notes_per_folder: FolderStat[];
  longest_notes: NoteWordRef[];
  shortest_notes: NoteWordRef[];
  orphan_notes: NoteRef[];
  no_tag_notes: NoteRef[];
  modified_this_week: NoteList;
  modified_this_month: NoteList;
};

export type GraphNode = { slug: string; title: string; primary_tag: string | null; backlink_count: number };
export type GraphEdge = { source: string; target: string };
export type GraphData = { nodes: GraphNode[]; edges: GraphEdge[] };
```

---

## 5. File Summary

| File | Change |
|---|---|
| `src/api_types.rs` | Add `VaultStatsResponse`, `GraphResponse`, `GraphNode`, `GraphEdge` |
| `src/cache/queries.rs` | Add `vault_stats()` and `graph_data()` methods |
| `src/handlers/api.rs` | Add `stats_handler` and `graph_handler` |
| `src/main.rs` (or router) | Register `/api/stats` and `/api/graph` routes |
| `frontend/src/types.ts` | Add `VaultStats`, `GraphNode`, `GraphEdge`, `GraphData` |
| `frontend/src/app/ExplorerPane.tsx` | Add Stats/Graph nav links; add note count to section label |
| `frontend/src/components/StatsPage.tsx` | New file |
| `frontend/src/components/GraphPage.tsx` | New file |
| `frontend/src/App.tsx` | Register `/stats` and `/graph` routes |
| `package.json` | Add `d3-force`, `d3-zoom`, `d3-drag`, `@types/d3` |

---

## 6. Out of Scope

- Editing notes from the graph view
- Saving graph layout between sessions
- Animated graph transitions beyond D3 defaults
- Real-time graph updates on vault change (can be added later)

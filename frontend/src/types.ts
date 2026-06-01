export type ExplorerFolder = {
  name: string;
  folders: ExplorerFolder[];
  notes: ExplorerNote[];
};

export type ExplorerNote = {
  title: string;
  slug: string;
};

export type Note = {
  title: string;
  slug: string;
  relative_path: string;
  content: string;
};

export type NoteLink = {
  title: string;
  slug: string;
  relative_path: string;
};

export type NoteLinks = {
  outgoing: NoteLink[];
  backlinks: NoteLink[];
};

export type NoteLinksResponse = {
  links: NoteLinks;
};

export type ActiveNoteMeta = {
  title: string;
  slug: string;
  relativePath: string;
  exportContent?: string;
};

export type RecentNote = ActiveNoteMeta & {
  viewedAt: number;
};

export type ModifiedNote = {
  title: string;
  slug: string;
  relative_path: string;
  mtime_ns: number;
};

export type RecentlyModifiedResponse = {
  notes: ModifiedNote[];
};

export type ReadPrefs = {
  fontSize: number;
  lineHeight: number;
  maxWidth: number;
};

export type ResolveBatchResponse = {
  results: Array<{
    target: string;
    slug: string | null;
  }>;
};

export type SearchMode = "semantic" | "keyword";

export interface OutboundLink {
  slug: string;
  title: string;
}

export interface SearchResult {
  chunk_id: number;
  note_slug: string;
  note_title: string;
  note_path: string;
  heading_path: string | null;
  content: string;
  score: number;
  outbound_links: OutboundLink[];
}

export type SearchSelection = {
  slug: string;
  query: string;
  matchKind: string;
};

export interface SearchResponse {
  mode: SearchMode;
  results: SearchResult[];
}

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

export type MermaidApi = {
  initialize: (config: {
    startOnLoad: boolean;
    securityLevel: "strict";
    theme?: string;
    fontFamily?: string;
    themeVariables?: {
      fontFamily?: string;
    };
  }) => void;
  render: (id: string, chart: string) => Promise<{ svg: string }>;
};

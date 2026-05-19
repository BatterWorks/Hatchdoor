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

export type SearchResult = {
  title: string;
  slug: string;
  relative_path: string;
  match_kind: string;
  snippet: string | null;
};

export type SearchSelection = {
  slug: string;
  query: string;
  matchKind: string;
};

export type SearchResponse = {
  results: SearchResult[];
};

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

export type MermaidApi = {
  initialize: (config: {
    startOnLoad: boolean;
    securityLevel: "strict";
    fontFamily?: string;
    themeVariables?: {
      fontFamily?: string;
    };
  }) => void;
  render: (id: string, chart: string) => Promise<{ svg: string }>;
};

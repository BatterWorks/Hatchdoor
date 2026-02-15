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
};

export type RecentNote = ActiveNoteMeta & {
  viewedAt: number;
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

export type MermaidApi = {
  initialize: (config: {
    startOnLoad: boolean;
    securityLevel: "strict";
  }) => void;
  render: (id: string, chart: string) => Promise<{ svg: string }>;
};

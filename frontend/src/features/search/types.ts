import type { NoteMetadata } from "../../types";

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
  metadata?: NoteMetadata;
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

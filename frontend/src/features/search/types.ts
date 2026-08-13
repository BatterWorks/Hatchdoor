import type { NoteMetadata, VaultId } from "../../types";

export type SearchMode = "semantic" | "keyword";

export interface OutboundLink {
  slug: string;
  title: string;
}

export interface SearchResult {
  vault_id: VaultId;
  chunk_id: number;
  note_slug: string;
  note_title: string;
  note_path: string;
  heading_path: string | null;
  content: string;
  score: number;
  layer: string | null;
  outbound_links: OutboundLink[];
  metadata?: NoteMetadata;
}

export type SearchSelection = {
  vaultId: VaultId;
  slug: string;
  query: string;
  matchKind: string;
};

export interface SearchResponse {
  mode: SearchMode;
  results: SearchResult[];
}

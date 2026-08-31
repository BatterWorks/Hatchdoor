// A Vault's immutable identity: a canonical UUID string. Never a display
// name — names are editable and not unique (docs/migrations/vault-scoped-clients.md).
export type VaultId = string;

// One Vault ID, or the literal "all" for every enabled Vault. Collection
// reads take a VaultScope; exact reads and mutations always take one VaultId.
export type VaultScope = VaultId | "all";

/** The stable `/api/v1/vaults/**` error body: `{code, message, vault_id?, retryable}`.
 * `code` is branched on; `message` is for people and may change. */
export type VaultApiError = {
  code: string;
  message: string;
  vault_id?: VaultId;
  retryable: boolean;
};

export type VaultParticipantState =
  | "fresh"
  | "stale"
  /** Rows are current but carry no vectors yet, so this Vault contributed
   * nothing to a semantic search. Browsing, keyword and tag search are
   * unaffected and report `fresh`. */
  | "not_searchable"
  | "unavailable";

export type VaultReadError = {
  code: string;
  message: string;
  vault_id: VaultId | null;
  retryable: boolean;
};

export type VaultParticipant = {
  vault_id: VaultId;
  vault_name: string;
  state: VaultParticipantState;
  error?: VaultReadError;
};

/** The shared one-or-all collection-read envelope every `{scope}` route
 * responds with. `partial` is true when any participant isn't `fresh`. */
export type VaultReadProjection<T> = {
  scope: string;
  collection_revision: number;
  partial: boolean;
  participants: VaultParticipant[];
  data: T;
};

export type VaultRuntimeErrorDetail =
  | { kind: "affected_paths"; paths: string[]; total: number }
  | { kind: "local_commits_ahead"; ahead: number };

export type VaultRuntimeError = {
  code: string;
  message: string;
  retryable: boolean;
  detail?: VaultRuntimeErrorDetail;
};

export type VaultCapabilities = {
  browse: boolean;
  search: boolean;
  mutate: boolean;
  pull: boolean;
  push: boolean;
  retry: boolean;
};

/** How a git-backed Vault's history is kept: local commits only, or synced
 * against a remote (`src/vault_registry.rs`'s `VaultGitMode`). Excluded from
 * source identity, so switching it needs no disable/confirm round trip. */
export type VaultGitMode = "local_history" | "pull_only" | "two_way";

/** A Vault's origin, discriminated on `type` (`src/vault_registry.rs`'s
 * `#[serde(tag = "type")] VaultSource`). Never carries credentials — those
 * live behind `credential_configured` on `VaultSummary` instead. */
export type VaultSource =
  | { type: "local"; path: string }
  | {
      type: "existing_git";
      repository_path: string;
      repository_url?: string;
      branch?: string;
      vault_subdirectory?: string;
      mode: VaultGitMode;
      poll_interval_secs: number;
    }
  | {
      type: "managed_git";
      repository_url: string;
      branch?: string;
      vault_subdirectory?: string;
      mode: VaultGitMode;
      poll_interval_secs: number;
    };

/** One Vault's discovery entry from `GET /api/v1/vaults`. The four status
 * fields are independent — a Vault can be `activation: "active"` and
 * `git: "unavailable"` at once. There is no single unified condition field. */
export type VaultSummary = {
  vault_id: VaultId;
  name: string;
  enabled: boolean;
  /** The non-secret Vault source definition from discovery. */
  source?: VaultSource;
  exclude_patterns: string[];
  credential_configured: boolean;
  archive_folder?: string;
  commit_identity?: { name: string; email: string };
  activation: "active" | "disabled" | "unavailable";
  local_content: "read_write" | "read_only" | "unavailable";
  search: "unavailable" | "indexing" | "browsable" | "ready" | "stale";
  git: "disabled" | "pending" | "ready" | "unavailable";
  /** When this Vault's last scheduled or manual Git turn finished, RFC 3339
   * UTC — whether it succeeded or failed, so read it with `git` rather than
   * as a successful sync. Absent for a Vault with no remote to poll, one
   * that has not completed a turn yet, and on the read-only demo. */
  last_checked_at?: string;
  /** When this Vault's next scheduled Git turn is due, RFC 3339 UTC. Present
   * for every Vault with a remote to poll, including one that has never
   * completed a turn — it is due immediately, not unscheduled. Absent for a
   * Vault with no remote, and on the read-only demo. */
  next_attempt_at?: string;
  watcher: "running" | "disabled" | "unavailable";
  capabilities: VaultCapabilities;
  activation_error?: VaultRuntimeError;
  search_error?: VaultRuntimeError;
  git_error?: VaultRuntimeError;
  watcher_error?: VaultRuntimeError;
};

export type VaultRegistryRecovery = {
  code: "vault_registry_recovery_required";
  kind: "corrupt" | "unsupported_schema" | "future_schema";
  message: string;
};

/** Present only when the registry itself loaded fine (empty, revision 0) but
 * a failed safe legacy import still needs operator recovery (#150). Distinct
 * from `VaultRegistryRecovery`: that one means the persisted registry file
 * itself is unreadable. */
export type LegacyMigrationRecovery = {
  code: "legacy_migration_required" | "legacy_environment_cleanup_required";
  message: string;
};

export type VaultDiscoveryResponse = {
  registry_revision?: number;
  collection_revision: number;
  vaults: VaultSummary[];
  recovery?: VaultRegistryRecovery;
  legacy_migration_recovery?: LegacyMigrationRecovery;
  demo_mode: boolean;
};

/** A folder exactly as `GET /api/v1/vaults/{scope}/tree` sends it.
 *
 * `note_count` is the notes held directly in this folder, excluding its
 * subfolders, and `truncated` marks a folder the server declined to expand.
 * Neither is used here — the explorer draws the whole tree — but they are part
 * of the payload and typed so nothing silently assumes they are absent. */
export type WireExplorerFolder = {
  name: string;
  note_count: number;
  truncated?: boolean;
  folders: WireExplorerFolder[];
  notes: WireExplorerNote[];
};

/** A note as the tree sends it: not vault-qualified, because the enclosing
 * `WireVaultTree` already names the vault (#192). */
export type WireExplorerNote = {
  title: string;
  slug: string;
};

/** One participating Vault's tree, as returned (grouped, never merged) by
 * `GET /api/v1/vaults/{scope}/tree`. */
export type WireVaultTree = {
  vault_id: VaultId;
  vault_name: string;
  tree: WireExplorerFolder;
};

/** A folder as the app carries it, once each note has been stamped with the
 * vault of the tree it arrived in. The grouping is lost the moment trees are
 * merged or flattened, so the attribution happens on arrival rather than being
 * rethreaded through every component that renders a note. */
export type ExplorerFolder = {
  name: string;
  folders: ExplorerFolder[];
  notes: ExplorerNote[];
};

export type ExplorerNote = {
  vault_id: VaultId;
  title: string;
  slug: string;
};

export type VaultTree = {
  vault_id: VaultId;
  vault_name: string;
  tree: ExplorerFolder;
};

export type Note = {
  title: string;
  slug: string;
  relative_path: string;
  content: string;
  content_hash: string;
  layer: string | null;
  metadata?: NoteMetadata;
};

export type NoteMetadata = {
  tags: string[];
  aliases: string[];
  properties: Record<string, unknown>;
};

/** `GET /api/v1/vaults/{vault_id}/notes/{slug}` — `note` stays nested,
 * `vault_id` is its sibling, not a field on `Note` itself. */
export type VaultQualifiedNote = {
  vault_id: VaultId;
  note: Note;
};

export type NoteLink = {
  title: string;
  slug: string;
  relative_path: string;
  layer: string | null;
};

export type VaultQualifiedNoteLink = {
  vault_id: VaultId;
  link: NoteLink;
};

/** `GET /api/v1/vaults/{vault_id}/notes/{slug}/links` — flat, not nested
 * under a `links` key the way the legacy response was. */
export type VaultQualifiedLinks = {
  vault_id: VaultId;
  outgoing: VaultQualifiedNoteLink[];
  backlinks: VaultQualifiedNoteLink[];
};

/** The local, unwrapped shape `VaultQualifiedLinks` is flattened into after a
 * fetch — every entry's `vault_id` is always the note's own (cross-Vault
 * backlinks are ruled out by #62), so callers that already know which Vault
 * they're reading don't carry it a second time per link. */
export type NoteLinks = {
  outgoing: NoteLink[];
  backlinks: NoteLink[];
};

export type WriteCapabilities = {
  vault_id: VaultId;
  enabled: boolean;
  warnings: string[];
};

export type WriteOutcome = {
  vault_id: VaultId;
  ok: boolean;
  slug: string | null;
  relative_path: string | null;
  content_hash: string | null;
  quality_warnings: string[];
  rewritten_notes: number;
  moved_assets: number;
  trashed_path: string | null;
  layer: string | null;
};

export type AttachmentOutcome = {
  vault_id: VaultId;
  ok: boolean;
  attachment: {
    relative_path: string;
    size_bytes: number;
    content_hash: string;
    layer: string | null;
  };
  rewritten_notes: number;
  trashed_path: string | null;
  cleanup_warning: string | null;
};

export type ActiveNoteMeta = {
  vaultId: VaultId;
  title: string;
  slug: string;
  relativePath: string;
  exportContent?: string;
  contentHash?: string;
};

export type RecentNote = ActiveNoteMeta & {
  viewedAt: number;
};

/** `GET /api/v1/vaults/{scope}/recent` — flattened across Vaults, each row
 * individually carrying its own `vault_id`. Powers both the server-tracked
 * "recently changed" panel and (unrelated) the client-tracked "recently
 * viewed" list is `RecentNote` above, not this type. */
export type ModifiedNote = {
  vault_id: VaultId;
  title: string;
  slug: string;
  relative_path: string;
  mtime_ns: number;
};

export type ReadPrefs = {
  fontSize: number;
  lineHeight: number;
  maxWidth: number;
};

export type VaultResolveResponse = {
  vault_id: VaultId;
  slug: string | null;
};

export type VaultResolveBatchResponse = {
  vault_id: VaultId;
  results: Array<{
    target: string;
    slug: string | null;
    archived: boolean;
  }>;
  // Vault-relative asset paths for the request's `asset_targets` (#158).
  // Optional so a response from a server predating asset resolution still
  // parses, leaving embeds on their note-relative reading.
  asset_results?: Array<{
    target: string;
    path: string | null;
  }>;
};

export type TagStat = { tag: string; note_count: number };
export type NoteRef = { title: string; slug: string };
export type NoteWordRef = NoteRef & { word_count: number };
export type LinkedNoteRef = NoteRef & { backlink_count: number };
export type MonthActivity = { month: string; modified_count: number };
export type FolderStat = { folder: string; note_count: number };
export type NoteList = { count: number; notes: NoteRef[] };

/** One Vault's lean statistics from the collection-scope
 * `GET /api/v1/vaults/{scope}/stats` route — `VaultTree`'s sibling in the
 * collection envelope. Powers the sidebar Scope zone's per-Vault note count
 * (#139); too lean to back the Statistics page, which uses `VaultStats`
 * below instead. */
export type VaultStatistics = {
  vault_id: VaultId;
  vault_name: string;
  note_count: number;
  tag_count: number;
  link_count: number;
  vault_size_bytes: number;
};

/** The rich, exact single-Vault report from
 * `GET /api/v1/vaults/{vault_id}/stats/detail`. Never `all` — this is an
 * exact read, like `notes/{slug}`, not a `{scope}` collection read. The
 * frozen `{scope}/stats` collection route intentionally returns only lean
 * counts (`VaultStatistics`, see `VaultTree`'s sibling in the collection
 * envelope) and cannot back this page. */
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

/** `GET /api/v1/vaults/{vault_id}/stats/detail`'s response — `stats` nests,
 * mirroring `VaultQualifiedNote`, not a flat merge. */
export type VaultQualifiedStats = {
  vault_id: VaultId;
  stats: VaultStats;
};

export type GraphNode = {
  vault_id: VaultId;
  slug: string;
  title: string;
  primary_tag: string | null;
  backlink_count: number;
};
export type GraphEdge = {
  vault_id: VaultId;
  source_slug: string;
  target_slug: string;
};

/** One participating Vault's graph component, as returned (grouped, edges
 * never crossing Vaults) by `GET /api/v1/vaults/{scope}/graph`. */
export type VaultGraph = {
  vault_id: VaultId;
  vault_name: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
};

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

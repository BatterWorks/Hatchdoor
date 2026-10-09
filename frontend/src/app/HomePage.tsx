import { Link } from "react-router-dom";

import { SideHead } from "../components/Explorer";
import { UiButton, VaultPrefix } from "../components/ui";
import { formatWhen } from "../features/settings";
import type {
  ModifiedNote,
  RecentNote,
  VaultId,
  VaultScope,
  VaultSummary,
} from "../types";
import { scopeName } from "./vaultSlotLogic";

/** Rows per list. Enough to be a landing, few enough that the two lists sit
 * side by side without a scroll of their own. */
const HOME_ROWS = 6;

type HomeRow = {
  key: string;
  to: string;
  title: string;
  path: string;
  vaultId: VaultId;
  when: string | null;
};

/**
 * The landing page with no note open (#530): what changed, what was read,
 * and the two things to do next. Every figure on it is state the shell
 * already holds for the sidebar, so it costs no read of its own. The
 * instruction it replaced ("Select any note from the explorer") told the
 * reader nothing the explorer beside it did not.
 */
export function HomePage({
  vaults,
  scope,
  noteCounts,
  modifiedNotes,
  recentNotes,
  writeEnabled,
  onNewNote,
  onOpenSearch,
}: {
  vaults: VaultSummary[];
  scope: VaultScope;
  noteCounts: Record<VaultId, number | undefined>;
  modifiedNotes: ModifiedNote[];
  recentNotes: RecentNote[];
  writeEnabled: boolean;
  onNewNote: () => void;
  onOpenSearch: () => void;
}) {
  const inScope =
    scope === "all"
      ? vaults
      : vaults.filter((vault) => vault.vault_id === scope);
  const counted = inScope.filter(
    (vault) => noteCounts[vault.vault_id] !== undefined,
  );
  const noteTotal = counted.reduce(
    (sum, vault) => sum + (noteCounts[vault.vault_id] ?? 0),
    0,
  );
  // A count is only stated once every Vault in scope has reported one;
  // "0 notes" over a Vault still indexing would be a claim the shell cannot
  // make (#333).
  const summary = [
    `${inScope.length} ${inScope.length === 1 ? "Vault" : "Vaults"}`,
    counted.length === inScope.length && inScope.length > 0
      ? `${noteTotal} ${noteTotal === 1 ? "note" : "notes"}`
      : null,
  ]
    .filter(Boolean)
    .join(", ");
  // Both lists are read through the browsing scope, like the summary above
  // them: `modifiedNotes` already is, a viewing history is not.
  const changed: HomeRow[] = modifiedNotes.map((note) => ({
    key: `${note.vault_id}-${note.slug}`,
    to: `/v/${encodeURIComponent(note.vault_id)}/n/${note.slug}`,
    title: note.title,
    path: note.relative_path,
    vaultId: note.vault_id,
    when: formatWhen(Math.round(note.mtime_ns / 1_000_000)),
  }));
  const recent: HomeRow[] = recentNotes
    .filter(
      (note) =>
        (scope === "all" || note.vaultId === scope) &&
        vaults.some((vault) => vault.vault_id === note.vaultId),
    )
    .map((note) => ({
      key: `${note.vaultId}-${note.slug}`,
      to: `/v/${encodeURIComponent(note.vaultId)}/n/${note.slug}`,
      title: note.title,
      path: note.relativePath,
      vaultId: note.vaultId,
      when: formatWhen(note.viewedAt),
    }));

  return (
    <div className="home-page">
      <div className="home-head">
        <div>
          <p className="home-eyebrow">{scopeName(scope, vaults)}</p>
          <h1 className="home-title">Notes</h1>
          <p className="home-summary">{summary}</p>
        </div>
        <div className="home-actions">
          {writeEnabled ? (
            <UiButton className="close-note" onClick={onNewNote}>
              New note
            </UiButton>
          ) : null}
          <UiButton className="close-note" onClick={onOpenSearch}>
            Search
            <span className="shortcut-hint" aria-hidden="true">
              ⌘K
            </span>
          </UiButton>
        </div>
      </div>

      <div className="home-grid">
        <HomeList
          label="Changed on disk"
          rows={changed}
          vaults={vaults}
          empty="Nothing has changed on disk yet."
        />
        <HomeList
          label="Recently viewed"
          rows={recent}
          vaults={vaults}
          empty="Notes you open show up here. Pick one from the explorer, or search."
        />
      </div>
    </div>
  );
}

/** One of Home's two lists: the sidebar's own section head over the first
 * rows, the count being the whole list's, not the rows shown. */
function HomeList({
  label,
  rows,
  vaults,
  empty,
}: {
  label: string;
  rows: HomeRow[];
  vaults: VaultSummary[];
  empty: string;
}) {
  const showVaultPrefix = vaults.length > 1;
  const vaultNameOf = (vaultId: VaultId) =>
    vaults.find((vault) => vault.vault_id === vaultId)?.name ?? vaultId;
  return (
    <section className="home-list" aria-label={label}>
      <SideHead label={label} count={rows.length} />
      {rows.length === 0 ? (
        <p className="home-empty">{empty}</p>
      ) : (
        <ul>
          {rows.slice(0, HOME_ROWS).map((row, index) => (
            <li key={row.key}>
              <Link to={row.to} title={`${row.path}.md`}>
                <span className="idx" aria-hidden="true">
                  {String(index + 1).padStart(3, "0")}
                </span>
                {showVaultPrefix ? (
                  <VaultPrefix name={vaultNameOf(row.vaultId)} />
                ) : null}
                <span className="home-note-title">{row.title}</span>
                <span className="home-note-meta">{row.when}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

import { Link } from "react-router-dom";

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
  const vaultNameOf = (vaultId: VaultId) =>
    vaults.find((vault) => vault.vault_id === vaultId)?.name ?? vaultId;
  const showVaultPrefix = vaults.length > 1;
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
  const changed = modifiedNotes.slice(0, HOME_ROWS);
  const recent = recentNotes
    .filter((note) => vaults.some((vault) => vault.vault_id === note.vaultId))
    .slice(0, HOME_ROWS);

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
        <section className="home-list" aria-labelledby="home-changed">
          <h2 id="home-changed">
            Changed on disk
            <span className="side-rule" aria-hidden="true" />
            <span className="side-count">
              {String(changed.length).padStart(2, "0")}
            </span>
          </h2>
          {changed.length === 0 ? (
            <p className="home-empty">Nothing has changed on disk yet.</p>
          ) : (
            <ul>
              {changed.map((note, index) => (
                <li key={`${note.vault_id}-${note.slug}`}>
                  <Link
                    to={`/v/${encodeURIComponent(note.vault_id)}/n/${note.slug}`}
                    title={`${note.relative_path}.md`}
                  >
                    <span className="idx" aria-hidden="true">
                      {String(index + 1).padStart(3, "0")}
                    </span>
                    {showVaultPrefix ? (
                      <VaultPrefix name={vaultNameOf(note.vault_id)} />
                    ) : null}
                    <span className="home-note-title">{note.title}</span>
                    <span className="home-note-meta">
                      {formatWhen(Math.round(note.mtime_ns / 1_000_000))}
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="home-list" aria-labelledby="home-recent">
          <h2 id="home-recent">
            Recently viewed
            <span className="side-rule" aria-hidden="true" />
            <span className="side-count">
              {String(recent.length).padStart(2, "0")}
            </span>
          </h2>
          {recent.length === 0 ? (
            <p className="home-empty">
              Notes you open show up here. Pick one from the explorer, or
              search.
            </p>
          ) : (
            <ul>
              {recent.map((note, index) => (
                <li key={`${note.vaultId}-${note.slug}`}>
                  <Link
                    to={`/v/${encodeURIComponent(note.vaultId)}/n/${note.slug}`}
                    title={`${note.relativePath}.md`}
                  >
                    <span className="idx" aria-hidden="true">
                      {String(index + 1).padStart(3, "0")}
                    </span>
                    {showVaultPrefix ? (
                      <VaultPrefix name={vaultNameOf(note.vaultId)} />
                    ) : null}
                    <span className="home-note-title">{note.title}</span>
                    <span className="home-note-meta">
                      {formatWhen(note.viewedAt)}
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

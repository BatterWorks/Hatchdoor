import { NavLink } from "react-router-dom";

import { describeMissingVaults } from "../lib/vaultParticipants";
import { SideHead } from "./Explorer";
import { StateBlock, VaultPrefix } from "./ui";
import type { ModifiedNote, VaultScope, VaultSummary } from "../types";

/** Matches the cap in the design: newest first, the rest behind a count. */
const VISIBLE_LIMIT = 15;

/**
 * Notes that changed on disk, opened from the sidebar rail.
 *
 * Deliberately **not** a notification badge yet. The design calls for a count
 * of changes that arrived from outside this browser — MCP agents, git sync,
 * another device — so that it means "something happened behind my back". The
 * data to do that does not exist: the SSE stream only bumps a revision counter
 * and carries no per-note detail, and `/api/v1/vaults/{scope}/recent` cannot
 * tell your own edit from an agent's.
 *
 * Shipping a count off the wrong data would be worse than shipping none, since
 * it would tick up every time you saved a note yourself. So this lists the
 * changes without claiming any of them are news.
 */
export function ChangesPanel({
  notes,
  onNavigate,
  vaults,
  scope,
  partial,
  missingVaultNames,
}: {
  notes: ModifiedNote[];
  onNavigate: () => void;
  vaults: VaultSummary[];
  scope: VaultScope;
  /** Whether at least one Vault this read asked did not answer fresh
   * (#141). Never a banner: results still render in the API's own ranking,
   * and only the empty case changes shape. */
  partial: boolean;
  missingVaultNames: string[];
}) {
  const visible = notes.slice(0, VISIBLE_LIMIT);
  const overflow = notes.length - visible.length;
  // Provenance only where the list can actually span Vaults (#140).
  const showVaultPrefix = scope === "all" && vaults.length > 1;

  return (
    <section
      className="explorer-changes"
      id="explorer-changes-panel"
      aria-label="Recently changed notes"
    >
      {/* Labelled like every other section in the pane; an unlabelled list of
          notes appearing above Recently viewed reads as part of it. */}
      <SideHead label="Changed on disk" count={notes.length} />
      {notes.length === 0 ? (
        partial ? (
          // Nothing usable: the documented error block replaces the empty
          // shell entirely rather than sitting under it. "Nothing has
          // changed" would be a lie when some Vaults never answered.
          <StateBlock
            tone="error"
            title="Nothing Found"
            description={describeMissingVaults(missingVaultNames)}
          />
        ) : (
          <p className="explorer-changes-empty">
            Nothing has changed on disk yet.
          </p>
        )
      ) : (
        <>
          <ul className="tree root-tree">
            {visible.map((note, index) => (
              <li key={`${note.vault_id}-${note.slug}`} className="note-item">
                {/* No active-note class: that highlight is canonical in the
                    tree only, which is what removed the multi-highlight bug. */}
                <NavLink
                  className="note-link"
                  to={`/v/${encodeURIComponent(note.vault_id)}/n/${note.slug}`}
                  onClick={onNavigate}
                  title={`${note.relative_path}.md`}
                >
                  <span className="idx" aria-hidden="true">
                    {String(index + 1).padStart(3, "0")}
                  </span>
                  {showVaultPrefix ? (
                    <VaultPrefix
                      name={
                        vaults.find((vault) => vault.vault_id === note.vault_id)
                          ?.name ?? note.vault_id
                      }
                    />
                  ) : null}
                  <span className="note-label">{note.title}</span>
                </NavLink>
              </li>
            ))}
          </ul>
          {overflow > 0 ? (
            <p className="explorer-changes-more">and {overflow} more</p>
          ) : null}
          {/* Ranking is unchanged by partiality; this trailing line is the
              only thing that changes (#141). */}
          {partial ? (
            <p className="explorer-changes-partial">
              {describeMissingVaults(missingVaultNames)}
            </p>
          ) : null}
        </>
      )}
    </section>
  );
}

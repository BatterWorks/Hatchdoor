import { NavLink } from "react-router-dom";

import { SideHead } from "./Explorer";
import type { ModifiedNote } from "../types";

/** Matches the cap in the design: newest first, the rest behind a count. */
const VISIBLE_LIMIT = 15;

/**
 * Notes that changed on disk, opened from the sidebar rail.
 *
 * Deliberately **not** a notification badge yet. The design calls for a count
 * of changes that arrived from outside this browser — MCP agents, git sync,
 * another device — so that it means "something happened behind my back". The
 * data to do that does not exist: the SSE stream only bumps a revision counter
 * and carries no per-note detail, and `/api/recently-modified` cannot tell your
 * own edit from an agent's.
 *
 * Shipping a count off the wrong data would be worse than shipping none, since
 * it would tick up every time you saved a note yourself. So this lists the
 * changes without claiming any of them are news.
 */
export function ChangesPanel({
  notes,
  onNavigate,
}: {
  notes: ModifiedNote[];
  onNavigate: () => void;
}) {
  const visible = notes.slice(0, VISIBLE_LIMIT);
  const overflow = notes.length - visible.length;

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
        <p className="explorer-changes-empty">
          Nothing has changed on disk yet.
        </p>
      ) : (
        <>
          <ul className="tree root-tree">
            {visible.map((note, index) => (
              <li key={note.slug} className="note-item">
                {/* No active-note class: that highlight is canonical in the
                    tree only, which is what removed the multi-highlight bug. */}
                <NavLink
                  className="note-link"
                  to={`/n/${note.slug}`}
                  onClick={onNavigate}
                  title={`${note.relative_path}.md`}
                >
                  <span className="idx" aria-hidden="true">
                    {String(index + 1).padStart(3, "0")}
                  </span>
                  <span className="note-label">{note.title}</span>
                </NavLink>
              </li>
            ))}
          </ul>
          {overflow > 0 ? (
            <p className="explorer-changes-more">and {overflow} more</p>
          ) : null}
        </>
      )}
    </section>
  );
}

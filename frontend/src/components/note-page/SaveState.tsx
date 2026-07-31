import type { AutosaveStatus } from "../../hooks/useNoteAutosave";

/**
 * Says whether the note is saved, because autosave removed the Save button and
 * with it the only evidence a user had that their writing survived. In a vault
 * with agent co-writers and git sync, that evidence is the trust surface of the
 * whole feature.
 *
 * States what is true now rather than announcing an event, so it reads
 * "Saved 14:32" and not "Saved!".
 */
export function SaveState({
  status,
  savedAt,
}: {
  status: AutosaveStatus;
  savedAt: Date | null;
}) {
  if (status === "conflict" || status === "error") {
    return <span className="save-state is-error">Not saving</span>;
  }

  if (status === "saving") {
    return <span className="save-state">Saving…</span>;
  }

  if (status === "saved" && savedAt) {
    return (
      <span className="save-state">
        Saved{" "}
        {savedAt.toLocaleTimeString(undefined, {
          hour: "2-digit",
          minute: "2-digit",
        })}
      </span>
    );
  }

  return null;
}

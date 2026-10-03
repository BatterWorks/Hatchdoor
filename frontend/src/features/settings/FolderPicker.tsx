import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode, Ref } from "react";

import { CONTEXTUAL_HELP, ContextualHelpLink } from "../help";
import type {
  FolderListing,
  FolderNoteCount,
  FolderVaultRef,
} from "../../types";
import {
  fetchFolderListing,
  mountFolderPath,
  noteCountLabel,
} from "./vaultCreation";

function lastSegment(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

/** Whether the mount has nothing to offer: it is missing, or holds no
 * Markdown at all. Only judged at the mount itself, where the count covers
 * every folder below. */
function mountIsEmpty(listing: FolderListing): boolean {
  if (!listing.root_found) return true;
  return (
    listing.path === "" &&
    listing.markdown.count === 0 &&
    !listing.markdown.at_least
  );
}

/** One pickable folder: the whole row picks it, and a folder that is
 * already a Vault refuses the pick with a line naming that Vault. */
function FolderRow({
  label,
  name,
  markdown,
  vault,
  pressed,
  blocked,
  rowClass,
  buttonRef,
  onPick,
  children,
}: {
  label: string;
  name: string;
  markdown: FolderNoteCount;
  vault: FolderVaultRef | null;
  pressed: boolean;
  blocked: boolean;
  rowClass?: string;
  buttonRef?: Ref<HTMLButtonElement>;
  onPick: () => void;
  children?: ReactNode;
}) {
  return (
    <li className="folder-picker-item">
      <button
        ref={buttonRef}
        type="button"
        className={
          rowClass ? `folder-picker-row ${rowClass}` : "folder-picker-row"
        }
        aria-pressed={pressed}
        aria-disabled={vault ? true : undefined}
        onClick={onPick}
      >
        <span className="folder-picker-name">{label}</span>
        {vault ? (
          <span className="folder-picker-badge">Already a Vault</span>
        ) : null}
        <span className="folder-picker-count">{noteCountLabel(markdown)}</span>
      </button>
      {children}
      {vault && blocked ? (
        <p className="folder-picker-row-note" role="status">
          {name} is already the Vault <strong>{vault.name}</strong>. Pick
          another folder.
        </p>
      ) : null}
    </li>
  );
}

/** Pick a notes folder from the folders Hatchdoor can see under its Vault
 * mount (#430, ADR-41), one level at a time with a trail back up. It reports
 * the picked folder's absolute path; the caller owns what happens next, so
 * the same picker serves Add a Vault and the first-run checklist (#419). */
export function FolderPicker({
  value,
  onPick,
}: {
  /** The absolute path currently chosen, highlighted when it is listed. */
  value: string;
  onPick: (path: string) => void;
}) {
  // The level on screen stays until the next one arrives, so a slow folder
  // never leaves the reader without the trail back up.
  const [listing, setListing] = useState<FolderListing | null>(null);
  const [loading, setLoading] = useState(true);
  const [failure, setFailure] = useState<string | null>(null);
  const [blocked, setBlocked] = useState<string | null>(null);
  const [outsideOpen, setOutsideOpen] = useState(false);
  const request = useRef(0);
  const firstRow = useRef<HTMLButtonElement>(null);
  const moveFocus = useRef(false);

  const loadLevel = useCallback(async (path: string) => {
    const id = ++request.current;
    setLoading(true);
    setBlocked(null);
    const result = await fetchFolderListing(path);
    // A slower answer for a folder the reader already left is dropped.
    if (id !== request.current) return;
    setLoading(false);
    if (result.ok) {
      setFailure(null);
      setListing(result.listing);
    } else {
      setFailure(result.message);
    }
  }, []);

  useEffect(() => {
    void loadLevel("");
  }, [loadLevel]);

  // Going into a folder or back up replaces the button that had focus, so
  // focus moves to the new level's first row instead of falling out.
  useEffect(() => {
    if (listing && moveFocus.current) {
      moveFocus.current = false;
      firstRow.current?.focus();
    }
  }, [listing]);

  const navigate = (path: string) => {
    moveFocus.current = true;
    void loadLevel(path);
  };

  const outsideNote = (
    <>
      <button
        type="button"
        className="folder-picker-more"
        aria-expanded={outsideOpen}
        onClick={() => setOutsideOpen((current) => !current)}
      >
        My folder isn&rsquo;t here
      </button>
      {outsideOpen ? (
        <div className="settings-notice folder-picker-note">
          <p>
            <strong>Hatchdoor can&rsquo;t see this folder yet.</strong> It only
            sees the folders shared with it when it was installed
            {listing ? (
              <>
                , under <code>{listing.root}</code>
              </>
            ) : null}
            . A folder anywhere else on this computer has to be added to the
            install first, then Hatchdoor restarted.{" "}
            <ContextualHelpLink to={CONTEXTUAL_HELP.folderOutsideMount} />
          </p>
          <p>Or ask your agent to add it.</p>
        </div>
      ) : null}
    </>
  );

  if (failure !== null) {
    return (
      <div className="folder-picker">
        <div className="settings-notice settings-notice-err" role="alert">
          {failure}{" "}
          <button
            type="button"
            className="folder-picker-more"
            onClick={() => void loadLevel(listing?.path ?? "")}
          >
            Try again
          </button>
        </div>
        {outsideNote}
      </div>
    );
  }

  if (!listing) {
    return (
      <div className="folder-picker">
        <p className="folder-picker-status" role="status">
          Looking for folders…
        </p>
      </div>
    );
  }

  // A mount with no notes says so, but a folder that exists stays pickable
  // under the message: starting an empty Vault is allowed (#428).
  const emptyPanel = mountIsEmpty(listing) ? (
    <div className="folder-picker-empty">
      <p className="folder-picker-empty-title">No notes found yet</p>
      <p>
        Hatchdoor looks for Markdown notes in <code>{listing.root}</code>{" "}
        {listing.root_found
          ? "and found none there."
          : "but that folder does not exist."}{" "}
        Put your notes in the folder you shared with Hatchdoor when you
        installed it, then look again.{" "}
        <ContextualHelpLink to={CONTEXTUAL_HELP.folderOutsideMount} />
      </p>
      <p>
        Or ask your agent to add it.
        {listing.root_found
          ? " To start with an empty Vault, pick a folder below."
          : null}
      </p>
      <button
        type="button"
        className="settings-btn"
        onClick={() => void loadLevel("")}
      >
        Look again
      </button>
    </div>
  ) : null;

  if (!listing.root_found) {
    return <div className="folder-picker">{emptyPanel}</div>;
  }

  const here = listing.path;
  const hereName = here ? lastSegment(here) : lastSegment(listing.root);
  const herePath = mountFolderPath(listing.root, here);
  const trail = here ? here.split("/") : [];

  const pick = (relative: string, vault: FolderVaultRef | null): void => {
    if (vault) {
      setBlocked(relative);
      return;
    }
    setBlocked(null);
    onPick(mountFolderPath(listing.root, relative));
  };

  return (
    <div className="folder-picker">
      {emptyPanel}
      <div className="folder-picker-box" aria-busy={loading}>
        <nav className="folder-picker-crumbs" aria-label="Folder location">
          {trail.length === 0 ? (
            <span aria-current="location">{hereName}</span>
          ) : (
            <button
              type="button"
              className="folder-picker-crumb"
              onClick={() => navigate("")}
            >
              {lastSegment(listing.root)}
            </button>
          )}
          {trail.map((segment, index) => {
            const path = trail.slice(0, index + 1).join("/");
            const last = index === trail.length - 1;
            return (
              <span key={path} className="folder-picker-crumb-step">
                <span className="folder-picker-sep" aria-hidden="true">
                  /
                </span>
                {last ? (
                  <span aria-current="location">{segment}</span>
                ) : (
                  <button
                    type="button"
                    className="folder-picker-crumb"
                    onClick={() => navigate(path)}
                  >
                    {segment}
                  </button>
                )}
              </span>
            );
          })}
        </nav>
        <ul className="folder-picker-list" aria-label="Folders">
          <FolderRow
            label={`Use ${hereName}`}
            name={hereName}
            markdown={listing.markdown}
            vault={listing.vault}
            pressed={value === herePath}
            blocked={blocked === here}
            rowClass="folder-picker-row-this"
            buttonRef={firstRow}
            onPick={() => pick(here, listing.vault)}
          />
          {listing.folders.map((folder) => (
            <FolderRow
              key={folder.path}
              label={folder.name}
              name={folder.name}
              markdown={folder.markdown}
              vault={folder.vault}
              pressed={value === mountFolderPath(listing.root, folder.path)}
              blocked={blocked === folder.path}
              onPick={() => pick(folder.path, folder.vault)}
            >
              {folder.has_subfolders ? (
                <button
                  type="button"
                  className="folder-picker-open"
                  aria-label={`Open ${folder.name}`}
                  onClick={() => navigate(folder.path)}
                >
                  <span aria-hidden="true">›</span>
                </button>
              ) : null}
            </FolderRow>
          ))}
        </ul>
      </div>
      {listing.skipped_invalid_names > 0 ? (
        <p className="folder-picker-status">
          {listing.skipped_invalid_names === 1
            ? "1 folder is not shown because its name cannot be read."
            : `${listing.skipped_invalid_names} folders are not shown because their names cannot be read.`}
        </p>
      ) : null}
      {loading ? (
        <p className="folder-picker-status" role="status">
          Looking for folders…
        </p>
      ) : null}
      <p className="folder-picker-picked">
        {value ? (
          <>
            Folder <code>{value}</code>
          </>
        ) : (
          "Pick the folder that holds your notes."
        )}
      </p>
      {outsideNote}
    </div>
  );
}

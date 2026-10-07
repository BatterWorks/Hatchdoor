import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { ReactNode, Ref } from "react";

import { CONTEXTUAL_HELP, ContextualHelpLink } from "../help";
import type {
  FolderListing,
  FolderListingEntry,
  FolderNoteCount,
  FolderVaultRef,
} from "../../types";
import {
  createFolder,
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

/** The Vaults each folder met so far is rooted at, keyed by the folder's
 * mount-relative path. A folder inside a Vault can only be reached through a
 * listing that showed that Vault, so this is enough to know when the folder
 * on screen belongs to one. */
type VaultsSeen = Record<string, FolderVaultRef>;

function vaultsIn(listing: FolderListing): VaultsSeen {
  const seen: VaultsSeen = {};
  if (listing.vault) seen[listing.path] = listing.vault;
  for (const folder of listing.folders)
    if (folder.vault) seen[folder.path] = folder.vault;
  return seen;
}

/** The Vault the folder at `path` is, or sits inside. */
function owningVault(seen: VaultsSeen, path: string): FolderVaultRef | null {
  for (const [root, vault] of Object.entries(seen))
    if (root === "" || path === root || path.startsWith(`${root}/`))
      return vault;
  return null;
}

/** The refusals that are about the server's folders rather than the name
 * typed, so the reader needs the manual more than another try. */
const NEEDS_HELP = new Set([
  "folder_not_writable",
  "folder_mount_not_found",
  "folder_parent_not_found",
]);

/** The listing's own order, by code unit, so a new folder lands where the
 * next listing will show it. */
function withFolder(
  listing: FolderListing,
  folder: FolderListingEntry,
): FolderListing {
  const folders = [
    ...listing.folders.filter((entry) => entry.path !== folder.path),
    folder,
  ].sort((left, right) =>
    left.name < right.name ? -1 : left.name > right.name ? 1 : 0,
  );
  return { ...listing, folders };
}

/** "New folder" under the list (#494, ADR-44): a name prompt that makes one
 * empty folder in the folder on screen. Its buttons never submit, and Enter
 * in the name is caught here, because the picker sits inside the Add a Vault
 * form. */
function NewFolder({
  parentName,
  onCreate,
}: {
  parentName: string;
  onCreate: (
    name: string,
  ) => Promise<{ ok: true } | { ok: false; code?: string; message: string }>;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{
    code?: string;
    message: string;
  } | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const restoreFocus = useRef(false);
  const errorId = useId();

  useEffect(() => {
    if (!open && restoreFocus.current) {
      restoreFocus.current = false;
      trigger.current?.focus();
    }
  }, [open]);

  const close = () => {
    restoreFocus.current = true;
    setOpen(false);
    setName("");
    setFailure(null);
  };

  const trimmed = name.trim();
  const submit = async () => {
    if (!trimmed || busy) return;
    setBusy(true);
    setFailure(null);
    const result = await onCreate(trimmed);
    setBusy(false);
    if (result.ok) {
      setOpen(false);
      setName("");
    } else {
      setFailure(result);
    }
  };

  if (!open) {
    return (
      <div className="folder-picker-new">
        <button
          ref={trigger}
          type="button"
          className="folder-picker-new-open"
          onClick={() => setOpen(true)}
        >
          <span aria-hidden="true">+</span> New folder
        </button>
      </div>
    );
  }

  return (
    <div className="folder-picker-new" role="group" aria-label="New folder">
      <label className="folder-picker-new-label">
        <span>
          Name of the new folder in <strong>{parentName}</strong>
        </span>
        <input
          className="settings-input"
          value={name}
          autoFocus
          autoComplete="off"
          spellCheck={false}
          maxLength={255}
          aria-invalid={failure ? true : undefined}
          aria-describedby={failure ? errorId : undefined}
          onChange={(event) => {
            setName(event.target.value);
            setFailure(null);
          }}
          onKeyDown={(event) => {
            if (event.key !== "Enter") return;
            event.preventDefault();
            void submit();
          }}
        />
      </label>
      <div className="folder-picker-new-actions">
        <button
          type="button"
          className="settings-btn settings-btn-hot"
          disabled={!trimmed || busy}
          onClick={() => void submit()}
        >
          {busy ? "Creating…" : "Create folder"}
        </button>
        <button
          type="button"
          className="settings-btn"
          disabled={busy}
          onClick={close}
        >
          Cancel
        </button>
      </div>
      {failure ? (
        <p id={errorId} className="folder-picker-new-error" role="alert">
          {failure.message}
          {failure.code && NEEDS_HELP.has(failure.code) ? (
            <>
              {" "}
              <ContextualHelpLink to={CONTEXTUAL_HELP.newFolderRefused} />
            </>
          ) : null}
        </p>
      ) : null}
    </div>
  );
}

/** One pickable folder: the whole row picks it, and a folder that is
 * already a Vault refuses the pick with a line naming that Vault. */
function FolderRow({
  label,
  name,
  path,
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
  /** Mount-relative, so a folder just made can be found and focused. */
  path: string;
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
        data-folder-path={path}
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
  const [vaultsSeen, setVaultsSeen] = useState<VaultsSeen>({});
  const request = useRef(0);
  const firstRow = useRef<HTMLButtonElement>(null);
  const moveFocus = useRef(false);
  const list = useRef<HTMLUListElement>(null);
  const created = useRef<string | null>(null);

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
      setVaultsSeen((seen) => ({ ...seen, ...vaultsIn(result.listing) }));
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
    // A folder just made takes focus instead, which also scrolls it into
    // view in a long list.
    if (listing && created.current !== null) {
      const path = created.current;
      created.current = null;
      for (const row of list.current?.querySelectorAll<HTMLElement>(
        "[data-folder-path]",
      ) ?? [])
        if (row.dataset.folderPath === path) row.focus();
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
          ? " To start with an empty Vault, pick a folder below or make a new one."
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

  const owner = owningVault(vaultsSeen, here);

  const create = async (name: string) => {
    const result = await createFolder(here, name);
    if (!result.ok) return result;
    const { folder } = result;
    created.current = folder.path;
    // The reader may have moved on while it was being made; the folder is
    // still theirs, so it is picked either way.
    setListing((current) =>
      current && current.path === here ? withFolder(current, folder) : current,
    );
    setBlocked(null);
    onPick(mountFolderPath(listing.root, folder.path));
    return { ok: true as const };
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
        <ul ref={list} className="folder-picker-list" aria-label="Folders">
          <FolderRow
            label={`Use ${hereName}`}
            name={hereName}
            path={here}
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
              path={folder.path}
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
        {owner ? (
          <p className="folder-picker-new folder-picker-new-off">
            No new folder here: this folder belongs to the Vault{" "}
            <strong>{owner.name}</strong>.
          </p>
        ) : (
          // Keyed by the folder on screen, so a half-typed name does not
          // follow the reader into another folder.
          <NewFolder key={here} parentName={hereName} onCreate={create} />
        )}
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

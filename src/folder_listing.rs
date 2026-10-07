//! The folders Hatchdoor can see under its Vault mount (ADR-41), so a person
//! adding notes picks a folder from a list instead of typing a container path.
//!
//! This is the one read that walks the filesystem outside any Vault, so its
//! limits are the whole contract: it starts at the configured Vault root and
//! never leaves it, never follows a symlink (which also rules out loops),
//! skips hidden folders and any folder the Vault registry would refuse as a
//! Vault, and reports folder names and Markdown counts only, never a file
//! name or any content. The listing opens no file and writes nothing.
//!
//! Beside it sits the one write Hatchdoor makes outside a Vault and its own
//! state (ADR-44): [`create_folder`] makes a single new, empty folder in a
//! folder the listing can show, never inside a registered Vault, and writes
//! nothing into it.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::vault_registry::{VaultId, VaultRegistryState, VaultRegistryStore, VaultSource};

/// How many Markdown notes one folder's count stops at.
pub const MARKDOWN_COUNT_CAP: u64 = 10_000;

/// How long one listing may spend counting before it answers with what it
/// has.
pub const COUNT_TIME_BUDGET: Duration = Duration::from_secs(2);

/// The bounds one listing counts within. A count cut short by either is
/// reported as "at least" what was found.
#[derive(Clone, Copy, Debug)]
pub struct ListingLimits {
    pub markdown_cap: u64,
    pub time_budget: Duration,
}

impl Default for ListingLimits {
    fn default() -> Self {
        Self {
            markdown_cap: MARKDOWN_COUNT_CAP,
            time_budget: COUNT_TIME_BUDGET,
        }
    }
}

/// One listing: the requested folder and its immediate visible subfolders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FolderListing {
    /// The Vault mount as an absolute path: the configured root, made
    /// absolute against the working directory but not resolved through
    /// symlinks. A picked folder's Vault path is this joined with its
    /// relative `path`, the shape `POST /api/v1/vaults` takes.
    pub root: String,
    /// False when the Vault root does not exist (or is not a folder). The
    /// listing is then empty rather than an error.
    pub root_found: bool,
    /// The listed folder, relative to the root, `/`-separated; empty for the
    /// root itself.
    pub path: String,
    /// Markdown notes in the listed folder, counted recursively. For the root
    /// this says whether a single-folder mount can be picked directly.
    pub markdown: MarkdownCount,
    /// The registered Vault whose root is exactly the listed folder.
    pub vault: Option<RegisteredVault>,
    pub folders: Vec<FolderEntry>,
    /// Immediate subfolders left out because their names are not valid
    /// UTF-8.
    pub skipped_invalid_names: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FolderEntry {
    pub name: String,
    /// Relative to the root, `/`-separated.
    pub path: String,
    pub markdown: MarkdownCount,
    pub vault: Option<RegisteredVault>,
    /// Whether the folder has a subfolder this listing would show, so the
    /// picker can open it. True when counting stopped before this was known.
    pub has_subfolders: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MarkdownCount {
    pub count: u64,
    /// True when the cap or the time budget cut the count short: there are at
    /// least `count` notes.
    pub at_least: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RegisteredVault {
    pub vault_id: VaultId,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderListingError {
    /// The requested path is absolute or climbs out of the root.
    OutsideRoot,
    /// The requested path is not a folder this listing shows: missing, a
    /// file, a symlink, hidden, or forbidden as a Vault.
    NotFound,
    /// The folder exists but Hatchdoor may not read it.
    Unreadable,
}

impl FolderListingError {
    pub fn code(self) -> &'static str {
        match self {
            Self::OutsideRoot => "folder_outside_root",
            Self::NotFound => "folder_not_found",
            Self::Unreadable => "folder_unreadable",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::OutsideRoot => {
                "The path must be relative to the Vault folder and stay inside it."
            }
            Self::NotFound => "No folder this listing can show exists at that path.",
            Self::Unreadable => "Hatchdoor does not have permission to read that folder.",
        }
    }
}

/// Why [`create_folder`] made nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderCreateError {
    /// The name is not one plain path segment, or names a folder the listing
    /// hides.
    InvalidName,
    /// Something already has that name in the parent folder.
    NameTaken,
    /// The Vault mount itself does not exist.
    MountNotFound,
    /// The parent is not a folder the listing shows: missing, a file, a
    /// symlink, hidden, or instance state.
    ParentNotFound,
    /// The parent is absolute, climbs out of the mount, or resolves outside
    /// it.
    OutsideRoot,
    /// The parent is a registered Vault or sits inside one, or the new folder
    /// would land on a registered Vault's own path.
    InsideVault,
    /// Hatchdoor may not write to the parent folder.
    NotWritable,
    /// The Vault registry cannot be read, so nothing says which folders
    /// belong to a Vault. Refusing is the safe answer.
    VaultsUnknown,
}

impl FolderCreateError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidName => "folder_name_invalid",
            Self::NameTaken => "folder_name_taken",
            Self::MountNotFound => "folder_mount_not_found",
            Self::ParentNotFound => "folder_parent_not_found",
            Self::OutsideRoot => "folder_outside_root",
            Self::InsideVault => "folder_inside_vault",
            Self::NotWritable => "folder_not_writable",
            Self::VaultsUnknown => "folder_vaults_unknown",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidName => {
                "That name cannot be used. A folder name cannot be empty, start with a dot or a space, end with a space, or contain a slash, and it cannot be a folder Hatchdoor keeps its own data in."
            }
            Self::NameTaken => "A folder with this name already exists.",
            Self::MountNotFound => {
                "The folder Hatchdoor looks for Vaults in does not exist, so there is nowhere to make a new folder."
            }
            Self::ParentNotFound => {
                "The folder to make it in is no longer there. Go back up and try again."
            }
            Self::OutsideRoot => FolderListingError::OutsideRoot.message(),
            Self::InsideVault => {
                "This place belongs to a Vault. A new folder can only be made outside every Vault."
            }
            Self::NotWritable => {
                "Hatchdoor could not make a folder here. The folder is read-only, or Hatchdoor is not allowed to write to it."
            }
            Self::VaultsUnknown => {
                "Hatchdoor cannot read its list of Vaults right now, so it cannot tell whether this folder belongs to one. Nothing was made."
            }
        }
    }
}

/// The longest name most filesystems accept for one folder, in bytes.
const MAX_NAME_BYTES: usize = 255;

/// Make one new, empty folder called `name` in the folder at `parent` under
/// `root`, and answer with it as the listing would show it (ADR-44).
///
/// The parent is found by the listing's own rules, so it is a folder
/// [`list_folders`] can show, reached without following a symlink. Exactly
/// one directory is created and nothing is written into it. The checks run
/// before the directory is made, not atomically with it.
pub fn create_folder(
    root: &Path,
    parent: &str,
    name: &str,
    registry: &VaultRegistryStore,
) -> Result<FolderEntry, FolderCreateError> {
    let components = parse_relative(parent).map_err(parent_error)?;
    if !is_plain_name(name) {
        return Err(FolderCreateError::InvalidName);
    }
    let Some(root) = resolve_root(root).map_err(parent_error)? else {
        return Err(FolderCreateError::MountNotFound);
    };
    let parent = shown_folder(&root, &components, registry).map_err(parent_error)?;
    // The walk followed no symlink, so this only fails when a folder was
    // swapped for a link after it was checked.
    match parent.canonicalize() {
        Ok(resolved) if resolved.starts_with(&root) => {}
        Ok(_) => return Err(FolderCreateError::OutsideRoot),
        Err(error) => return Err(parent_error(read_error(error))),
    }

    let target = parent.join(name);
    let Some(vaults) = vault_roots(registry) else {
        return Err(FolderCreateError::VaultsUnknown);
    };
    if vaults.iter().any(|vault| target.starts_with(vault)) {
        return Err(FolderCreateError::InsideVault);
    }
    // A name the listing would hide as instance state is as unusable as a
    // hidden one.
    if !may_be_vault(registry, &target) {
        return Err(FolderCreateError::InvalidName);
    }

    fs::create_dir(&target).map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => FolderCreateError::NameTaken,
        io::ErrorKind::NotFound => FolderCreateError::ParentNotFound,
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidFilename => {
            FolderCreateError::InvalidName
        }
        _ => FolderCreateError::NotWritable,
    })?;

    let prefix = components.join("/");
    Ok(FolderEntry {
        path: if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        },
        name: name.to_owned(),
        markdown: MarkdownCount::default(),
        vault: None,
        has_subfolders: false,
    })
}

fn parent_error(error: FolderListingError) -> FolderCreateError {
    match error {
        FolderListingError::OutsideRoot => FolderCreateError::OutsideRoot,
        FolderListingError::NotFound => FolderCreateError::ParentNotFound,
        FolderListingError::Unreadable => FolderCreateError::NotWritable,
    }
}

/// One path segment the listing would show: not empty, not hidden (which
/// rules out `.` and `..`), no separator or control character, and no space
/// at either end, which some filesystems drop without saying.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name.trim() == name
        && !is_hidden(name)
        && !name
            .chars()
            .any(|character| matches!(character, '/' | '\\') || character.is_control())
}

/// List the folder at `relative` under `root`. `registry` supplies both the
/// already-registered Vaults and the containment rule that hides instance
/// state.
pub fn list_folders(
    root: &Path,
    relative: &str,
    registry: &VaultRegistryStore,
    limits: ListingLimits,
) -> Result<FolderListing, FolderListingError> {
    let components = parse_relative(relative)?;
    let configured_root = absolute_root(root);
    let Some(root) = resolve_root(root)? else {
        return Ok(FolderListing::root_missing(configured_root));
    };
    let target = shown_folder(&root, &components, registry)?;

    let vaults = registered_vaults(registry);
    let deadline = Instant::now() + limits.time_budget;
    let mut direct_markdown = 0u64;
    let mut skipped_invalid_names = 0u64;
    let mut children = Vec::new();
    for entry in fs::read_dir(&target).map_err(read_error)? {
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        if file_type.is_file() {
            if is_markdown(&name) {
                direct_markdown += 1;
            }
            continue;
        }
        // `file_type` does not follow links, so a symlink to a folder is
        // neither a file nor a folder here and is dropped.
        if !file_type.is_dir() {
            continue;
        }
        let Some(name) = name.to_str() else {
            skipped_invalid_names += 1;
            continue;
        };
        if is_hidden(name) {
            continue;
        }
        let path = target.join(name);
        if !may_be_vault(registry, &path) {
            continue;
        }
        children.push((name.to_owned(), path));
    }
    children.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut markdown = MarkdownCount {
        count: direct_markdown,
        at_least: false,
    };
    let prefix = components.join("/");
    let folders = children
        .into_iter()
        .map(|(name, path)| {
            let tree = count_tree(&path, limits.markdown_cap, deadline);
            markdown.count += tree.markdown.count;
            markdown.at_least |= tree.markdown.at_least;
            FolderEntry {
                path: if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                },
                name,
                vault: vault_at(&vaults, &path),
                markdown: tree.markdown,
                has_subfolders: tree.has_subfolders,
            }
        })
        .collect();

    Ok(FolderListing {
        root: configured_root,
        root_found: true,
        vault: vault_at(&vaults, &target),
        path: prefix,
        markdown,
        folders,
        skipped_invalid_names,
    })
}

impl FolderListing {
    fn root_missing(root: String) -> Self {
        Self {
            root,
            root_found: false,
            path: String::new(),
            markdown: MarkdownCount::default(),
            vault: None,
            folders: Vec::new(),
            skipped_invalid_names: 0,
        }
    }
}

/// The Vault mount resolved, or `None` when it does not exist or is not a
/// folder. The root is the operator's own configuration, so it is resolved
/// once, symlinks and all. Nothing below it is followed.
fn resolve_root(root: &Path) -> Result<Option<PathBuf>, FolderListingError> {
    match fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => root.canonicalize().map(Some).map_err(read_error),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(read_error(error)),
    }
}

/// The folder at `components` under the resolved `root`, when it is one the
/// listing shows: every step a real folder, none hidden, none a symlink, and
/// the folder itself not instance state.
fn shown_folder(
    root: &Path,
    components: &[&str],
    registry: &VaultRegistryStore,
) -> Result<PathBuf, FolderListingError> {
    let mut target = root.to_path_buf();
    for component in components {
        if is_hidden(component) {
            return Err(FolderListingError::NotFound);
        }
        target.push(component);
        let metadata = fs::symlink_metadata(&target).map_err(read_error)?;
        if !metadata.is_dir() {
            return Err(FolderListingError::NotFound);
        }
    }
    if !components.is_empty() && !may_be_vault(registry, &target) {
        return Err(FolderListingError::NotFound);
    }
    Ok(target)
}

/// The configured root as the absolute path a Vault definition would store.
/// It comes from `VAULT_PATH`, an environment string, so the lossy
/// conversion only matters for a working directory that is not UTF-8.
fn absolute_root(root: &Path) -> String {
    std::path::absolute(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Split a `/`-separated relative path, refusing anything that could name a
/// place outside the root. Empty and `.` segments are ignored.
fn parse_relative(relative: &str) -> Result<Vec<&str>, FolderListingError> {
    if relative.starts_with('/') || relative.starts_with('\\') {
        return Err(FolderListingError::OutsideRoot);
    }
    let mut components = Vec::new();
    for component in relative.split('/') {
        match component {
            "" | "." => {}
            ".." => return Err(FolderListingError::OutsideRoot),
            _ if component.contains('\0') => return Err(FolderListingError::NotFound),
            _ => components.push(component),
        }
    }
    Ok(components)
}

fn read_error(error: io::Error) -> FolderListingError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => FolderListingError::Unreadable,
        _ => FolderListingError::NotFound,
    }
}

/// `.git`, `.obsidian`, `.trash`, Hatchdoor's own `.hatchdoor-trash` and any
/// other dot-folder.
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// The test the Vault index applies when it collects notes.
fn is_markdown(name: &OsStr) -> bool {
    Path::new(name).extension() == Some(OsStr::new("md"))
}

/// Whether the registry would accept this folder as a local Vault root as
/// far as instance state goes: it neither contains nor sits inside a
/// directory Hatchdoor keeps its own state in.
fn may_be_vault(registry: &VaultRegistryStore, path: &Path) -> bool {
    registry
        .ensure_outside_instance_state(&VaultSource::Local {
            path: path.to_path_buf(),
        })
        .is_ok()
}

/// Every registered Vault's root, resolved. A Vault whose folder is missing
/// cannot match a listed folder, and a registry in recovery lists none.
fn registered_vaults(registry: &VaultRegistryStore) -> Vec<(PathBuf, RegisteredVault)> {
    let Ok(VaultRegistryState::Ready(snapshot)) = registry.load() else {
        return Vec::new();
    };
    snapshot
        .definitions()
        .filter_map(|definition| {
            let path = registry.vault_path(&definition).canonicalize().ok()?;
            Some((
                path,
                RegisteredVault {
                    vault_id: definition.vault_id(),
                    name: definition.name().to_owned(),
                },
            ))
        })
        .collect()
}

/// Every registered Vault's root as [`create_folder`] must respect it, or
/// `None` when the registry cannot say (unreadable, or in recovery). Unlike
/// [`registered_vaults`] this keeps a Vault whose folder is missing, resolved
/// through its parent, so a new folder cannot land where that Vault expects
/// its notes.
fn vault_roots(registry: &VaultRegistryStore) -> Option<Vec<PathBuf>> {
    let Ok(VaultRegistryState::Ready(snapshot)) = registry.load() else {
        return None;
    };
    let roots = snapshot
        .definitions()
        .map(|definition| {
            let path = registry.vault_path(&definition);
            path.canonicalize().unwrap_or_else(|_| {
                match (path.parent().map(Path::canonicalize), path.file_name()) {
                    (Some(Ok(parent)), Some(name)) => parent.join(name),
                    _ => std::path::absolute(&path).unwrap_or(path),
                }
            })
        })
        .collect();
    Some(roots)
}

fn vault_at(vaults: &[(PathBuf, RegisteredVault)], path: &Path) -> Option<RegisteredVault> {
    vaults
        .iter()
        .find(|(root, _)| root == path)
        .map(|(_, vault)| vault.clone())
}

struct TreeCount {
    markdown: MarkdownCount,
    has_subfolders: bool,
}

/// Count the Markdown notes under `folder` by the listing's own rules (no
/// symlinks, no hidden or non-UTF-8 folders), stopping at `cap` notes or at
/// `deadline`, and say whether `folder` itself has a visible subfolder.
fn count_tree(folder: &Path, cap: u64, deadline: Instant) -> TreeCount {
    let mut count = 0u64;
    let mut has_subfolders = false;
    let mut top_level_done = false;
    let mut pending = vec![folder.to_path_buf()];
    let mut finished = true;
    'walk: while let Some(directory) = pending.pop() {
        if Instant::now() >= deadline {
            finished = false;
            break;
        }
        let top_level = directory == folder;
        let Ok(entries) = fs::read_dir(&directory) else {
            top_level_done |= top_level;
            continue;
        };
        for entry in entries {
            if count >= cap || Instant::now() >= deadline {
                finished = false;
                break 'walk;
            }
            let Ok(entry) = entry else { continue };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            if file_type.is_file() {
                count += u64::from(is_markdown(&name));
            } else if file_type.is_dir()
                && let Some(name) = name.to_str()
                && !is_hidden(name)
            {
                has_subfolders |= top_level;
                pending.push(directory.join(name));
            }
        }
        top_level_done |= top_level;
    }
    TreeCount {
        markdown: MarkdownCount {
            count,
            at_least: !finished,
        },
        has_subfolders: has_subfolders || !top_level_done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault_registry::NewVaultDefinition;
    use std::os::unix::ffi::OsStrExt;
    use tempfile::TempDir;

    struct Fixture {
        _dir: TempDir,
        root: PathBuf,
        registry: VaultRegistryStore,
    }

    /// A Vault mount at `<tmp>/mount` with the registry's state beside it,
    /// outside the mount, as the stock Compose file lays them out.
    fn fixture() -> Fixture {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("mount");
        fs::create_dir(&root).unwrap();
        let registry = VaultRegistryStore::new(dir.path().join("state/vaults.json"));
        Fixture {
            _dir: dir,
            root,
            registry,
        }
    }

    fn write(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "# note\n").unwrap();
    }

    fn list(fixture: &Fixture, relative: &str) -> Result<FolderListing, FolderListingError> {
        list_folders(
            &fixture.root,
            relative,
            &fixture.registry,
            ListingLimits::default(),
        )
    }

    fn names(listing: &FolderListing) -> Vec<&str> {
        listing.folders.iter().map(|f| f.name.as_str()).collect()
    }

    fn exact(count: u64) -> MarkdownCount {
        MarkdownCount {
            count,
            at_least: false,
        }
    }

    fn register(fixture: &Fixture, name: &str, path: PathBuf) -> VaultId {
        let VaultRegistryState::Ready(snapshot) = fixture.registry.load().unwrap() else {
            panic!("registry in recovery");
        };
        fixture
            .registry
            .add(
                snapshot.revision(),
                NewVaultDefinition {
                    name: name.to_owned(),
                    enabled: true,
                    source: VaultSource::Local { path },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .unwrap()
            .definitions()
            .find(|definition| definition.name() == name)
            .unwrap()
            .vault_id()
    }

    #[test]
    fn lists_subfolders_with_counts_vault_and_subfolder_flags() {
        let fixture = fixture();
        write(&fixture.root.join("Work/a.md"));
        write(&fixture.root.join("Work/Projects/b.md"));
        write(&fixture.root.join("Work/Projects/deep/c.md"));
        write(&fixture.root.join("Work/notes.txt"));
        write(&fixture.root.join("Personal/journal.md"));
        fs::create_dir(fixture.root.join("Empty")).unwrap();
        write(&fixture.root.join("top.md"));
        let work = register(&fixture, "Work", fixture.root.join("Work"));

        let listing = list(&fixture, "").unwrap();

        assert!(listing.root_found);
        assert_eq!(listing.path, "");
        assert_eq!(listing.markdown, exact(5));
        assert_eq!(listing.vault, None);
        assert_eq!(names(&listing), ["Empty", "Personal", "Work"]);
        let empty = &listing.folders[0];
        assert_eq!(empty.markdown, exact(0));
        assert!(!empty.has_subfolders);
        let personal = &listing.folders[1];
        assert_eq!(personal.path, "Personal");
        assert_eq!(personal.markdown, exact(1));
        assert!(!personal.has_subfolders);
        assert_eq!(personal.vault, None);
        let entry = &listing.folders[2];
        assert_eq!(entry.markdown, exact(3));
        assert!(entry.has_subfolders);
        assert_eq!(
            entry.vault,
            Some(RegisteredVault {
                vault_id: work,
                name: "Work".to_owned(),
            })
        );

        let nested = list(&fixture, "Work/").unwrap();
        assert_eq!(nested.path, "Work");
        assert_eq!(nested.markdown, exact(3));
        assert_eq!(
            nested.vault.as_ref().map(|vault| vault.vault_id),
            Some(work)
        );
        assert_eq!(names(&nested), ["Projects"]);
        assert_eq!(nested.folders[0].path, "Work/Projects");
        assert_eq!(nested.folders[0].markdown, exact(2));
        assert!(nested.folders[0].has_subfolders);
    }

    #[test]
    fn every_listing_names_the_mount_as_an_absolute_path() {
        let fixture = fixture();
        fs::create_dir(fixture.root.join("Work")).unwrap();

        assert_eq!(
            list(&fixture, "").unwrap().root,
            fixture.root.to_str().unwrap()
        );
        assert_eq!(
            list(&fixture, "Work").unwrap().root,
            fixture.root.to_str().unwrap()
        );

        let missing = fixture.root.join("absent");
        let listing =
            list_folders(&missing, "", &fixture.registry, ListingLimits::default()).unwrap();
        assert_eq!(listing.root, missing.to_str().unwrap());

        // A relative configuration (`./vault` when `VAULT_PATH` is unset) is
        // reported against the working directory, the way the registry
        // resolves a Vault path.
        let expected = std::env::current_dir().unwrap().join("vault");
        for configured in ["vault", "./vault"] {
            let relative = list_folders(
                Path::new(configured),
                "",
                &fixture.registry,
                ListingLimits::default(),
            )
            .unwrap();
            assert_eq!(relative.root, expected.to_str().unwrap());
        }
    }

    #[test]
    fn a_registered_root_is_flagged_on_the_root_listing() {
        let fixture = fixture();
        write(&fixture.root.join("only.md"));
        let id = register(&fixture, "Mount", fixture.root.clone());

        let listing = list(&fixture, "").unwrap();

        assert_eq!(listing.markdown, exact(1));
        assert_eq!(listing.vault.map(|vault| vault.vault_id), Some(id));
    }

    #[test]
    fn a_missing_root_is_an_empty_listing_not_an_error() {
        let fixture = fixture();
        let missing = fixture.root.join("absent");
        for relative in ["", "anything"] {
            let listing = list_folders(
                &missing,
                relative,
                &fixture.registry,
                ListingLimits::default(),
            )
            .unwrap();
            assert!(!listing.root_found);
            assert!(listing.folders.is_empty());
        }
    }

    #[test]
    fn a_root_that_is_a_file_counts_as_missing() {
        let fixture = fixture();
        let file = fixture.root.join("file");
        fs::write(&file, "x").unwrap();
        let listing = list_folders(&file, "", &fixture.registry, ListingLimits::default()).unwrap();
        assert!(!listing.root_found);
    }

    #[test]
    fn paths_that_leave_the_root_are_refused() {
        let fixture = fixture();
        fs::create_dir(fixture.root.join("Work")).unwrap();
        for relative in ["../..", "..", "Work/../..", "Work/..", "/etc", "\\etc"] {
            assert_eq!(
                list(&fixture, relative),
                Err(FolderListingError::OutsideRoot),
                "{relative}"
            );
        }
    }

    #[test]
    fn missing_paths_files_and_hidden_folders_are_not_found() {
        let fixture = fixture();
        write(&fixture.root.join("note.md"));
        fs::create_dir_all(fixture.root.join(".git/objects")).unwrap();
        for relative in ["absent", "note.md", ".git", ".git/objects", "a\0b"] {
            assert_eq!(
                list(&fixture, relative),
                Err(FolderListingError::NotFound),
                "{relative:?}"
            );
        }
    }

    #[test]
    fn no_symlink_is_followed_inside_or_outside_the_root() {
        let fixture = fixture();
        let outside = fixture._dir.path().join("outside");
        write(&outside.join("secret.md"));
        write(&fixture.root.join("Real/a.md"));
        std::os::unix::fs::symlink(&outside, fixture.root.join("Escape")).unwrap();
        std::os::unix::fs::symlink(fixture.root.join("Real"), fixture.root.join("Alias")).unwrap();
        // A loop back to the root inside a counted folder.
        std::os::unix::fs::symlink(&fixture.root, fixture.root.join("Real/loop")).unwrap();
        std::os::unix::fs::symlink(
            outside.join("secret.md"),
            fixture.root.join("Real/linked.md"),
        )
        .unwrap();

        let listing = list(&fixture, "").unwrap();

        assert_eq!(names(&listing), ["Real"]);
        assert_eq!(listing.folders[0].markdown, exact(1));
        assert!(!listing.folders[0].has_subfolders);
        assert_eq!(listing.markdown, exact(1));
        for relative in ["Escape", "Alias", "Real/loop", "Escape/x"] {
            assert_eq!(
                list(&fixture, relative),
                Err(FolderListingError::NotFound),
                "{relative}"
            );
        }
    }

    #[test]
    fn a_symlinked_root_is_resolved_once() {
        let fixture = fixture();
        write(&fixture.root.join("Work/a.md"));
        let link = fixture._dir.path().join("mount-link");
        std::os::unix::fs::symlink(&fixture.root, &link).unwrap();
        let listing = list_folders(&link, "", &fixture.registry, ListingLimits::default()).unwrap();
        assert_eq!(names(&listing), ["Work"]);
    }

    #[test]
    fn hidden_and_reserved_folders_are_skipped() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("mount");
        for hidden in [".git", ".obsidian", ".trash", ".hatchdoor-trash"] {
            write(&root.join(hidden).join("x.md"));
        }
        write(&root.join("Notes/.obsidian/workspace.md"));
        write(&root.join("Notes/a.md"));
        // Instance state inside the mount: the cache directory, and a folder
        // that holds the settings directory.
        fs::create_dir_all(root.join("cache")).unwrap();
        fs::create_dir_all(root.join("config/settings")).unwrap();
        let registry = VaultRegistryStore::new(dir.path().join("state/vaults.json"))
            .with_reserved_directories([root.join("cache"), root.join("config/settings")]);

        let listing = list_folders(&root, "", &registry, ListingLimits::default()).unwrap();

        assert_eq!(names(&listing), ["Notes"]);
        assert_eq!(listing.folders[0].markdown, exact(1));
        assert!(!listing.folders[0].has_subfolders);
        for relative in ["cache", "config", "config/settings"] {
            assert_eq!(
                list_folders(&root, relative, &registry, ListingLimits::default()),
                Err(FolderListingError::NotFound),
                "{relative}"
            );
        }
    }

    #[test]
    fn invalid_utf8_names_are_skipped_and_counted() {
        let fixture = fixture();
        let invalid = OsStr::from_bytes(b"bad-\xff-name");
        write(&fixture.root.join(invalid).join("a.md"));
        write(&fixture.root.join("Good/a.md"));
        write(&fixture.root.join("Good").join(invalid).join("b.md"));
        // Non-Latin, right-to-left, emoji, and NFC/NFD twins are valid names.
        for name in ["日本語", "עברית", "📓 Notes", "Caf\u{e9}", "Cafe\u{301}"] {
            fs::create_dir(fixture.root.join(name)).unwrap();
        }

        let listing = list(&fixture, "").unwrap();

        assert_eq!(listing.skipped_invalid_names, 1);
        assert_eq!(listing.folders.len(), 6);
        assert!(names(&listing).contains(&"Caf\u{e9}"));
        assert!(names(&listing).contains(&"Cafe\u{301}"));
        let good = listing.folders.iter().find(|f| f.name == "Good").unwrap();
        assert_eq!(good.markdown, exact(1));
        assert!(!good.has_subfolders);
    }

    fn create(
        fixture: &Fixture,
        parent: &str,
        name: &str,
    ) -> Result<FolderEntry, FolderCreateError> {
        create_folder(&fixture.root, parent, name, &fixture.registry)
    }

    /// Every path under the fixture's temporary directory, so a test can show
    /// a refusal changed nothing anywhere, inside the mount or beside it.
    fn every_path(fixture: &Fixture) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![fixture._dir.path().to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    pending.push(entry.path());
                }
                found.push(entry.path());
            }
        }
        found.sort();
        found
    }

    #[test]
    fn creates_one_empty_folder_at_the_mount_and_inside_a_subfolder() {
        let fixture = fixture();
        write(&fixture.root.join("Work/a.md"));
        let before = every_path(&fixture);

        let top = create(&fixture, "", "Journal").unwrap();

        assert_eq!(
            top,
            FolderEntry {
                name: "Journal".to_owned(),
                path: "Journal".to_owned(),
                markdown: exact(0),
                vault: None,
                has_subfolders: false,
            }
        );
        let nested = create(&fixture, "Work/", "Projects").unwrap();
        assert_eq!(nested.name, "Projects");
        assert_eq!(nested.path, "Work/Projects");

        // Exactly the two folders, and nothing inside either.
        let mut expected = before;
        expected.push(fixture.root.join("Journal"));
        expected.push(fixture.root.join("Work/Projects"));
        expected.sort();
        assert_eq!(every_path(&fixture), expected);
        // The listing shows what was made, as it was answered.
        let listing = list(&fixture, "").unwrap();
        assert_eq!(listing.folders[0], top);
        assert_eq!(list(&fixture, "Work").unwrap().folders, [nested]);
    }

    #[test]
    fn a_new_folder_keeps_the_name_as_typed_in_any_script() {
        let fixture = fixture();
        for name in ["日本語", "עברית", "📓 Notes", "Caf\u{e9}", "My Notes 2026"] {
            let entry = create(&fixture, "", name).unwrap();
            assert_eq!(entry.name, name);
            assert!(fixture.root.join(name).is_dir(), "{name}");
        }
    }

    #[test]
    fn names_that_are_not_one_plain_segment_are_refused() {
        let fixture = fixture();
        fs::create_dir(fixture.root.join("Work")).unwrap();
        let before = every_path(&fixture);
        let long = "x".repeat(MAX_NAME_BYTES + 1);
        for name in [
            "",
            " ",
            ".",
            "..",
            ".git",
            ".hidden",
            "a/b",
            "/a",
            "a/",
            "../up",
            "a\\b",
            "a\0b",
            "line\nbreak",
            " lead",
            "trail ",
            long.as_str(),
        ] {
            for parent in ["", "Work"] {
                assert_eq!(
                    create(&fixture, parent, name),
                    Err(FolderCreateError::InvalidName),
                    "{name:?} in {parent:?}"
                );
            }
        }
        assert_eq!(every_path(&fixture), before);
    }

    #[test]
    fn a_name_the_listing_hides_as_instance_state_is_refused() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("mount");
        fs::create_dir(&root).unwrap();
        let registry = VaultRegistryStore::new(dir.path().join("state/vaults.json"))
            .with_reserved_directories([root.join("cache")]);

        assert_eq!(
            create_folder(&root, "", "cache", &registry),
            Err(FolderCreateError::InvalidName)
        );
        assert!(!root.join("cache").exists());
    }

    #[test]
    fn a_taken_name_is_refused_and_the_existing_folder_is_left_alone() {
        let fixture = fixture();
        write(&fixture.root.join("Work/a.md"));
        fs::write(fixture.root.join("file"), "x").unwrap();
        std::os::unix::fs::symlink(fixture.root.join("Work"), fixture.root.join("Alias")).unwrap();
        let before = every_path(&fixture);

        for name in ["Work", "file", "Alias"] {
            assert_eq!(
                create(&fixture, "", name),
                Err(FolderCreateError::NameTaken),
                "{name}"
            );
        }
        assert_eq!(every_path(&fixture), before);
    }

    #[test]
    fn no_chain_of_folders_is_made_for_a_missing_parent() {
        let fixture = fixture();
        write(&fixture.root.join("note.md"));
        fs::create_dir_all(fixture.root.join(".git/objects")).unwrap();
        let before = every_path(&fixture);

        for parent in [
            "Missing",
            "Missing/Deeper",
            "note.md",
            ".git",
            ".git/objects",
        ] {
            assert_eq!(
                create(&fixture, parent, "New"),
                Err(FolderCreateError::ParentNotFound),
                "{parent}"
            );
        }
        assert_eq!(every_path(&fixture), before);
    }

    #[test]
    fn a_parent_that_leaves_the_mount_is_refused() {
        let fixture = fixture();
        fs::create_dir(fixture.root.join("Work")).unwrap();
        let before = every_path(&fixture);

        for parent in [
            "..",
            "../..",
            "Work/..",
            "Work/../..",
            "/etc",
            "/tmp",
            "\\etc",
        ] {
            assert_eq!(
                create(&fixture, parent, "New"),
                Err(FolderCreateError::OutsideRoot),
                "{parent}"
            );
        }
        assert_eq!(every_path(&fixture), before);
    }

    #[test]
    fn no_symlink_is_followed_to_the_parent() {
        let fixture = fixture();
        let outside = fixture._dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::create_dir_all(fixture.root.join("Real/Sub")).unwrap();
        std::os::unix::fs::symlink(&outside, fixture.root.join("Escape")).unwrap();
        std::os::unix::fs::symlink(fixture.root.join("Real"), fixture.root.join("Alias")).unwrap();
        std::os::unix::fs::symlink(&outside, fixture.root.join("Real/out")).unwrap();
        let before = every_path(&fixture);

        for parent in ["Escape", "Alias", "Alias/Sub", "Real/out"] {
            assert_eq!(
                create(&fixture, parent, "New"),
                Err(FolderCreateError::ParentNotFound),
                "{parent}"
            );
        }
        assert_eq!(every_path(&fixture), before);
    }

    #[test]
    fn a_symlinked_mount_is_resolved_once_and_written_through() {
        let fixture = fixture();
        let link = fixture._dir.path().join("mount-link");
        std::os::unix::fs::symlink(&fixture.root, &link).unwrap();

        let entry = create_folder(&link, "", "New", &fixture.registry).unwrap();

        assert_eq!(entry.path, "New");
        assert!(fixture.root.join("New").is_dir());
    }

    #[test]
    fn a_parent_that_is_or_is_inside_a_vault_is_refused() {
        let fixture = fixture();
        write(&fixture.root.join("Work/Projects/a.md"));
        fs::create_dir(fixture.root.join("Personal")).unwrap();
        register(&fixture, "Work", fixture.root.join("Work"));
        let before = every_path(&fixture);

        for parent in ["Work", "Work/Projects"] {
            assert_eq!(
                create(&fixture, parent, "New"),
                Err(FolderCreateError::InsideVault),
                "{parent}"
            );
        }
        assert_eq!(every_path(&fixture), before);
        // Beside the Vault, and in a folder that only holds Vaults, is fine.
        assert!(create(&fixture, "", "Another").is_ok());
        assert!(create(&fixture, "Personal", "New").is_ok());
    }

    #[test]
    fn a_mount_that_is_itself_a_vault_takes_no_new_folder() {
        let fixture = fixture();
        fs::create_dir(fixture.root.join("Sub")).unwrap();
        register(&fixture, "Mount", fixture.root.clone());

        for parent in ["", "Sub"] {
            assert_eq!(
                create(&fixture, parent, "New"),
                Err(FolderCreateError::InsideVault),
                "{parent:?}"
            );
        }
    }

    #[test]
    fn a_new_folder_cannot_land_on_a_vault_whose_folder_is_missing() {
        let fixture = fixture();
        fs::create_dir(fixture.root.join("Gone")).unwrap();
        register(&fixture, "Gone", fixture.root.join("Gone"));
        fs::remove_dir(fixture.root.join("Gone")).unwrap();

        assert_eq!(
            create(&fixture, "", "Gone"),
            Err(FolderCreateError::InsideVault)
        );
        assert!(!fixture.root.join("Gone").exists());
    }

    #[test]
    fn nothing_is_made_while_the_registry_cannot_name_the_vaults() {
        let fixture = fixture();
        fs::create_dir(fixture.root.join("Work")).unwrap();
        register(&fixture, "Work", fixture.root.join("Work"));
        // A registry that no longer parses is in recovery and lists nothing.
        fs::write(fixture._dir.path().join("state/vaults.json"), "{ not json").unwrap();
        let before = every_path(&fixture);

        for parent in ["", "Work"] {
            assert_eq!(
                create(&fixture, parent, "New"),
                Err(FolderCreateError::VaultsUnknown),
                "{parent:?}"
            );
        }
        assert_eq!(every_path(&fixture), before);
    }

    #[test]
    fn a_missing_mount_is_its_own_refusal_and_is_not_created() {
        let fixture = fixture();
        let missing = fixture.root.join("absent");
        for parent in ["", "anything"] {
            assert_eq!(
                create_folder(&missing, parent, "New", &fixture.registry),
                Err(FolderCreateError::MountNotFound)
            );
        }
        assert!(!missing.exists());
    }

    #[test]
    fn a_read_only_parent_is_not_writable() {
        if unsafe { libc::geteuid() } == 0 {
            // Root writes through a read-only mode, so this proves nothing.
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let fixture = fixture();
        fs::create_dir(fixture.root.join("Locked")).unwrap();
        for folder in [fixture.root.join("Locked"), fixture.root.clone()] {
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).unwrap();
        }

        let results = [
            create(&fixture, "", "New"),
            create(&fixture, "Locked", "New"),
        ];

        for folder in [fixture.root.clone(), fixture.root.join("Locked")] {
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        }
        for result in results {
            assert_eq!(result, Err(FolderCreateError::NotWritable));
        }
        assert!(!fixture.root.join("New").exists());
        assert!(!fixture.root.join("Locked/New").exists());
    }

    #[test]
    fn every_creation_refusal_has_its_own_stable_code() {
        let codes = [
            (FolderCreateError::InvalidName, "folder_name_invalid"),
            (FolderCreateError::NameTaken, "folder_name_taken"),
            (FolderCreateError::MountNotFound, "folder_mount_not_found"),
            (FolderCreateError::ParentNotFound, "folder_parent_not_found"),
            (FolderCreateError::OutsideRoot, "folder_outside_root"),
            (FolderCreateError::InsideVault, "folder_inside_vault"),
            (FolderCreateError::NotWritable, "folder_not_writable"),
            (FolderCreateError::VaultsUnknown, "folder_vaults_unknown"),
        ];
        for (error, code) in codes {
            assert_eq!(error.code(), code);
            assert!(!error.message().contains("os error"));
        }
    }

    #[test]
    fn a_tree_over_the_cap_answers_at_least_the_cap() {
        let fixture = fixture();
        for index in 0..20 {
            write(&fixture.root.join(format!("Big/{}/n{index}.md", index % 4)));
        }
        let limits = ListingLimits {
            markdown_cap: 5,
            time_budget: COUNT_TIME_BUDGET,
        };

        let listing = list_folders(&fixture.root, "", &fixture.registry, limits).unwrap();

        let big = &listing.folders[0];
        assert_eq!(
            big.markdown,
            MarkdownCount {
                count: 5,
                at_least: true,
            }
        );
        assert!(big.has_subfolders);
        assert!(listing.markdown.at_least);
    }

    #[test]
    fn a_folder_holding_exactly_the_cap_is_an_exact_count() {
        let fixture = fixture();
        for index in 0..5 {
            write(&fixture.root.join(format!("Full/n{index}.md")));
        }
        let limits = ListingLimits {
            markdown_cap: 5,
            time_budget: COUNT_TIME_BUDGET,
        };
        let listing = list_folders(&fixture.root, "", &fixture.registry, limits).unwrap();
        assert_eq!(listing.folders[0].markdown, exact(5));
    }

    #[test]
    fn a_spent_time_budget_answers_at_least_what_was_counted() {
        let fixture = fixture();
        write(&fixture.root.join("Work/a.md"));
        let limits = ListingLimits {
            markdown_cap: MARKDOWN_COUNT_CAP,
            time_budget: Duration::ZERO,
        };
        let started = Instant::now();

        let listing = list_folders(&fixture.root, "", &fixture.registry, limits).unwrap();

        assert!(started.elapsed() < Duration::from_secs(1));
        let work = &listing.folders[0];
        assert!(work.markdown.at_least);
        // Unknown, so the picker may still open it.
        assert!(work.has_subfolders);
        assert!(listing.markdown.at_least);
    }
}

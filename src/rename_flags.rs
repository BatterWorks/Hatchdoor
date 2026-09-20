//! Whether the filesystem under a directory can perform the `renameat2` flag
//! operations Hatchdoor's write commit points are built on.
//!
//! `renameat2` has been in Linux since 3.15, but each filesystem opts into its
//! flags separately. OpenZFS before 2.2.0 rejects any non-zero flag with
//! `EINVAL`, as does every FUSE filesystem, and `EINVAL` is also what the
//! kernel returns for genuine misuse of the call — exchanging a path with
//! itself, say. Telling those two apart is the whole job of this module: it
//! asks the kernel, on the directory in question, whether the flag works at
//! all, and caches the answer per filesystem. Nothing here looks at a
//! filesystem's name or version (ADR-26).

use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// The `renameat2` flags Hatchdoor commits with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RenameFlag {
    /// Swap two names, which is how a conditional write holds the content it
    /// displaced so it can be inspected and put back.
    Exchange,
    /// Rename only onto a free name, which is how a managed Git checkout is
    /// installed without ever overwriting an existing one.
    NoReplace,
}

impl RenameFlag {
    fn bits(self) -> libc::c_uint {
        match self {
            Self::Exchange => libc::RENAME_EXCHANGE,
            Self::NoReplace => libc::RENAME_NOREPLACE,
        }
    }

    /// The flag's kernel name, for an operator-facing message.
    pub fn name(self) -> &'static str {
        match self {
            Self::Exchange => "RENAME_EXCHANGE",
            Self::NoReplace => "RENAME_NOREPLACE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagSupport {
    Supported,
    Unsupported,
    /// The probe could not reach a verdict: the directory is read-only, does
    /// not exist, or refused the scratch file. A Vault in that state has its
    /// own reason for being unwritable and must not be reported as lacking
    /// compare-and-swap.
    Undetermined,
}

/// Whether `flag` works on the filesystem holding `directory`.
///
/// The first call per filesystem runs a real `renameat2` on two dot-prefixed
/// scratch names inside `directory` and removes them again; later calls read
/// the cached verdict. Only a definite verdict is cached, so a read-only or
/// missing directory is re-probed if it later becomes usable.
pub fn support(directory: &Path, flag: RenameFlag) -> FlagSupport {
    if forced_unsupported(directory) {
        return FlagSupport::Unsupported;
    }
    let key = device_of(directory).map(|device| (device, flag));
    if let Some(cached) = key.and_then(cached_support) {
        return cached;
    }
    let support = probe(directory, flag);
    if let Some(key) = key
        && matches!(support, FlagSupport::Supported | FlagSupport::Unsupported)
    {
        remember_support(key, support);
    }
    support
}

/// Whether a failed flagged rename failed because the filesystem cannot do the
/// flag at all.
///
/// The errno alone is not enough, because `EINVAL` is equally what misuse
/// returns. A caller that fell back on the errno alone would turn a future
/// bug of that kind into silently non-atomic writes, so the verdict comes from
/// the probe and the errno only decides whether asking is worthwhile.
pub fn flag_unavailable(directory: &Path, flag: RenameFlag, error: &io::Error) -> bool {
    is_unsupported_errno(error) && support(directory, flag) == FlagSupport::Unsupported
}

/// The errnos a filesystem uses to say it does not implement a flag.
///
/// `EOPNOTSUPP` and `ENOTSUP` are the same number on Linux, so naming both
/// here would be an unreachable arm rather than extra coverage.
pub fn is_unsupported_errno(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP)
    )
}

/// Perform a flagged rename between two names in the same directory.
pub fn rename_flagged_at(
    from_parent: &fs::File,
    from: &CString,
    to_parent: &fs::File,
    to: &CString,
    flag: RenameFlag,
) -> Result<(), io::Error> {
    #[cfg(test)]
    if directory_of(from_parent).is_some_and(|directory| forced_unsupported(&directory)) {
        return Err(rejected_flag());
    }
    // SAFETY: both descriptors and both C strings outlive this call.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            from_parent.as_raw_fd(),
            from.as_ptr(),
            to_parent.as_raw_fd(),
            to.as_ptr(),
            flag.bits(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Perform a flagged rename between two absolute paths.
pub fn rename_flagged_paths(
    from: &CString,
    to: &CString,
    flag: RenameFlag,
) -> Result<(), io::Error> {
    #[cfg(test)]
    if forced_unsupported(Path::new(std::ffi::OsStr::from_bytes(from.as_bytes()))) {
        return Err(rejected_flag());
    }
    // SAFETY: both C strings outlive this call. `AT_FDCWD` is ignored for the
    // absolute paths this is given.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            flag.bits(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn probe(directory: &Path, flag: RenameFlag) -> FlagSupport {
    let Ok(parent) = open_directory(directory) else {
        return FlagSupport::Undetermined;
    };
    let Ok(first) = create_scratch_file(&parent) else {
        return FlagSupport::Undetermined;
    };
    let support = match flag {
        // An exchange needs both names to exist.
        RenameFlag::Exchange => match create_scratch_file(&parent) {
            Ok(second) => {
                let support = classify(rename_flagged_at(&parent, &first, &parent, &second, flag));
                let _ = unlink_at(&parent, &second);
                support
            }
            Err(_) => FlagSupport::Undetermined,
        },
        // A no-replace rename needs the destination name to be free, so the
        // second name is generated and deliberately not created.
        RenameFlag::NoReplace => {
            let Ok(second) = scratch_name() else {
                let _ = unlink_at(&parent, &first);
                return FlagSupport::Undetermined;
            };
            let support = classify(rename_flagged_at(&parent, &first, &parent, &second, flag));
            // On success the scratch file now answers to the second name.
            let _ = unlink_at(&parent, &second);
            support
        }
    };
    let _ = unlink_at(&parent, &first);
    support
}

fn classify(result: Result<(), io::Error>) -> FlagSupport {
    match result {
        Ok(()) => FlagSupport::Supported,
        Err(error) if is_unsupported_errno(&error) => FlagSupport::Unsupported,
        Err(_) => FlagSupport::Undetermined,
    }
}

fn device_of(directory: &Path) -> Option<u64> {
    fs::metadata(directory).ok().map(|metadata| metadata.dev())
}

type SupportCache = Mutex<HashMap<(u64, RenameFlag), FlagSupport>>;

fn support_cache() -> &'static SupportCache {
    static CACHE: OnceLock<SupportCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_support(key: (u64, RenameFlag)) -> Option<FlagSupport> {
    support_cache()
        .lock()
        .expect("rename flag cache poisoned")
        .get(&key)
        .copied()
}

fn remember_support(key: (u64, RenameFlag), support: FlagSupport) {
    support_cache()
        .lock()
        .expect("rename flag cache poisoned")
        .insert(key, support);
}

fn open_directory(directory: &Path) -> Result<fs::File, io::Error> {
    let name = CString::new(directory.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory path has NUL"))?;
    // SAFETY: the C string outlives the call and the descriptor is adopted
    // exactly once below.
    let descriptor = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { fs::File::from_raw_fd(descriptor) })
    }
}

/// Probe scratch names are dot-prefixed, which is what the Vault already
/// excludes as noise, so a probe that is killed mid-flight leaves nothing a
/// Vault would index.
fn scratch_name() -> Result<CString, io::Error> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| io::Error::other("failed to generate probe filename entropy"))?;
    let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    CString::new(format!(".hatchdoor-rename-probe-{suffix}"))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "probe name has NUL"))
}

fn create_scratch_file(parent: &fs::File) -> Result<CString, io::Error> {
    let name = scratch_name()?;
    // SAFETY: the descriptor and C string outlive the call; the new descriptor
    // is closed immediately, since only the name matters here.
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    drop(unsafe { fs::File::from_raw_fd(descriptor) });
    Ok(name)
}

fn unlink_at(parent: &fs::File, name: &CString) -> Result<(), io::Error> {
    // SAFETY: the descriptor and C string outlive the call.
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(test))]
fn forced_unsupported(_directory: &Path) -> bool {
    false
}

/// Make every directory under `root` report its flags as unsupported, so a
/// test can exercise the fallback paths without a filesystem that rejects
/// them. Keyed by path rather than set process-wide, because the suite runs
/// its tests in parallel and each one owns its own temporary directory.
#[cfg(test)]
pub(crate) fn force_unsupported_for_tests(root: &Path) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    forced_roots()
        .lock()
        .expect("forced rename flag roots poisoned")
        .push(root);
}

#[cfg(test)]
fn forced_roots() -> &'static Mutex<Vec<std::path::PathBuf>> {
    static ROOTS: OnceLock<Mutex<Vec<std::path::PathBuf>>> = OnceLock::new();
    ROOTS.get_or_init(|| Mutex::new(Vec::new()))
}

#[cfg(test)]
fn forced_unsupported(directory: &Path) -> bool {
    let canonical = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf());
    forced_roots()
        .lock()
        .expect("forced rename flag roots poisoned")
        .iter()
        .any(|root| canonical.starts_with(root) || directory.starts_with(root))
}

/// What a filesystem that does not implement the flag reports. A forced test
/// directory answers with it so the fallback runs against the real errno
/// rather than a separate pretend path.
#[cfg(test)]
fn rejected_flag() -> io::Error {
    io::Error::from_raw_os_error(libc::EINVAL)
}

/// The path a directory descriptor currently names, for the test override
/// alone: the syscall wrapper takes descriptors, and the override is keyed by
/// path so tests running in parallel cannot disturb each other.
#[cfg(test)]
fn directory_of(parent: &fs::File) -> Option<std::path::PathBuf> {
    fs::read_link(format!("/proc/self/fd/{}", parent.as_raw_fd())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn an_ordinary_filesystem_supports_both_flags_and_the_probe_leaves_nothing_behind() {
        let directory = tempdir().expect("tempdir");
        assert_eq!(
            support(directory.path(), RenameFlag::Exchange),
            FlagSupport::Supported
        );
        assert_eq!(
            support(directory.path(), RenameFlag::NoReplace),
            FlagSupport::Supported
        );
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .expect("read probe directory")
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "the probe must clean up after itself, found {leftovers:?}"
        );
    }

    #[test]
    fn a_missing_directory_is_undetermined_rather_than_unsupported() {
        let directory = tempdir().expect("tempdir");
        let missing = directory.path().join("absent");
        assert_eq!(
            support(&missing, RenameFlag::Exchange),
            FlagSupport::Undetermined
        );
    }

    /// `EINVAL` is what a filesystem without flag support returns *and* what
    /// misuse of the syscall returns. Only the probe can tell them apart, and
    /// a caller that guessed from the errno alone would quietly turn a future
    /// bug into non-atomic writes (ADR-26).
    #[test]
    fn an_einval_on_a_filesystem_that_passes_the_probe_stays_an_error() {
        let directory = tempdir().expect("tempdir");
        let einval = io::Error::from_raw_os_error(libc::EINVAL);

        assert!(
            is_unsupported_errno(&einval),
            "EINVAL is one of the errnos worth asking about"
        );
        assert_eq!(
            support(directory.path(), RenameFlag::Exchange),
            FlagSupport::Supported
        );
        assert!(
            !flag_unavailable(directory.path(), RenameFlag::Exchange, &einval),
            "where the flag demonstrably works, EINVAL is a bug and must stay one"
        );
    }

    #[test]
    fn an_errno_that_is_not_about_flag_support_is_never_worth_asking_about() {
        for raw in [libc::ENOENT, libc::EACCES, libc::EXDEV, libc::EROFS] {
            let error = io::Error::from_raw_os_error(raw);
            assert!(
                !is_unsupported_errno(&error),
                "{error} says nothing about whether the flag exists"
            );
        }
    }

    #[test]
    fn the_test_override_reports_unsupported_without_touching_the_filesystem() {
        let directory = tempdir().expect("tempdir");
        force_unsupported_for_tests(directory.path());
        assert_eq!(
            support(directory.path(), RenameFlag::Exchange),
            FlagSupport::Unsupported
        );
        assert_eq!(
            support(&directory.path().join("nested"), RenameFlag::NoReplace),
            FlagSupport::Unsupported
        );
        let einval = io::Error::from_raw_os_error(libc::EINVAL);
        assert!(flag_unavailable(
            directory.path(),
            RenameFlag::Exchange,
            &einval
        ));
    }
}

pub mod commit_cooldown;
pub mod config;
pub mod managed_checkout;
pub mod managed_sync;
pub mod managed_task;
pub mod message;
pub mod note_history;
pub mod sync;

use crate::vault_registry::{VaultGitMode, VaultSource};

pub use commit_cooldown::{
    COMMIT_COOLDOWN_TICK_INTERVAL, CommitCooldown, spawn_commit_cooldown_tick,
};
pub use config::{GitConfig, GitMode};
pub use managed_checkout::{
    ManagedCheckout, ManagedCheckoutError, ManagedCheckoutLease, ManagedCheckoutRequest,
    ManagedHttpsCredentials,
};
pub use managed_sync::{
    ManagedSyncConfig, ManagedSyncError, ManagedSyncMode, ManagedSyncOutcome,
    synchronize_managed_checkout,
};
pub use managed_task::{
    DEFAULT_POLL_INTERVAL, DEFAULT_TICK_INTERVAL, GitPollingClock, ManagedGitOutcome,
    ManagedGitScheduler, ManagedGitTurnConfig, run_existing_git_commit_turn,
    run_existing_git_remote_turn, run_managed_git_commit_turn, run_managed_git_turn,
    spawn_scheduler_tick,
};
pub use message::{WriteLedger, WriteRecord, build_commit_message};
pub use note_history::{FirstAdd, FirstAdds, HistoryRead, NoteHistory};
pub use sync::{
    CommitOutcome, GitError, commit_local, has_uncommitted_changes, init_local_repo,
    run_local_history_git_turn, validate_local_repo, validate_repo,
};

/// How long libgit2 may wait to open a connection to a remote before the
/// operation fails.
#[cfg(not(test))]
const NETWORK_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// How long an open connection to a remote may go without moving a byte in
/// either direction before the operation fails. This is the stall watchdog:
/// it measures silence, not total duration, so a large but moving transfer
/// is never cut short, while a remote that stops answering mid-transfer
/// (a dropped VPN, a firewall that drops rather than rejects, a hung proxy)
/// releases the Vault's mutation lock and the one work lane within this
/// bound instead of holding both forever (#322).
///
/// Shutdown waits for an in-flight turn, so `stop_grace_period` in
/// `docker-compose.yml` must stay above this plus the connect bound.
#[cfg(not(test))]
const NETWORK_TRANSFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
// Test builds use short bounds so a stalled-remote test finishes quickly.
// Every other test talks to a local path remote, which opens no socket and
// never sees either value.
#[cfg(test)]
const NETWORK_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(test)]
const NETWORK_TRANSFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Give libgit2's sockets the bounds above, once per process, before any
/// fetch, push, or clone opens one.
///
/// The vendored libgit2 starts with both timeouts disabled, which means a
/// read on a connection the remote has stopped answering blocks forever. A
/// timeout surfaces as an ordinary libgit2 network error, which the callers
/// already classify as a retryable remote failure. Called from every call
/// site that opens a connection rather than from startup, so no entry point,
/// test or production, can reach the network with the bounds unset.
pub(crate) fn bound_network_waits() {
    static CONFIGURED: std::sync::Once = std::sync::Once::new();
    CONFIGURED.call_once(|| {
        let milliseconds = |duration: std::time::Duration| {
            libc::c_int::try_from(duration.as_millis()).unwrap_or(libc::c_int::MAX)
        };
        // SAFETY: these options are process-global integers that libgit2
        // reads when it opens a socket stream. `Once` makes this the only
        // write, and it happens before the caller opens its own connection.
        // Neither setter can fail; the `Result` only mirrors the options API.
        unsafe {
            let _ = git2::opts::set_server_connect_timeout_in_milliseconds(milliseconds(
                NETWORK_CONNECT_TIMEOUT,
            ));
            let _ = git2::opts::set_server_timeout_in_milliseconds(milliseconds(
                NETWORK_TRANSFER_TIMEOUT,
            ));
        }
    });
}

/// A remote that accepts connections and then never sends a byte: the
/// pathology [`bound_network_waits`] exists for. Returns an HTTPS URL for it.
#[cfg(test)]
pub(crate) fn stalled_https_remote() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stalled remote");
    let port = listener
        .local_addr()
        .expect("stalled remote address")
        .port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for connection in listener.incoming().flatten() {
            held.push(connection);
        }
    });
    format!("https://127.0.0.1:{port}/vault.git")
}

/// Whether this source makes local commits of its own, and so has a
/// `VaultWorkKind::Commit` turn (issue #267).
///
/// True for Local history, which is nothing but local commits, and for
/// Two-way, whose commit is the half of its sync that needs no remote. False
/// for Pull-only, which refuses writes and must leave a folder its operator
/// dirtied alone, and for a plain local folder, which has no Git at all.
///
/// Deliberately separate from `VaultSource::managed_git_poll_interval`, which
/// answers a different question, does this Vault poll a remote, and stays
/// exactly as it is. Both live here rather than on `VaultSource` itself
/// because the answer is a fact about Git behaviour, not about the registry
/// record, and this boundary is what owns Git behaviour.
pub fn source_commits(source: &VaultSource) -> bool {
    match source {
        VaultSource::Local { .. } => false,
        VaultSource::ExistingGit { mode, .. } | VaultSource::ManagedGit { mode, .. } => {
            matches!(mode, VaultGitMode::LocalHistory | VaultGitMode::TwoWay)
        }
    }
}

/// Whether this source has a remote to synchronize with, which is what
/// separates a Vault whose console offers **Sync now** from one that can only
/// be offered **Commit now**.
pub fn source_syncs_remote(source: &VaultSource) -> bool {
    match source {
        VaultSource::Local { .. } => false,
        VaultSource::ExistingGit { mode, .. } | VaultSource::ManagedGit { mode, .. } => {
            matches!(mode, VaultGitMode::PullOnly | VaultGitMode::TwoWay)
        }
    }
}

#[cfg(test)]
mod network_timeout_tests {
    #[test]
    fn network_waits_are_bounded_once_configured() {
        super::bound_network_waits();
        // SAFETY: reads of the process-global integers set above.
        let (connect, transfer) = unsafe {
            (
                git2::opts::get_server_connect_timeout_in_milliseconds().expect("connect"),
                git2::opts::get_server_timeout_in_milliseconds().expect("transfer"),
            )
        };
        assert_eq!(connect, 2_000);
        assert_eq!(transfer, 2_000);
    }
}

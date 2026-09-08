pub mod commit_cooldown;
pub mod config;
pub mod managed_checkout;
pub mod managed_sync;
pub mod managed_task;
pub mod message;
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
pub use sync::{
    CommitOutcome, GitError, commit_local, has_uncommitted_changes, init_local_repo,
    run_local_history_git_turn, validate_local_repo, validate_repo,
};

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

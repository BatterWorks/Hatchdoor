pub mod config;
pub mod managed_checkout;
pub mod managed_sync;
pub mod managed_task;
pub mod message;
pub mod sync;

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
    DEFAULT_POLL_INTERVAL, DEFAULT_TICK_INTERVAL, ManagedGitOutcome, ManagedGitScheduler,
    ManagedGitTurnConfig, run_existing_git_remote_turn, run_managed_git_turn, spawn_scheduler_tick,
};
pub use message::{WriteRecord, build_commit_message};
pub use sync::{
    CommitOutcome, GitError, commit_local, has_uncommitted_changes, init_local_repo,
    run_local_history_git_turn, validate_local_repo, validate_repo,
};

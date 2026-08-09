pub mod config;
pub mod managed_checkout;
pub mod managed_sync;
pub mod managed_task;
pub mod message;
pub mod status;
pub mod sync;
pub mod task;

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
    ManagedGitTurnConfig, run_managed_git_turn, spawn_scheduler_tick,
};
pub use message::{WriteRecord, build_commit_message};
pub use status::GitSyncStatus;
pub use sync::{
    CommitOutcome, GitError, SyncOutcome, SyncReport, commit_local, fetch_remote,
    has_uncommitted_changes, has_unpushed, init_local_repo, integrate_fetched, push_branch,
    run_local_history_git_turn, sync, unpushed_count, validate_local_repo, validate_repo,
};
pub use task::{GitSyncHandle, SyncOps, spawn_sync_task};

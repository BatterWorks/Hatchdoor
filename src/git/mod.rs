pub mod config;
pub mod message;
pub mod status;
pub mod sync;

pub use config::GitConfig;
pub use message::{WriteRecord, build_commit_message};
pub use status::GitSyncStatus;
pub use sync::{GitError, SyncOutcome, SyncReport, has_unpushed, sync, validate_repo};

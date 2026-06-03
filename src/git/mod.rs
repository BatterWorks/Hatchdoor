pub mod config;
pub mod sync;

pub use config::GitConfig;
pub use sync::{GitError, SyncOutcome, SyncReport, has_unpushed, sync, validate_repo};

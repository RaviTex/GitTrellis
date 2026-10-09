//! Application core: settings, logging, snapshots and background tasks.
//!
//! This layer sits between the UI and the Git service layer. It contains no
//! GTK widgets and no raw git invocations.

pub mod logging;
pub mod settings;
pub mod snapshot;
pub mod task;

pub use settings::{DiffViewMode, RecentRepo, Settings};
pub use snapshot::{load_snapshot, OperationResult, RepoSnapshot, SectionError, SnapshotOptions};

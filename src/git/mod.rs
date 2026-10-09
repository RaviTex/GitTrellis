//! Git service layer.
//!
//! This module is the *only* place in the application that talks to Git.
//! The UI depends on the structured types exported here and never executes
//! git itself.
//!
//! ```text
//! UI  ->  Core (controller)  ->  git::Repository  ->  git executable
//! ```

pub mod command;
pub mod error;
pub mod ops;
pub mod parse;
pub mod patch;
pub mod repo;
pub mod types;

pub use command::{Cancellation, Git, StreamKind};
pub use error::{GitError, GitResult};
pub use ops::{CommitOptions, ForceMode, ResetMode};
pub use parse::{parse_conflict_regions, resolve_conflicts};
pub use repo::{LogOptions, MergeOutcome, Repository};
pub use types::*;

/// Parse a version string like `git version 2.47.1` into a comparable tuple.
pub fn parse_git_version(version: &str) -> Option<(u32, u32, u32)> {
    let v = version.trim().strip_prefix("git version ")?;
    let mut parts = v.split(['.', ' ', '-']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    Some((major, minor, patch))
}

/// The minimum git version this application is tested against.
pub const MIN_GIT_VERSION: (u32, u32, u32) = (2, 23, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        assert_eq!(parse_git_version("git version 2.47.1"), Some((2, 47, 1)));
        assert_eq!(
            parse_git_version("git version 2.39.3 (Apple Git-145)"),
            Some((2, 39, 3))
        );
        assert_eq!(parse_git_version("nonsense"), None);
    }
}

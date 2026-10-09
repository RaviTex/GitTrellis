//! Repository snapshot: one consistent read of all repository metadata.
//!
//! Loading a snapshot never fails as a whole: individual sections that fail
//! (for example an unreadable stash list) are reported alongside the rest of
//! the data so the UI can show partial results and a precise error.

use std::path::PathBuf;

use crate::git::{
    Branch, ConfigEntry, GitError, Remote, RepoStatus, Repository, StashEntry, Tag,
};

#[derive(Debug, Clone, Default)]
pub struct RepoSnapshot {
    pub status: RepoStatus,
    pub branches: Vec<Branch>,
    pub tags: Vec<Tag>,
    pub remotes: Vec<Remote>,
    pub stashes: Vec<StashEntry>,
    pub config: Vec<ConfigEntry>,
}

#[derive(Debug, Clone, Copy)]
pub struct SnapshotOptions {
    pub branches: bool,
    pub tags: bool,
    pub remotes: bool,
    pub stashes: bool,
    pub config: bool,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            branches: true,
            tags: true,
            remotes: true,
            stashes: true,
            config: false,
        }
    }
}

impl SnapshotOptions {
    /// The minimum needed for the changes view; fast enough for every edit.
    pub fn status_only() -> Self {
        Self {
            branches: false,
            tags: false,
            remotes: false,
            stashes: false,
            config: false,
        }
    }
}

pub type SectionError = (&'static str, GitError);

pub fn load_snapshot(repo: &Repository, opts: SnapshotOptions) -> (RepoSnapshot, Vec<SectionError>) {
    let mut snapshot = RepoSnapshot::default();
    let mut errors: Vec<SectionError> = Vec::new();

    match repo.status() {
        Ok(s) => snapshot.status = s,
        Err(e) => errors.push(("status", e)),
    }
    if opts.branches {
        match repo.branches() {
            Ok(b) => snapshot.branches = b,
            Err(e) => errors.push(("branches", e)),
        }
    }
    if opts.tags {
        match repo.tags() {
            Ok(t) => snapshot.tags = t,
            Err(e) => errors.push(("tags", e)),
        }
    }
    if opts.remotes {
        match repo.remotes() {
            Ok(r) => snapshot.remotes = r,
            Err(e) => errors.push(("remotes", e)),
        }
    }
    if opts.stashes {
        match repo.stashes() {
            Ok(s) => snapshot.stashes = s,
            Err(e) => errors.push(("stashes", e)),
        }
    }
    if opts.config {
        match repo.config_list() {
            Ok(c) => snapshot.config = c,
            Err(e) => errors.push(("config", e)),
        }
    }
    (snapshot, errors)
}

/// Result of a long running repository operation.
#[derive(Debug, Clone)]
pub enum OperationResult {
    Completed(String),
    Conflicted(Vec<PathBuf>),
    Cancelled,
}

impl OperationResult {
    pub fn from_merge_outcome(
        outcome: crate::git::MergeOutcome,
        did: &str,
    ) -> OperationResult {
        use crate::git::MergeOutcome;
        match outcome {
            MergeOutcome::UpToDate => OperationResult::Completed(format!("{did}: already up to date")),
            MergeOutcome::FastForward => OperationResult::Completed(format!("{did}: fast-forward")),
            MergeOutcome::Merged => OperationResult::Completed(format!("{did}: done")),
            MergeOutcome::Conflicts(paths) => OperationResult::Conflicted(paths),
        }
    }
}

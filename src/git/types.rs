//! Structured domain types returned by the Git service layer.
//!
//! The UI only ever consumes these types; it never parses raw git output.

use std::path::PathBuf;

pub type Oid = String;

/// How a path changed between two trees (or between tree and worktree).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Unmodified,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Ignored,
    Unmerged,
}

impl ChangeKind {
    pub fn from_code(c: u8) -> ChangeKind {
        match c {
            b'M' => ChangeKind::Modified,
            b'A' => ChangeKind::Added,
            b'D' => ChangeKind::Deleted,
            b'R' => ChangeKind::Renamed,
            b'C' => ChangeKind::Copied,
            b'T' => ChangeKind::TypeChanged,
            b'U' => ChangeKind::Unmerged,
            b'?' => ChangeKind::Untracked,
            b'!' => ChangeKind::Ignored,
            _ => ChangeKind::Unmodified,
        }
    }

    pub fn short_code(&self) -> &'static str {
        match self {
            ChangeKind::Unmodified => "",
            ChangeKind::Added => "A",
            ChangeKind::Modified => "M",
            ChangeKind::Deleted => "D",
            ChangeKind::Renamed => "R",
            ChangeKind::Copied => "C",
            ChangeKind::TypeChanged => "T",
            ChangeKind::Untracked => "?",
            ChangeKind::Ignored => "!",
            ChangeKind::Unmerged => "U",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ChangeKind::Unmodified => "Unmodified",
            ChangeKind::Added => "Added",
            ChangeKind::Modified => "Modified",
            ChangeKind::Deleted => "Deleted",
            ChangeKind::Renamed => "Renamed",
            ChangeKind::Copied => "Copied",
            ChangeKind::TypeChanged => "Type changed",
            ChangeKind::Untracked => "Untracked",
            ChangeKind::Ignored => "Ignored",
            ChangeKind::Unmerged => "Conflicted",
        }
    }
}

/// The kind of unmerged (conflicted) entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    BothDeleted,
    AddedByUs,
    DeletedByThem,
    AddedByThem,
    DeletedByUs,
    BothAdded,
    BothModified,
}

impl ConflictKind {
    pub fn from_xy(x: u8, y: u8) -> Option<ConflictKind> {
        Some(match (x, y) {
            (b'D', b'D') => ConflictKind::BothDeleted,
            (b'A', b'U') => ConflictKind::AddedByUs,
            (b'U', b'D') => ConflictKind::DeletedByThem,
            (b'U', b'A') => ConflictKind::AddedByThem,
            (b'D', b'U') => ConflictKind::DeletedByUs,
            (b'A', b'A') => ConflictKind::BothAdded,
            (b'U', b'U') => ConflictKind::BothModified,
            _ => return None,
        })
    }

    pub fn description(&self) -> &'static str {
        match self {
            ConflictKind::BothDeleted => "Deleted by both sides",
            ConflictKind::AddedByUs => "Added by us, modified by them",
            ConflictKind::DeletedByThem => "Modified by us, deleted by them",
            ConflictKind::AddedByThem => "Modified by us, added by them",
            ConflictKind::DeletedByUs => "Added by them, deleted by us",
            ConflictKind::BothAdded => "Added by both sides",
            ConflictKind::BothModified => "Modified by both sides",
        }
    }
}

/// One entry of `git status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// Current path (worktree path).
    pub path: PathBuf,
    /// Original path for renames/copies.
    pub orig_path: Option<PathBuf>,
    /// Index (staged) change.
    pub index: ChangeKind,
    /// Worktree (unstaged) change.
    pub worktree: ChangeKind,
    /// Conflict classification, when unmerged.
    pub conflict: Option<ConflictKind>,
    /// Submodule state field from porcelain v2 (raw, for advanced display).
    pub submodule: String,
}

impl StatusEntry {
    pub fn is_staged(&self) -> bool {
        !matches!(
            self.index,
            ChangeKind::Unmodified | ChangeKind::Untracked | ChangeKind::Ignored | ChangeKind::Unmerged
        )
    }

    pub fn is_unstaged(&self) -> bool {
        !matches!(
            self.worktree,
            ChangeKind::Unmodified | ChangeKind::Untracked | ChangeKind::Ignored | ChangeKind::Unmerged
        )
    }

    pub fn is_conflicted(&self) -> bool {
        self.conflict.is_some()
    }

    pub fn is_untracked(&self) -> bool {
        self.index == ChangeKind::Untracked || self.worktree == ChangeKind::Untracked
    }

    /// Kind used for display in the "Changes" (unstaged) list.
    pub fn display_unstaged_kind(&self) -> ChangeKind {
        if self.worktree != ChangeKind::Unmodified {
            self.worktree
        } else {
            self.index
        }
    }
}

/// Which operation (if any) is currently in progress in a repository.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OperationState {
    #[default]
    None,
    Merge {
        message: Option<String>,
    },
    Rebase {
        head_name: Option<String>,
        onto: Option<String>,
        done: Option<u32>,
        total: Option<u32>,
        interactive: bool,
    },
    CherryPick {
        sha: Option<String>,
    },
    Revert {
        sha: Option<String>,
    },
    Bisect,
}

impl OperationState {
    pub fn is_none(&self) -> bool {
        matches!(self, OperationState::None)
    }

    pub fn label(&self) -> &'static str {
        match self {
            OperationState::None => "",
            OperationState::Merge { .. } => "Merging",
            OperationState::Rebase { .. } => "Rebasing",
            OperationState::CherryPick { .. } => "Cherry-picking",
            OperationState::Revert { .. } => "Reverting",
            OperationState::Bisect => "Bisecting",
        }
    }

    pub fn can_continue(&self) -> bool {
        !self.is_none()
    }

    /// The git command used to continue the operation.
    pub fn continue_command(&self) -> Option<&'static str> {
        match self {
            OperationState::None => None,
            OperationState::Merge { .. } => Some("merge"),
            OperationState::Rebase { .. } => Some("rebase"),
            OperationState::CherryPick { .. } => Some("cherry-pick"),
            OperationState::Revert { .. } => Some("revert"),
            OperationState::Bisect => Some("bisect"),
        }
    }

    pub fn abort_command(&self) -> Option<&'static str> {
        match self {
            OperationState::None => None,
            OperationState::Merge { .. } => Some("merge"),
            OperationState::Rebase { .. } => Some("rebase"),
            OperationState::CherryPick { .. } => Some("cherry-pick"),
            OperationState::Revert { .. } => Some("revert"),
            OperationState::Bisect => Some("bisect"),
        }
    }
}

/// Full repository status.
#[derive(Debug, Clone, Default)]
pub struct RepoStatus {
    pub entries: Vec<StatusEntry>,
    /// OID of HEAD, or all zeros when unborn.
    pub head_oid: Option<Oid>,
    /// Branch name, or None when detached.
    pub head_name: Option<String>,
    pub detached: bool,
    pub unborn: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub operation: OperationState,
}

impl RepoStatus {
    pub fn staged(&self) -> Vec<&StatusEntry> {
        self.entries.iter().filter(|e| e.is_staged()).collect()
    }

    pub fn unstaged(&self) -> Vec<&StatusEntry> {
        self.entries
            .iter()
            .filter(|e| e.is_unstaged() && !e.is_conflicted())
            .collect()
    }

    pub fn untracked(&self) -> Vec<&StatusEntry> {
        self.entries.iter().filter(|e| e.is_untracked()).collect()
    }

    pub fn conflicted(&self) -> Vec<&StatusEntry> {
        self.entries.iter().filter(|e| e.is_conflicted()).collect()
    }

    pub fn has_conflicts(&self) -> bool {
        self.entries.iter().any(|e| e.is_conflicted())
    }

    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn staged_count(&self) -> usize {
        self.staged().len()
    }
}

/// Reference label attached to a commit (branch, tag, HEAD...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Head,
    Branch,
    RemoteBranch,
    Tag,
}

/// A commit as shown in history and commit detail views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub id: Oid,
    pub short_id: String,
    pub parents: Vec<Oid>,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub commit_time: i64,
    pub summary: String,
    pub body: String,
    pub refs: Vec<RefLabel>,
}

impl Commit {
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }

    pub fn head_ref(&self) -> Option<&RefLabel> {
        self.refs.iter().find(|r| r.kind == RefKind::Head)
    }

    pub fn first_parent(&self) -> Option<&Oid> {
        self.parents.first()
    }
}

/// A branch (local or remote-tracking).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub full_ref: String,
    pub is_remote: bool,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub gone: bool,
    pub tip: Oid,
    pub tip_summary: String,
    pub tip_author: String,
    pub tip_time: i64,
    /// Remote name for remote-tracking branches (`origin/main` -> `origin`).
    pub remote: Option<String>,
}

/// A tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub oid: Oid,
    /// Commit the tag points at (peeled for annotated tags).
    pub target: Oid,
    pub is_annotated: bool,
    pub tagger: Option<String>,
    pub time: Option<i64>,
    pub subject: String,
}

/// A configured remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
}

/// A stash entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    pub index: usize,
    pub oid: Oid,
    pub message: String,
    pub branch: Option<String>,
    pub time: Option<i64>,
    pub has_untracked: bool,
}

/// Kind of a single diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    NoNewline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub section: String,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    pub fn header(&self) -> String {
        format!(
            "@@ -{},{} +{},{} @@{}",
            self.old_start,
            self.old_count,
            self.new_start,
            self.new_count,
            if self.section.is_empty() {
                String::new()
            } else {
                format!(" {}", self.section)
            }
        )
    }
}

/// Diff of a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub status: ChangeKind,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub is_binary: bool,
    pub similarity: Option<u32>,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    pub fn display_path(&self) -> String {
        match (&self.old_path, &self.new_path) {
            (Some(o), Some(n)) if o != n => {
                format!("{} → {}", o.display(), n.display())
            }
            (_, Some(n)) => n.display().to_string(),
            (Some(o), None) => o.display().to_string(),
            (None, None) => "<unknown>".to_string(),
        }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.new_path.as_ref().or(self.old_path.as_ref())
    }

    pub fn added_lines(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == LineKind::Added)
            .count()
    }

    pub fn removed_lines(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == LineKind::Removed)
            .count()
    }
}

/// A file changed between two revisions, as reported by `--name-status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub status: ChangeKind,
    pub path: PathBuf,
    pub orig_path: Option<PathBuf>,
}

/// A commit signature/verification result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInfo {
    pub status: String,
    pub signer: Option<String>,
    pub key: Option<String>,
}

/// One entry in `git config --list --show-origin --show-scope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    pub scope: ConfigScope,
    pub origin: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    System,
    Global,
    Local,
    Worktree,
    Command,
    Unknown,
}

impl ConfigScope {
    pub fn from_str(s: &str) -> ConfigScope {
        match s {
            "system" => ConfigScope::System,
            "global" => ConfigScope::Global,
            "local" => ConfigScope::Local,
            "worktree" => ConfigScope::Worktree,
            "command" => ConfigScope::Command,
            _ => ConfigScope::Unknown,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ConfigScope::System => "System",
            ConfigScope::Global => "Global",
            ConfigScope::Local => "Repository",
            ConfigScope::Worktree => "Worktree",
            ConfigScope::Command => "Command line",
            ConfigScope::Unknown => "Other",
        }
    }
}

/// A parsed conflict region inside a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRegion {
    /// 0-based line index where the region starts (the `<<<<<<<` marker).
    pub start_line: usize,
    /// 0-based line index just past the region (after `>>>>>>>`).
    pub end_line: usize,
    pub ours_label: String,
    pub theirs_label: String,
    pub base_label: Option<String>,
    pub ours: Vec<String>,
    pub theirs: Vec<String>,
    pub base: Vec<String>,
}

/// Which side of a conflict to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    Ours,
    Theirs,
    Both,
    BothTheirsFirst,
}

/// Build a synthetic "added file" diff from on-disk content. Used to show
/// untracked files in the diff viewer before they are staged.
pub fn synthetic_added_diff(path: &std::path::Path, content: &str) -> FileDiff {
    const MAX: usize = 20_000;
    let mut lines: Vec<DiffLine> = Vec::new();
    for line in content.lines().take(MAX) {
        lines.push(DiffLine {
            kind: LineKind::Added,
            text: line.to_string(),
        });
    }
    let count = lines.len() as u32;
    let hunks = if lines.is_empty() {
        Vec::new()
    } else {
        vec![Hunk {
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: count,
            section: String::new(),
            lines,
        }]
    };
    FileDiff {
        old_path: None,
        new_path: Some(path.to_path_buf()),
        status: ChangeKind::Added,
        old_mode: None,
        new_mode: Some("100644".into()),
        is_binary: false,
        similarity: None,
        hunks,
    }
}

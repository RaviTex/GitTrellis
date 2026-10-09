//! `Repository` — the entry point of the Git service layer.
//!
//! Every git invocation made by the application ultimately goes through this
//! type. Read operations return structured domain types; write operations are
//! small and explicit so that the UI layer can control confirmation flows.

use std::path::{Path, PathBuf};

use super::command::Git;
use super::error::{GitError, GitResult};
use super::parse;
use super::types::*;

/// Options for walking the commit history.
#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    pub all_refs: bool,
    pub limit: usize,
    pub skip: usize,
    pub rev: Option<String>,
    pub paths: Vec<PathBuf>,
    pub grep: Option<String>,
    pub author: Option<String>,
    pub follow: bool,
}

impl LogOptions {
    pub fn all() -> Self {
        LogOptions {
            all_refs: true,
            limit: 2000,
            ..Default::default()
        }
    }
}

/// Outcome of an operation that can stop on conflicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    UpToDate,
    FastForward,
    Merged,
    Conflicts(Vec<PathBuf>),
}

/// An opened Git repository.
#[derive(Clone)]
pub struct Repository {
    base_dir: PathBuf,
    git_dir: PathBuf,
    git: Git,
}

impl std::fmt::Debug for Repository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repository")
            .field("workdir", &self.base_dir)
            .field("git_dir", &self.git_dir)
            .finish()
    }
}

impl Repository {
    /// Open a repository, resolving the work tree and git directory.
    pub fn open(path: impl AsRef<Path>) -> GitResult<Self> {
        Self::open_with_git(path, Git::system())
    }

    /// Open a repository using a specific `Git` handle (used by tests to
    /// isolate from the developer's configuration).
    pub fn open_with_git(path: impl AsRef<Path>, git: Git) -> GitResult<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(GitError::NotARepository {
                path: path.to_path_buf(),
            });
        }
        let workdir = git
            .command()
            .cwd(path)
            .args(["rev-parse", "--show-toplevel"])
            .run()
            .map_err(|_| GitError::NotARepository {
                path: path.to_path_buf(),
            })?;
        let workdir = workdir.stdout_lossy().trim().to_string();
        if workdir.is_empty() {
            return Err(GitError::InvalidState {
                message: "Bare repositories are not supported by this application."
                    .into(),
                recovery: Some("Open a repository with a working tree instead.".into()),
            });
        }
        let git_dir = git
            .command()
            .cwd(path)
            .args(["rev-parse", "--absolute-git-dir"])
            .run_string()?;
        let base_dir = PathBuf::from(&workdir);
        let git_dir = PathBuf::from(git_dir.trim());
        Ok(Self {
            base_dir,
            git_dir,
            git,
        })
    }

    /// Diff a revision against the working tree (not HEAD).
    pub fn diff_vs_worktree(&self, rev: &str) -> GitResult<Vec<FileDiff>> {
        let out = self
            .command()
            .args([
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--find-renames",
                "--no-textconv",
                "-U3",
                rev,
                "--",
            ])
            .run_bytes()?;
        parse::parse_diff(&out)
    }

    /// Changed files between a revision and the working tree.
    pub fn changed_files_vs_worktree(&self, rev: &str) -> GitResult<Vec<ChangedFile>> {
        let out = self
            .command()
            .args([
                "diff",
                "--name-status",
                "-z",
                "--find-renames",
                "--no-ext-diff",
                rev,
                "--",
            ])
            .run_bytes()?;
        Ok(parse::parse_name_status(&out))
    }

    /// Initialize a new repository in `path`.
    pub fn init(path: impl AsRef<Path>, initial_branch: Option<&str>) -> GitResult<Self> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)?;
        let git = Git::system();
        let mut cmd = git.command().cwd(path).arg("init").arg("-q");
        if let Some(b) = initial_branch {
            let b = b.trim();
            if b.is_empty() {
                return Err(GitError::InvalidState {
                    message: "Initial branch name must not be empty.".into(),
                    recovery: None,
                });
            }
            cmd = cmd.args(["-b", b]);
        }
        cmd.arg("--").arg(path).run()?;
        Self::open(path)
    }

    /// Clone `url` into `dest`.
    pub fn clone(
        url: &str,
        dest: &Path,
        branch: Option<&str>,
        depth: Option<u32>,
        cancel: Option<super::command::Cancellation>,
        on_progress: impl FnMut(super::command::StreamKind, &str) + Send + 'static,
    ) -> GitResult<Self> {
        let git = Git::system();
        if dest.exists() {
            let empty = std::fs::read_dir(dest)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false);
            if !empty {
                return Err(GitError::InvalidState {
                    message: format!(
                        "The destination directory {} already exists and is not empty.",
                        dest.display()
                    ),
                    recovery: Some("Choose a different destination directory.".into()),
                });
            }
        } else if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut cmd = git.command().arg("clone").arg("--progress");
        if let Some(b) = branch {
            cmd = cmd.arg("--branch").arg(b);
        }
        if let Some(d) = depth {
            cmd = cmd.arg("--depth").arg(d.to_string());
        }
        let mut cmd = cmd.arg("--").arg(url).arg(dest);
        if let Some(c) = cancel {
            cmd = cmd.cancel(c);
        }
        cmd.run_streaming(on_progress)?;
        Self::open(dest)
    }

    pub fn workdir(&self) -> &Path {
        &self.base_dir
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn git(&self) -> &Git {
        &self.git
    }

    pub fn display_name(&self) -> String {
        self.base_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.base_dir.display().to_string())
    }

    pub fn command(&self) -> super::command::CommandBuilder {
        self.git.command().cwd(&self.base_dir)
    }

    // -----------------------------------------------------------------
    // Read operations
    // -----------------------------------------------------------------

    pub fn status(&self) -> GitResult<RepoStatus> {
        let out = self
            .command()
            .args([
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
            ])
            .run_bytes()?;
        let mut status = parse::parse_status_v2(&out)?;
        status.operation = self.operation_state();
        Ok(status)
    }

    pub fn operation_state(&self) -> OperationState {
        let gd = &self.git_dir;
        if gd.join("MERGE_HEAD").exists() {
            let message = std::fs::read_to_string(gd.join("MERGE_MSG"))
                .ok()
                .and_then(|s| s.lines().next().map(str::to_string));
            return OperationState::Merge { message };
        }
        if gd.join("rebase-merge").exists() || gd.join("rebase-apply").exists() {
            let dir = if gd.join("rebase-merge").exists() {
                gd.join("rebase-merge")
            } else {
                gd.join("rebase-apply")
            };
            let read = |f: &str| {
                std::fs::read_to_string(dir.join(f))
                    .ok()
                    .map(|s| s.trim().to_string())
            };
            let head_name = read("head-name").map(|s| {
                s.strip_prefix("refs/heads/").unwrap_or(&s).to_string()
            });
            let onto = read("onto").map(|s| s[..s.len().min(9)].to_string());
            let done = read("msgnum").and_then(|s| s.parse().ok());
            let total = read("end").and_then(|s| s.parse().ok());
            return OperationState::Rebase {
                head_name,
                onto,
                done,
                total,
                interactive: gd.join("rebase-merge").exists(),
            };
        }
        if gd.join("CHERRY_PICK_HEAD").exists() {
            let sha = std::fs::read_to_string(gd.join("CHERRY_PICK_HEAD"))
                .ok()
                .map(|s| s.trim().chars().take(9).collect());
            return OperationState::CherryPick { sha };
        }
        if gd.join("REVERT_HEAD").exists() {
            let sha = std::fs::read_to_string(gd.join("REVERT_HEAD"))
                .ok()
                .map(|s| s.trim().chars().take(9).collect());
            return OperationState::Revert { sha };
        }
        if gd.join("BISECT_LOG").exists() {
            return OperationState::Bisect;
        }
        OperationState::None
    }

    pub fn log(&self, opts: &LogOptions) -> GitResult<Vec<Commit>> {
        let mut args: Vec<std::ffi::OsString> = vec![
            "log".into(),
            "--no-color".into(),
            "--topo-order".into(),
            "-z".into(),
            format!("--format={}", parse::LOG_FORMAT).into(),
        ];
        if opts.all_refs {
            args.push("--all".into());
        }
        if let Some(rev) = &opts.rev {
            args.push(rev.clone().into());
        }
        if opts.limit > 0 {
            args.push(format!("-n{}", opts.limit).into());
        }
        if opts.skip > 0 {
            args.push(format!("--skip={}", opts.skip).into());
        }
        if let Some(grep) = &opts.grep {
            args.push("--grep".into());
            args.push(grep.clone().into());
            args.push("--regexp-ignore-case".into());
        }
        if let Some(author) = &opts.author {
            args.push(format!("--author={author}").into());
        }
        if !opts.paths.is_empty() {
            args.push("--".into());
            if opts.follow {
                // --follow only supports a single path
                args.push(opts.paths[0].clone().into());
            } else {
                for p in &opts.paths {
                    args.push(p.clone().into());
                }
            }
        } else {
            args.push("--".into());
        }
        let out = self.command().args(args).run_bytes()?;
        parse::parse_log(&out)
    }

    pub fn log_commit(&self, rev: &str) -> GitResult<Commit> {
        if rev.starts_with('-') {
            return Err(GitError::InvalidState {
                message: "Invalid revision.".into(),
                recovery: None,
            });
        }
        let out = self
            .command()
            .args([
                "log",
                "--no-color",
                "-z",
                "-n1",
                &format!("--format={}", parse::LOG_FORMAT),
                rev,
                "--",
            ])
            .run_bytes()?;
        parse::parse_log(&out)?
            .pop()
            .ok_or_else(|| GitError::Parse {
                context: "commit lookup",
                details: format!("no commit found for {rev}"),
            })
    }

    pub fn branches(&self) -> GitResult<Vec<Branch>> {
        let out = self
            .command()
            .args([
                "for-each-ref",
                &format!("--format={}", parse::BRANCH_FORMAT),
                "--sort=-committerdate",
                "refs/heads",
                "refs/remotes",
            ])
            .run_string()?;
        Ok(parse::parse_branches(&out))
    }

    pub fn tags(&self) -> GitResult<Vec<Tag>> {
        let out = self
            .command()
            .args([
                "for-each-ref",
                &format!("--format={}", parse::TAG_FORMAT),
                "--sort=-taggerdate",
                "refs/tags",
            ])
            .run_string()?;
        Ok(parse::parse_tags(&out))
    }

    pub fn remotes(&self) -> GitResult<Vec<Remote>> {
        let out = self
            .command()
            .args(["remote", "-v"])
            .run_string()?;
        Ok(parse::parse_remotes(&out))
    }

    pub fn stashes(&self) -> GitResult<Vec<StashEntry>> {
        let out = self
            .command()
            .args([
                "stash",
                "list",
                &format!("--format={}", parse::STASH_FORMAT),
            ])
            .run_string()?;
        Ok(parse::parse_stash_list(&out))
    }

    pub fn config_list(&self) -> GitResult<Vec<ConfigEntry>> {
        let out = self
            .command()
            .args([
                "config",
                "--list",
                "--show-origin",
                "--show-scope",
                "-z",
            ])
            .run_bytes()?;
        Ok(parse::parse_config_list(&out))
    }

    pub fn config_get(&self, key: &str) -> GitResult<Option<String>> {
        let out = self
            .command()
            .args(["config", "--get", key])
            .run_unchecked()?;
        if out.code == Some(0) {
            Ok(Some(out.stdout_lossy().trim_end().to_string()))
        } else {
            Ok(None)
        }
    }

    pub fn config_get_global(&self, key: &str) -> GitResult<Option<String>> {
        self.config_get_scoped(key, ConfigScope::Global)
    }

    /// Read a config value from one specific scope.
    pub fn config_get_scoped(&self, key: &str, scope: ConfigScope) -> GitResult<Option<String>> {
        let out = self
            .command()
            .args(["config", scope_flag(scope), "--get", key])
            .run_unchecked()?;
        if out.code == Some(0) {
            Ok(Some(out.stdout_lossy().trim_end().to_string()))
        } else {
            Ok(None)
        }
    }

    /// Set a config value in the requested scope.
    pub fn config_set(&self, key: &str, value: &str, scope: ConfigScope) -> GitResult<()> {
        validate_config_key(key)?;
        self.command()
            .args(["config", scope_flag(scope), "--"])
            .arg(key)
            .arg(value)
            .run()?;
        Ok(())
    }

    pub fn config_unset(&self, key: &str, scope: ConfigScope) -> GitResult<()> {
        validate_config_key(key)?;
        let out = self
            .command()
            .args(["config", scope_flag(scope), "--unset", key])
            .run()?;
        let _ = out;
        Ok(())
    }

    pub fn rev_parse(&self, spec: &str) -> GitResult<String> {
        let out = self
            .command()
            .args(["rev-parse", "--verify", "--quiet", spec])
            .run_unchecked()?;
        if out.code == Some(0) {
            Ok(out.stdout_lossy().trim().to_string())
        } else {
            Err(GitError::InvalidState {
                message: format!("Revision `{spec}` could not be resolved."),
                recovery: Some("Refresh the repository view.".into()),
            })
        }
    }

    pub fn merge_base(&self, a: &str, b: &str) -> GitResult<Option<String>> {
        let out = self
            .command()
            .args(["merge-base", a, b])
            .run_unchecked()?;
        if out.code == Some(0) {
            Ok(Some(out.stdout_lossy().trim().to_string()))
        } else {
            Ok(None)
        }
    }

    /// Number of commits reachable from `rev` (used for progress estimation).
    pub fn count_commits(&self, rev: &str) -> GitResult<u32> {
        let out = self
            .command()
            .args(["rev-list", "--count", rev, "--"])
            .run_string()?;
        Ok(out.trim().parse().unwrap_or(0))
    }

    // -----------------------------------------------------------------
    // Diff operations
    // -----------------------------------------------------------------

    pub fn diff_worktree(&self, path: Option<&Path>) -> GitResult<Vec<FileDiff>> {
        self.run_diff(&["diff", "--no-ext-diff", "--find-renames"], path)
    }

    pub fn diff_index(&self, path: Option<&Path>) -> GitResult<Vec<FileDiff>> {
        self.run_diff(
            &["diff", "--cached", "--no-ext-diff", "--find-renames"],
            path,
        )
    }

    /// Diff of `to` relative to `from`.
    pub fn diff_commits(&self, from: &str, to: &str) -> GitResult<Vec<FileDiff>> {
        let out = self
            .command()
            .args([
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--find-renames",
                "--no-textconv",
                "-U3",
                from,
                to,
                "--",
            ])
            .run_bytes()?;
        parse::parse_diff(&out)
    }

    /// Diff introduced by a single commit (against its first parent).
    pub fn diff_commit(&self, oid: &str) -> GitResult<Vec<FileDiff>> {
        let out = self
            .command()
            .args([
                "show",
                "--no-ext-diff",
                "--no-color",
                "--find-renames",
                "--no-textconv",
                "-U3",
                "--format=",
                oid,
                "--",
            ])
            .run_bytes()?;
        parse::parse_diff(&out)
    }

    pub fn diff_stash(&self, index: usize) -> GitResult<Vec<FileDiff>> {
        let out = self
            .command()
            .args([
                "stash",
                "show",
                "--no-ext-diff",
                "--no-color",
                "--find-renames",
                "--no-textconv",
                "-U3",
                "-p",
                &format!("stash@{{{index}}}"),
            ])
            .run_bytes()?;
        parse::parse_diff(&out)
    }

    fn run_diff(&self, base_args: &[&str], path: Option<&Path>) -> GitResult<Vec<FileDiff>> {
        let mut args: Vec<std::ffi::OsString> = base_args
            .iter()
            .map(|s| std::ffi::OsString::from(*s))
            .collect();
        args.push("--no-color".into());
        args.push("--no-textconv".into());
        args.push("-U3".into());
        args.push("--".into());
        if let Some(p) = path {
            args.push(p.as_os_str().to_os_string());
        }
        let out = self.command().args(args).run_bytes()?;
        parse::parse_diff(&out)
    }

    pub fn changed_files_between(&self, from: &str, to: &str) -> GitResult<Vec<ChangedFile>> {
        let out = self
            .command()
            .args([
                "diff",
                "--name-status",
                "-z",
                "--find-renames",
                "--no-ext-diff",
                from,
                to,
                "--",
            ])
            .run_bytes()?;
        Ok(parse::parse_name_status(&out))
    }

    pub fn changed_files_commit(&self, oid: &str) -> GitResult<Vec<ChangedFile>> {
        let parents = self
            .command()
            .args(["rev-list", "--parents", "-n1", oid])
            .run_string()?;
        let mut it = parents.split_whitespace();
        let _self_id = it.next();
        let from = match it.next() {
            Some(parent) => parent.to_string(),
            None => {
                // root commit: everything is added
                String::new()
            }
        };
        let out = if from.is_empty() {
            self.command()
                .args([
                    "diff-tree",
                    "--root",
                    "--no-commit-id",
                    "--name-status",
                    "-z",
                    "-r",
                    oid,
                    "--",
                ])
                .run_bytes()?
        } else {
            self.command()
                .args([
                    "diff",
                    "--name-status",
                    "-z",
                    "--find-renames",
                    "--no-ext-diff",
                    &from,
                    oid,
                    "--",
                ])
                .run_bytes()?
        };
        Ok(parse::parse_name_status(&out))
    }

    /// Read the content of a file at a given revision. Returns raw bytes.
    pub fn show_file(&self, rev: &str, path: &Path) -> GitResult<Vec<u8>> {
        let spec = format!("{}:{}", rev, path.display());
        let out = self.command().args(["show", &spec]).run()?;
        Ok(out.stdout)
    }

    /// Read a merge stage version of a file (`:1:path`, `:2:path`, `:3:path`).
    pub fn show_stage(&self, stage: u8, path: &Path) -> GitResult<Option<Vec<u8>>> {
        let spec = format!(":{}:{}", stage, path.display());
        let out = self.command().args(["show", &spec]).run_unchecked()?;
        if out.code == Some(0) {
            Ok(Some(out.stdout))
        } else {
            Ok(None)
        }
    }

    /// List conflicted paths (`git ls-files -u`).
    pub fn conflicted_paths(&self) -> GitResult<Vec<PathBuf>> {
        let out = self
            .command()
            .args(["ls-files", "-u", "-z"])
            .run_bytes()?;
        let mut paths: Vec<PathBuf> = Vec::new();
        for rec in out.split(|&b| b == 0) {
            if rec.is_empty() {
                continue;
            }
            // format: <mode> <sha> <stage>\t<path>
            if let Some(tab) = rec.iter().position(|&b| b == b'\t') {
                let p = parse::path_from_bytes(&rec[tab + 1..]);
                if !paths.contains(&p) {
                    paths.push(p);
                }
            }
        }
        Ok(paths)
    }

    /// Read a worktree file as text (lossy). Returns None when missing.
    pub fn read_worktree_file(&self, path: &Path) -> GitResult<Option<String>> {
        let full = self.base_dir.join(path);
        match std::fs::read(&full) {
            Ok(bytes) => {
                if bytes.contains(&0) {
                    return Ok(None);
                }
                Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Write a worktree file (used by the conflict resolver). Refuses paths
    /// that escape the repository.
    pub fn write_worktree_file(&self, path: &Path, content: &str) -> GitResult<()> {
        let full = self.resolve_in_workdir(path)?;
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full, content)?;
        Ok(())
    }

    /// Resolve a repository-relative path, guaranteeing it stays inside the
    /// working tree. Rejects absolute paths and `..` escapes.
    pub fn resolve_in_workdir(&self, path: &Path) -> GitResult<PathBuf> {
        if path.is_absolute() {
            return Err(GitError::InvalidState {
                message: format!("Refusing absolute path: {}", path.display()),
                recovery: None,
            });
        }
        let joined = self.base_dir.join(path);
        let normalized = normalize_path(&joined);
        if !normalized.starts_with(&self.base_dir) {
            return Err(GitError::InvalidState {
                message: format!("Refusing path outside repository: {}", path.display()),
                recovery: None,
            });
        }
        Ok(normalized)
    }

    /// True when the working tree has no changes and no untracked files.
    pub fn is_clean(&self) -> GitResult<bool> {
        let out = self
            .command()
            .args(["status", "--porcelain"])
            .run_string()?;
        Ok(out.trim().is_empty())
    }

    pub fn head_summary(&self) -> GitResult<String> {
        let out = self
            .command()
            .args(["log", "-1", "--format=%h %s"])
            .run_unchecked()?;
        if out.code == Some(0) {
            Ok(out.stdout_lossy().trim().to_string())
        } else {
            Ok(String::new())
        }
    }
}

fn validate_config_key(key: &str) -> GitResult<()> {
    let valid = !key.is_empty()
        && key.len() < 256
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(GitError::InvalidState {
            message: format!("`{key}` is not a valid configuration key."),
            recovery: None,
        })
    }
}

pub fn scope_flag(scope: ConfigScope) -> &'static str {
    match scope {
        ConfigScope::System => "--system",
        ConfigScope::Global => "--global",
        ConfigScope::Local | ConfigScope::Worktree => "--local",
        ConfigScope::Command | ConfigScope::Unknown => "--local",
    }
}

/// Lexically normalize a path (remove `.` and `..` segments).
pub fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

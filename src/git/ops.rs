//! High level write operations on a repository.
//!
//! These functions are deliberately explicit and small: each one maps to a
//! single git command (or a short, well defined sequence) so that the UI layer
//! can offer precise confirmations and precise error messages.

use std::path::{Path, PathBuf};

use super::command::{Cancellation, StreamKind};
use super::error::{GitError, GitResult};
use super::repo::{normalize_path, MergeOutcome, Repository};

/// How to push when the remote branch has diverged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceMode {
    /// Plain push; fail if non-fast-forward.
    None,
    /// `--force-with-lease`: refuses to overwrite unseen work.
    WithLease,
    /// `--force`: overwrites remote unconditionally.
    Force,
}

#[derive(Debug, Clone, Default)]
pub struct CommitOptions {
    pub message: String,
    pub amend: bool,
    pub sign_off: bool,
    /// `Some(None)` = sign with default key, `Some(Some(key))` = explicit key.
    pub gpg_sign: Option<Option<String>>,
    pub allow_empty: bool,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
    Keep,
}

impl ResetMode {
    fn flag(&self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
            ResetMode::Keep => "--keep",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ResetMode::Soft => "Soft — keep changes staged",
            ResetMode::Mixed => "Mixed — keep changes in working tree",
            ResetMode::Hard => "Hard — discard all changes (destructive)",
            ResetMode::Keep => "Keep — keep uncommitted changes without staging",
        }
    }
}

/// A summary of a repository used by the UI before opening it.
#[derive(Debug, Clone)]
pub struct RepoSummary {
    pub path: PathBuf,
    pub name: String,
    pub head: String,
}

impl Repository {
    // -----------------------------------------------------------------
    // Staging
    // -----------------------------------------------------------------

    fn pathspec_stdin(paths: &[PathBuf]) -> GitResult<Vec<u8>> {
        if paths.is_empty() {
            return Err(GitError::InvalidState {
                message: "No files were selected.".into(),
                recovery: None,
            });
        }
        let mut data = Vec::new();
        for p in paths {
            let s = p.to_string_lossy();
            if s.contains('\n') {
                // NUL terminated pathspec files cannot represent newlines in
                // filenames; git itself rejects them in most commands. Report
                // instead of silently doing the wrong thing.
                return Err(GitError::InvalidState {
                    message: format!("File names containing newlines are not supported: {s}"),
                    recovery: None,
                });
            }
            data.extend_from_slice(s.as_bytes());
            data.push(0);
        }
        Ok(data)
    }

    /// Stage the given paths (worktree paths relative to the repository root).
    pub fn add_paths(&self, paths: &[PathBuf]) -> GitResult<()> {
        let data = Self::pathspec_stdin(paths)?;
        self.command()
            .args(["add", "--pathspec-from-file=-", "--pathspec-file-nul", "--"])
            .stdin(data)
            .run()?;
        Ok(())
    }

    /// Stage every change, including untracked files.
    pub fn add_all(&self) -> GitResult<()> {
        self.command().args(["add", "-A"]).run()?;
        Ok(())
    }

    /// Unstage the given paths.
    ///
    /// No explicit `HEAD` is passed: `git reset` defaults to HEAD when it
    /// exists, and still works on an unborn branch (a fresh repository with
    /// no commits), where `HEAD` cannot be resolved.
    pub fn reset_paths(&self, paths: &[PathBuf]) -> GitResult<()> {
        let data = Self::pathspec_stdin(paths)?;
        self.command()
            .args([
                "reset",
                "-q",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ])
            .stdin(data)
            .run()?;
        Ok(())
    }

    /// Unstage everything (works on unborn branches too).
    pub fn reset_all(&self) -> GitResult<()> {
        self.command().args(["reset", "-q"]).run()?;
        Ok(())
    }

    /// Discard all changes (index and working tree) for the given paths,
    /// restoring them from HEAD.
    ///
    /// Callers must include both sides of a rename in `paths`. Untracked files
    /// are not affected; remove them explicitly with
    /// [`Repository::remove_untracked`] after confirmation.
    pub fn discard_paths(&self, paths: &[PathBuf]) -> GitResult<()> {
        let data = Self::pathspec_stdin(paths)?;
        let result = self
            .command()
            .args([
                "restore",
                "--source=HEAD",
                "--staged",
                "--worktree",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ])
            .stdin(data)
            .run();
        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                // On an unborn branch there is no HEAD. In that state every
                // tracked file is staged-added, so "discard" means removing it
                // from the index and deleting the file.
                if !self.rev_parse("HEAD").is_ok() {
                    let data = Self::pathspec_stdin(paths)?;
                    self.command()
                        .args([
                            "rm",
                            "--cached",
                            "-q",
                            "--force",
                            "--pathspec-from-file=-",
                            "--pathspec-file-nul",
                        ])
                        .stdin(data)
                        .run()?;
                    self.remove_untracked(paths)?;
                    return Ok(());
                }
                Err(e)
            }
        }
    }

    /// Delete untracked files. Callers must have confirmed with the user.
    pub fn remove_untracked(&self, paths: &[PathBuf]) -> GitResult<()> {
        for p in paths {
            let full = self.resolve_in_workdir(p)?;
            if full.is_dir() {
                std::fs::remove_dir_all(&full)?;
            } else {
                std::fs::remove_file(&full)?;
            }
        }
        Ok(())
    }

    /// Apply a patch to the index (stage) or to the index in reverse (unstage),
    /// or to the worktree in reverse (discard a hunk).
    pub fn apply_patch(&self, patch: &str, cached: bool, reverse: bool, worktree: bool) -> GitResult<()> {
        if patch.trim().is_empty() {
            return Err(GitError::InvalidState {
                message: "The selected change produced an empty patch.".into(),
                recovery: None,
            });
        }
        let mut args: Vec<&str> = vec!["apply", "--recount", "--whitespace=nowarn"];
        if cached {
            args.push("--cached");
        }
        if reverse {
            args.push("--reverse");
        }
        if worktree {
            // default applies to worktree; nothing to add
        }
        let out = self
            .command()
            .args(args)
            .arg("-")
            .stdin(patch.as_bytes().to_vec())
            .run();
        match out {
            Ok(_) => Ok(()),
            Err(GitError::CommandFailed { stderr, .. }) => Err(GitError::InvalidState {
                message: "The patch could not be applied cleanly.".into(),
                recovery: Some(format!(
                    "Refresh the view and try again. Git said: {}",
                    stderr.lines().next().unwrap_or("")
                )),
            }),
            Err(e) => Err(e),
        }
    }

    // -----------------------------------------------------------------
    // Commits
    // -----------------------------------------------------------------

    /// Create a commit from the current index.
    pub fn create_commit(&self, opts: &CommitOptions) -> GitResult<String> {
        if opts.message.trim().is_empty() {
            return Err(GitError::InvalidState {
                message: "Please enter a commit message.".into(),
                recovery: None,
            });
        }
        if !opts.amend && !opts.allow_empty {
            let staged = self.status()?.staged_count();
            if staged == 0 {
                return Err(GitError::InvalidState {
                    message: "There is nothing staged to commit.".into(),
                    recovery: Some(
                        "Stage files in the Changes panel before committing, or amend the last commit."
                            .into(),
                    ),
                });
            }
        }
        let mut args: Vec<std::ffi::OsString> = vec!["commit".into(), "-F".into(), "-".into()];
        if opts.amend {
            args.push("--amend".into());
        }
        if opts.sign_off {
            args.push("--signoff".into());
        }
        match &opts.gpg_sign {
            Some(Some(key)) if !key.is_empty() => {
                args.push(format!("--gpg-sign={key}").into());
            }
            Some(_) => {
                args.push("--gpg-sign".into());
            }
            None => {
                args.push("--no-gpg-sign".into());
            }
        }
        if opts.allow_empty {
            args.push("--allow-empty".into());
        }
        if let Some(author) = &opts.author {
            args.push(format!("--author={author}").into());
        }
        self.command()
            .args(args)
            .stdin(opts.message.clone().into_bytes())
            .run()?;
        self.rev_parse("HEAD")
    }

    /// Full commit message of HEAD (subject + body), used for amend.
    pub fn head_commit_message(&self) -> GitResult<String> {
        let out = self
            .command()
            .args(["log", "-1", "--format=%B"])
            .run()?;
        if out.code == Some(0) {
            Ok(out.stdout_lossy().trim_end_matches('\n').to_string())
        } else {
            Ok(String::new())
        }
    }

    // -----------------------------------------------------------------
    // Branches
    // -----------------------------------------------------------------

    pub fn create_branch(
        &self,
        name: &str,
        start_point: Option<&str>,
        checkout: bool,
    ) -> GitResult<()> {
        validate_ref_name(name)?;
        if checkout {
            return self.checkout_new_branch(name, start_point);
        }
        let mut cmd = self.command().args(["branch", name]);
        if let Some(sp) = start_point {
            validate_refish(sp)?;
            cmd = cmd.arg(sp);
        }
        cmd.run()?;
        Ok(())
    }

    pub fn checkout_branch(&self, name: &str) -> GitResult<()> {
        validate_ref_name(name)?;
        self.command().args(["switch", name]).run()?;
        Ok(())
    }

    /// Create and switch to a branch in one step.
    pub fn checkout_new_branch(&self, name: &str, start_point: Option<&str>) -> GitResult<()> {
        validate_ref_name(name)?;
        let mut cmd = self.command().args(["switch", "-c", name]);
        if let Some(sp) = start_point {
            validate_refish(sp)?;
            cmd = cmd.arg(sp);
        }
        cmd.run()?;
        Ok(())
    }

    pub fn checkout_detached(&self, rev: &str) -> GitResult<()> {
        if rev.starts_with('-') {
            return Err(GitError::InvalidState {
                message: "Invalid revision.".into(),
                recovery: None,
            });
        }
        self.command().args(["switch", "--detach", rev]).run()?;
        Ok(())
    }

    pub fn delete_branch(&self, name: &str, force: bool) -> GitResult<()> {
        validate_ref_name(name)?;
        self.command()
            .args(["branch", if force { "-D" } else { "-d" }, name])
            .run()?;
        Ok(())
    }

    pub fn rename_branch(&self, old: &str, new: &str, force: bool) -> GitResult<()> {
        validate_ref_name(new)?;
        let mut cmd = self.command().args(["branch", "-m"]);
        if force {
            cmd = cmd.arg("-M");
        }
        cmd.args([old, new]).run()?;
        Ok(())
    }

    pub fn set_upstream(&self, branch: &str, upstream: &str) -> GitResult<()> {
        validate_ref_name(branch)?;
        self.command()
            .args(["branch", "--set-upstream-to", upstream, branch])
            .run()?;
        Ok(())
    }

    pub fn unset_upstream(&self, branch: &str) -> GitResult<()> {
        validate_ref_name(branch)?;
        self.command()
            .args(["branch", "--unset-upstream", branch])
            .run()?;
        Ok(())
    }

    pub fn upstream_of_current(&self) -> GitResult<Option<String>> {
        let out = self
            .command()
            .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
            .run_unchecked()?;
        if out.code == Some(0) {
            let s = out.stdout_lossy().trim().to_string();
            if s.is_empty() { Ok(None) } else { Ok(Some(s)) }
        } else {
            Ok(None)
        }
    }

    /// Number of commits `rev` is ahead/behind its upstream.
    pub fn ahead_behind(&self, rev: &str, upstream: &str) -> GitResult<(u32, u32)> {
        let out = self
            .command()
            .args(["rev-list", "--left-right", "--count"])
            .arg(format!("{upstream}...{rev}"))
            .run_unchecked()?;
        if out.code == Some(0) {
            let s = out.stdout_lossy();
            let mut it = s.split_whitespace();
            let behind: u32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            let ahead: u32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            Ok((ahead, behind))
        } else {
            Ok((0, 0))
        }
    }

    // -----------------------------------------------------------------
    // Merge / rebase / cherry-pick / revert / reset
    // -----------------------------------------------------------------

    pub fn merge(
        &self,
        target: &str,
        no_ff: bool,
        squash: bool,
        message: Option<&str>,
    ) -> GitResult<MergeOutcome> {
        validate_refish(target)?;
        let mut args: Vec<std::ffi::OsString> =
            vec!["merge".into(), "--no-edit".into(), "--no-gpg-sign".into()];
        if no_ff {
            args.push("--no-ff".into());
        }
        if squash {
            args.push("--squash".into());
        }
        if let Some(m) = message {
            args.push("-m".into());
            args.push(m.into());
        }
        args.push(target.into());
        let result = self.command().args(args).run();
        match result {
            Ok(out) => {
                let text = out.stdout_lossy();
                if text.contains("Already up to date") || text.contains("Already up-to-date") {
                    Ok(MergeOutcome::UpToDate)
                } else if text.contains("Fast-forward") {
                    Ok(MergeOutcome::FastForward)
                } else {
                    Ok(MergeOutcome::Merged)
                }
            }
            Err(e) => self.classify_stop(e),
        }
    }

    /// Continue an in-progress merge (after conflicts were resolved).
    pub fn merge_continue(&self) -> GitResult<MergeOutcome> {
        let result = self
            .command()
            .args(["commit", "--no-edit", "--no-gpg-sign"])
            .run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn merge_abort(&self) -> GitResult<()> {
        self.command().args(["merge", "--abort"]).run()?;
        Ok(())
    }

    pub fn rebase(
        &self,
        upstream: Option<&str>,
        onto: Option<&str>,
        branch: Option<&str>,
    ) -> GitResult<MergeOutcome> {
        if let Some(u) = upstream {
            validate_refish(u)?;
        }
        if let Some(o) = onto {
            validate_refish(o)?;
        }
        let mut args: Vec<std::ffi::OsString> = vec!["rebase".into(), "--no-autosquash".into()];
        if let Some(b) = branch {
            validate_ref_name(b)?;
        }
        if onto.is_some() {
            args.push("--onto".into());
            args.push(onto.unwrap().into());
        }
        if let Some(u) = upstream {
            args.push(u.into());
        }
        if let Some(b) = branch {
            args.push(b.into());
        }
        let result = self.command().args(args).run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn rebase_continue(&self) -> GitResult<MergeOutcome> {
        let result = self
            .command()
            .args(["rebase", "--continue"])
            .env("GIT_EDITOR", "true")
            .run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn rebase_skip(&self) -> GitResult<MergeOutcome> {
        let result = self.command().args(["rebase", "--skip"]).run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn rebase_abort(&self) -> GitResult<()> {
        self.command().args(["rebase", "--abort"]).run()?;
        Ok(())
    }

    pub fn cherry_pick(&self, rev: &str) -> GitResult<MergeOutcome> {
        validate_refish(rev)?;
        let result = self
            .command()
            .args(["cherry-pick", "--no-gpg-sign", rev])
            .run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn cherry_pick_continue(&self) -> GitResult<MergeOutcome> {
        let result = self
            .command()
            .args(["cherry-pick", "--continue"])
            .env("GIT_EDITOR", "true")
            .run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn cherry_pick_abort(&self) -> GitResult<()> {
        self.command().args(["cherry-pick", "--abort"]).run()?;
        Ok(())
    }

    pub fn revert(&self, rev: &str, mainline: Option<u32>) -> GitResult<MergeOutcome> {
        validate_refish(rev)?;
        let mut args: Vec<std::ffi::OsString> =
            vec!["revert".into(), "--no-edit".into(), "--no-gpg-sign".into()];
        if let Some(m) = mainline {
            args.push("-m".into());
            args.push(m.to_string().into());
        }
        args.push(rev.into());
        let result = self.command().args(args).run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn revert_continue(&self) -> GitResult<MergeOutcome> {
        let result = self
            .command()
            .args(["revert", "--continue"])
            .env("GIT_EDITOR", "true")
            .run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn revert_abort(&self) -> GitResult<()> {
        self.command().args(["revert", "--abort"]).run()?;
        Ok(())
    }

    pub fn reset(&self, mode: ResetMode, target: &str) -> GitResult<()> {
        validate_refish(target)?;
        // No trailing `--`: a pathspec reset has different (safer) semantics
        // that leave untracked files alone, which is not what "reset" means
        // here.
        self.command().args(["reset", mode.flag(), target]).run()?;
        Ok(())
    }

    /// Classify a failed git command: if the repository is now in a conflicted
    /// state, return `MergeOutcome::Conflicts` instead of a hard error.
    fn classify_stop(&self, error: GitError) -> GitResult<MergeOutcome> {
        if let Ok(status) = self.status() {
            if status.has_conflicts() {
                let paths = status
                    .conflicted()
                    .iter()
                    .map(|e| e.path.clone())
                    .collect();
                return Ok(MergeOutcome::Conflicts(paths));
            }
        }
        Err(error)
    }

    // -----------------------------------------------------------------
    // Remotes and network operations
    // -----------------------------------------------------------------

    pub fn add_remote(&self, name: &str, url: &str) -> GitResult<()> {
        validate_remote_name(name)?;
        self.command().args(["remote", "add", name, url]).run()?;
        Ok(())
    }

    pub fn set_remote_url(&self, name: &str, url: &str, push: bool) -> GitResult<()> {
        validate_remote_name(name)?;
        let mut args = vec!["remote", "set-url"];
        if push {
            args.push("--push");
        }
        args.push(name);
        args.push(url);
        self.command().args(args).run()?;
        Ok(())
    }

    pub fn remove_remote(&self, name: &str) -> GitResult<()> {
        validate_remote_name(name)?;
        self.command().args(["remote", "remove", name]).run()?;
        Ok(())
    }

    pub fn rename_remote(&self, old: &str, new: &str) -> GitResult<()> {
        validate_remote_name(old)?;
        validate_remote_name(new)?;
        self.command().args(["remote", "rename", old, new]).run()?;
        Ok(())
    }

    pub fn fetch(
        &self,
        remote: Option<&str>,
        prune: bool,
        tags: bool,
        cancel: Option<Cancellation>,
        on_progress: impl FnMut(StreamKind, &str) + Send + 'static,
    ) -> GitResult<()> {
        let mut args: Vec<std::ffi::OsString> = vec!["fetch".into(), "--progress".into()];
        if prune {
            args.push("--prune".into());
        }
        if tags {
            args.push("--tags".into());
        }
        match remote {
            Some(r) => {
                validate_remote_name(r)?;
                args.push(r.into());
            }
            None => args.push("--all".into()),
        }
        let mut cmd = self.command().args(args);
        if let Some(c) = cancel {
            cmd = cmd.cancel(c);
        }
        cmd.run_streaming(on_progress)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &self,
        remote: &str,
        local_ref: &str,
        remote_ref: &str,
        set_upstream: bool,
        force: ForceMode,
        cancel: Option<Cancellation>,
        on_progress: impl FnMut(StreamKind, &str) + Send + 'static,
    ) -> GitResult<()> {
        validate_remote_name(remote)?;
        if local_ref.starts_with('-') || remote_ref.starts_with('-') {
            return Err(GitError::InvalidState {
                message: "Invalid refspec.".into(),
                recovery: None,
            });
        }
        let mut args: Vec<std::ffi::OsString> = vec!["push".into(), "--progress".into()];
        match force {
            ForceMode::None => {}
            ForceMode::WithLease => args.push("--force-with-lease".into()),
            ForceMode::Force => args.push("--force".into()),
        }
        if set_upstream {
            args.push("--set-upstream".into());
        }
        args.push(remote.into());
        args.push(format!("{local_ref}:{remote_ref}").into());
        let mut cmd = self.command().args(args);
        if let Some(c) = cancel {
            cmd = cmd.cancel(c);
        }
        cmd.run_streaming(on_progress)?;
        Ok(())
    }

    pub fn push_delete_branch(
        &self,
        remote: &str,
        branch: &str,
        cancel: Option<Cancellation>,
        on_progress: impl FnMut(StreamKind, &str) + Send + 'static,
    ) -> GitResult<()> {
        validate_remote_name(remote)?;
        let mut cmd = self.command().args([
            "push",
            "--progress",
            remote,
            "--delete",
            branch,
        ]);
        if let Some(c) = cancel {
            cmd = cmd.cancel(c);
        }
        cmd.run_streaming(on_progress)?;
        Ok(())
    }

    pub fn push_tags(
        &self,
        remote: &str,
        cancel: Option<Cancellation>,
        on_progress: impl FnMut(StreamKind, &str) + Send + 'static,
    ) -> GitResult<()> {
        validate_remote_name(remote)?;
        let mut cmd = self
            .command()
            .args(["push", "--progress", remote, "--tags"]);
        if let Some(c) = cancel {
            cmd = cmd.cancel(c);
        }
        cmd.run_streaming(on_progress)?;
        Ok(())
    }

    pub fn push_tag(
        &self,
        remote: &str,
        tag: &str,
        cancel: Option<Cancellation>,
        on_progress: impl FnMut(StreamKind, &str) + Send + 'static,
    ) -> GitResult<()> {
        validate_remote_name(remote)?;
        validate_ref_name(tag)?;
        let mut cmd = self.command().args([
            "push",
            "--progress",
            remote,
            tag,
        ]);
        if let Some(c) = cancel {
            cmd = cmd.cancel(c);
        }
        cmd.run_streaming(on_progress)?;
        Ok(())
    }

    pub fn delete_remote_tag(
        &self,
        remote: &str,
        tag: &str,
        cancel: Option<Cancellation>,
        on_progress: impl FnMut(StreamKind, &str) + Send + 'static,
    ) -> GitResult<()> {
        validate_remote_name(remote)?;
        validate_ref_name(tag)?;
        let mut cmd = self.command().args([
            "push",
            "--progress",
            remote,
            "--delete",
            tag,
        ]);
        if let Some(c) = cancel {
            cmd = cmd.cancel(c);
        }
        cmd.run_streaming(on_progress)?;
        Ok(())
    }

    /// Fetch, then merge or rebase the configured upstream.
    #[allow(clippy::too_many_arguments)]
    pub fn pull(
        &self,
        remote: Option<&str>,
        rebase: bool,
        ff_only: bool,
        cancel: Option<Cancellation>,
        on_progress: impl FnMut(StreamKind, &str) + Send + 'static,
    ) -> GitResult<MergeOutcome> {
        let upstream = self
            .upstream_of_current()?
            .ok_or_else(|| GitError::InvalidState {
                message: "The current branch has no upstream configured.".into(),
                recovery: Some(
                    "Push the branch and enable \"set upstream\", or set an upstream in the Branches panel."
                        .into(),
                ),
            })?;
        self.fetch(remote, false, false, cancel, on_progress)?;
        if rebase {
            self.rebase(Some(&upstream), None, None)
        } else if ff_only {
            let result = self
                .command()
                .args(["merge", "--ff-only", "--no-edit", &upstream, "--"])
                .run();
            match result {
                Ok(_) => Ok(MergeOutcome::FastForward),
                Err(e) => self.classify_stop(e),
            }
        } else {
            self.merge(&upstream, false, false, None)
        }
    }

    // -----------------------------------------------------------------
    // Stash
    // -----------------------------------------------------------------

    pub fn stash_save(
        &self,
        message: Option<&str>,
        include_untracked: bool,
        keep_index: bool,
        paths: Option<&[PathBuf]>,
    ) -> GitResult<()> {
        let mut args: Vec<std::ffi::OsString> = vec!["stash".into(), "push".into()];
        if include_untracked {
            args.push("--include-untracked".into());
        }
        if keep_index {
            args.push("--keep-index".into());
        }
        if let Some(m) = message {
            if !m.is_empty() {
                args.push("-m".into());
                args.push(m.into());
            }
        }
        if let Some(paths) = paths {
            if paths.is_empty() {
                return Err(GitError::InvalidState {
                    message: "No files selected for stashing.".into(),
                    recovery: None,
                });
            }
            args.push("--".into());
            for p in paths {
                args.push(p.as_os_str().to_os_string());
            }
        }
        self.command().args(args).run()?;
        Ok(())
    }

    fn stash_ref(index: usize) -> String {
        format!("stash@{{{index}}}")
    }

    pub fn stash_apply(&self, index: usize, reinstate_index: bool) -> GitResult<MergeOutcome> {
        let mut args: Vec<std::ffi::OsString> = vec!["stash".into(), "apply".into()];
        if reinstate_index {
            args.push("--index".into());
        }
        args.push(Self::stash_ref(index).into());
        let result = self.command().args(args).run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn stash_pop(&self, index: usize, reinstate_index: bool) -> GitResult<MergeOutcome> {
        let mut args: Vec<std::ffi::OsString> = vec!["stash".into(), "pop".into()];
        if reinstate_index {
            args.push("--index".into());
        }
        args.push(Self::stash_ref(index).into());
        let result = self.command().args(args).run();
        match result {
            Ok(_) => Ok(MergeOutcome::Merged),
            Err(e) => self.classify_stop(e),
        }
    }

    pub fn stash_drop(&self, index: usize) -> GitResult<()> {
        self.command()
            .args(["stash", "drop", &Self::stash_ref(index)])
            .run()?;
        Ok(())
    }

    pub fn stash_clear(&self) -> GitResult<()> {
        self.command().args(["stash", "clear"]).run()?;
        Ok(())
    }

    pub fn stash_branch(&self, index: usize, branch: &str) -> GitResult<()> {
        validate_ref_name(branch)?;
        self.command()
            .args(["stash", "branch", branch])
            .arg(Self::stash_ref(index))
            .run()?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Tags
    // -----------------------------------------------------------------

    pub fn create_tag(
        &self,
        name: &str,
        target: Option<&str>,
        message: Option<&str>,
        sign: bool,
        force: bool,
    ) -> GitResult<()> {
        validate_ref_name(name)?;
        let mut args: Vec<std::ffi::OsString> = vec!["tag".into()];
        if force {
            args.push("-f".into());
        }
        if sign {
            args.push("-s".into());
        }
        if let Some(m) = message {
            args.push("-a".into());
            args.push("-m".into());
            args.push(m.into());
        } else if sign {
            // signing implies an annotated tag; provide an empty message
            args.push("-m".into());
            args.push(name.into());
        } else {
            args.push("--no-sign".into());
        }
        args.push(name.into());
        if let Some(t) = target {
            validate_refish(t)?;
            args.push(t.into());
        }
        self.command().args(args).run()?;
        Ok(())
    }

    pub fn delete_tag(&self, name: &str) -> GitResult<()> {
        validate_ref_name(name)?;
        self.command().args(["tag", "-d", name]).run()?;
        Ok(())
    }

    /// Verify whether a signature can be created with the configured key.
    pub fn signing_configured(&self) -> bool {
        self.config_get("user.signingkey").ok().flatten().is_some()
            || self.config_get("gpg.format").ok().flatten().is_some()
    }

    /// Create a worktree path string relative to the repository root.
    pub fn relative(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            let joined = normalize_path(path);
            joined
                .strip_prefix(self.workdir())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| path.to_path_buf())
        } else {
            path.to_path_buf()
        }
    }
}

/// Reject ref names that git would reject anyway, with a friendly message.
pub fn validate_ref_name(name: &str) -> GitResult<()> {
    let invalid = name.is_empty()
        || name.starts_with('-')
        || name.starts_with('.')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("@{")
        || name.contains('\\')
        || name.contains(' ')
        || name.contains('~')
        || name.contains('^')
        || name.contains(':')
        || name.contains('?')
        || name.contains('*')
        || name.contains('[')
        || name.chars().any(|c| c.is_control())
        || name.contains("//")
        || name == "@";
    if invalid {
        Err(GitError::InvalidState {
            message: format!("`{name}` is not a valid Git reference name."),
            recovery: None,
        })
    } else {
        Ok(())
    }
}

pub fn validate_remote_name(name: &str) -> GitResult<()> {
    if name.is_empty()
        || name.starts_with('-')
        || name.contains(' ')
        || name.contains('\t')
        || name.contains('\n')
        || name.chars().any(|c| c.is_control())
    {
        return Err(GitError::InvalidState {
            message: format!("`{name}` is not a valid remote name."),
            recovery: None,
        });
    }
    Ok(())
}

pub fn validate_refish(name: &str) -> GitResult<()> {
    if name.is_empty()
        || name.starts_with('-')
        || name.contains(char::is_whitespace)
        || name.chars().any(|c| c.is_control())
    {
        return Err(GitError::InvalidState {
            message: format!("`{name}` is not a valid revision or reference."),
            recovery: None,
        });
    }
    Ok(())
}

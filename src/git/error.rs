use std::fmt;
use std::path::PathBuf;

/// Errors produced by the Git service layer.
///
/// Every variant carries enough information to (a) show a human friendly
/// message and (b) show the raw technical output for advanced users.
#[derive(Debug)]
pub enum GitError {
    /// The `git` executable could not be found.
    GitNotFound,
    /// A path is not inside a Git repository.
    NotARepository { path: PathBuf },
    /// A git command exited with a non-zero status.
    CommandFailed {
        args: Vec<String>,
        cwd: Option<PathBuf>,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    /// Authentication against a remote failed.
    AuthFailed {
        remote: Option<String>,
        stderr: String,
    },
    /// The repository is corrupt or in an unusable state.
    CorruptRepository { path: PathBuf, details: String },
    /// Output of a git command could not be parsed.
    Parse {
        context: &'static str,
        details: String,
    },
    /// I/O error while spawning a process or touching the filesystem.
    Io(std::io::Error),
    /// A running operation was cancelled by the user.
    Cancelled,
    /// The operation cannot be performed in the current repository state.
    InvalidState { message: String, recovery: Option<String> },
}

impl GitError {
    /// Short, human readable message suitable for a dialog or banner.
    pub fn user_message(&self) -> String {
        match self {
            GitError::GitNotFound => "Git is not installed.\n\nInstall Git (for example with `sudo dnf install git`) and try again.".into(),
            GitError::NotARepository { path } => format!(
                "This directory is not a Git repository.\n\n{}\n\nChoose another directory or initialize a new repository.",
                path.display()
            ),
            GitError::CommandFailed { stderr, args, .. } => {
                let first = first_meaningful_line(stderr);
                if first.is_empty() {
                    format!("The git command `git {}` failed.", args.join(" "))
                } else {
                    first
                }
            }
            GitError::AuthFailed { remote, .. } => {
                let where_ = match remote {
                    Some(r) => format!(" for remote `{r}`"),
                    None => String::new(),
                };
                format!(
                    "Authentication failed{where_}.\n\nIf your SSH key has a passphrase, GitTrellis shows a prompt for it.\
Check that the key is available (for example with `ssh-add -l`), that it is added to your GitHub/GitLab account, \
and that your credential helper is configured for HTTPS remotes."
                )
            }
            GitError::CorruptRepository { path, .. } => format!(
                "The repository at {} appears to be damaged.\n\nRun `git fsck` for details.",
                path.display()
            ),
            GitError::Parse { context, .. } => {
                format!("Unexpected output from Git while reading {context}.")
            }
            GitError::Io(e) => format!("I/O error: {e}"),
            GitError::Cancelled => "Operation cancelled.".into(),
            GitError::InvalidState { message, .. } => message.clone(),
        }
    }

    /// Raw technical details, including command output. Safe to copy into a
    /// bug report. Never contains credentials injected by this application
    /// (Git itself is responsible for redacting secrets, and we never pass
    /// secrets on the command line).
    pub fn technical_details(&self) -> String {
        let mut out = String::new();
        match self {
            GitError::CommandFailed {
                args,
                cwd,
                code,
                stdout,
                stderr,
            } => {
                out.push_str(&format!(
                    "$ git {}\n",
                    args.iter()
                        .map(|a| shell_quote(a))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
                if let Some(cwd) = cwd {
                    out.push_str(&format!("working directory: {}\n", cwd.display()));
                }
                if let Some(code) = code {
                    out.push_str(&format!("exit code: {code}\n"));
                }
                if !stdout.trim().is_empty() {
                    out.push_str("\n--- stdout ---\n");
                    out.push_str(stdout.trim_end());
                    out.push('\n');
                }
                if !stderr.trim().is_empty() {
                    out.push_str("\n--- stderr ---\n");
                    out.push_str(stderr.trim_end());
                    out.push('\n');
                }
            }
            GitError::AuthFailed { stderr, .. } => {
                out.push_str("authentication failed\n");
                out.push_str(stderr.trim_end());
            }
            GitError::CorruptRepository { path, details } => {
                out.push_str(&format!("repository: {}\n{details}", path.display()));
            }
            GitError::InvalidState { message, recovery } => {
                out.push_str(message);
                if let Some(r) = recovery {
                    out.push_str(&format!("\nrecovery: {r}"));
                }
            }
            other => out.push_str(&other.to_string()),
        }
        out
    }

    /// Suggested recovery action for the UI, when known.
    pub fn recovery_action(&self) -> Option<String> {
        match self {
            GitError::GitNotFound => Some("Install the git package and restart GitTrellis.".into()),
            GitError::NotARepository { .. } => {
                Some("Open another directory or initialize a new repository here.".into())
            }
            GitError::AuthFailed { .. } => Some(
                "Run `ssh-add ~/.ssh/id_ed25519` once per session to cache the passphrase, or configure a credential helper. Use `ssh -T git@host` in a terminal to test the key."
                    .into(),
            ),
            GitError::CorruptRepository { .. } => {
                Some("Run `git fsck` and review the reported problems.".into())
            }
            GitError::InvalidState { recovery, .. } => recovery.clone(),
            GitError::Cancelled => None,
            _ => None,
        }
    }

    /// Classify common stderr patterns into structured errors.
    pub fn from_stderr(
        args: Vec<String>,
        cwd: Option<PathBuf>,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    ) -> GitError {
        let lower = stderr.to_lowercase();
        if lower.contains("authentication failed")
            || lower.contains("permission denied (publickey")
            || lower.contains("could not read username")
            || lower.contains("terminal prompts disabled")
            || lower.contains("invalid username or password")
            || lower.contains("access denied")
        {
            return GitError::AuthFailed {
                remote: None,
                stderr,
            };
        }
        if lower.contains("not a git repository") {
            return GitError::NotARepository {
                path: cwd.unwrap_or_default(),
            };
        }
        if lower.contains("bad object")
            || lower.contains("unable to read tree")
            || lower.contains("object file")
            || lower.contains("corrupt")
            || lower.contains("invalid object")
            || lower.contains("loose object")
            || lower.contains("index file smaller")
            || lower.contains("bad index file")
        {
            return GitError::CorruptRepository {
                path: cwd.unwrap_or_default(),
                details: stderr,
            };
        }
        GitError::CommandFailed {
            args,
            cwd,
            code,
            stdout,
            stderr,
        }
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.user_message())
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(e: std::io::Error) -> Self {
        GitError::Io(e)
    }
}

fn first_meaningful_line(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("hint:"))
        .unwrap_or("")
        .trim_start_matches("fatal: ")
        .trim_start_matches("error: ")
        .to_string()
}

/// Quote a word for display only (never for execution).
pub fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-/=:+,@".contains(c))
        && !s.is_empty()
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub type GitResult<T> = Result<T, GitError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_not_a_repo_is_helpful() {
        let e = GitError::NotARepository {
            path: PathBuf::from("/tmp/x"),
        };
        let m = e.user_message();
        assert!(m.contains("not a Git repository"));
        assert!(m.contains("initialize"));
    }

    #[test]
    fn command_failed_prefers_stderr_first_line() {
        let e = GitError::CommandFailed {
            args: vec!["merge".into(), "x".into()],
            cwd: None,
            code: Some(1),
            stdout: String::new(),
            stderr: "error: Your local changes would be overwritten\nhint: more".into(),
        };
        assert_eq!(
            e.user_message(),
            "Your local changes would be overwritten"
        );
    }

    #[test]
    fn format_bad_path_is_human_readable() {
        let e = GitError::NotARepository {
            path: PathBuf::from("/home/u/p"),
        };
        assert!(e.user_message().contains("/home/u/p"));
    }

    #[test]
    fn auth_classification() {
        let e = GitError::from_stderr(
            vec!["fetch".into()],
            None,
            Some(128),
            String::new(),
            "fatal: Authentication failed for 'https://example.com/repo'".into(),
        );
        assert!(matches!(e, GitError::AuthFailed { .. }));
    }

    #[test]
    fn not_a_repo_classification() {
        let e = GitError::from_stderr(
            vec!["status".into()],
            Some(PathBuf::from("/tmp")),
            Some(128),
            String::new(),
            "fatal: not a git repository (or any of the parent directories): .git".into(),
        );
        assert!(matches!(e, GitError::NotARepository { .. }));
    }

    #[test]
    fn shell_quote_handles_spaces() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("plain"), "plain");
        assert_eq!(shell_quote("-dash"), "-dash");
    }
}

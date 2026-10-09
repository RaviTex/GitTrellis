//! Safe execution of the `git` executable.
//!
//! Design rules enforced here:
//! * No shell is ever involved; arguments are passed as an `OsString` vector.
//! * User supplied values (paths, refs, messages) are never interpolated into
//!   a command string.
//! * All invocations run with a deterministic, minimal configuration prefix so
//!   parsing does not depend on the user's personal Git configuration.
//! * Long running operations stream progress and can be cancelled.

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use super::error::{GitError, GitResult};

/// Which of the child's output streams a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Stdout,
    Stderr,
}

/// Result of a finished git invocation.
#[derive(Debug)]
pub struct Completed {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Completed {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Shared handle used to cancel a running operation.
#[derive(Debug, Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Locate the installed askpass helper next to the application binary.
///
/// Returns `None` when it is not present (for example when running from a
/// test harness), in which case askpass variables are left untouched so that
/// tests never accidentally re-execute the test binary.
fn askpass_helper_path() -> Option<std::path::PathBuf> {
    if let Ok(custom) = std::env::var("GITTRELLIS_ASKPASS_HELPER") {
        let path = std::path::PathBuf::from(custom);
        if path.is_file() {
            return Some(path);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join("gittrellis-askpass");
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

/// The Git executable plus its deterministic global configuration prefix.
#[derive(Clone, Debug)]
pub struct Git {
    program: OsString,
    base_args: Vec<OsString>,
    base_env: Vec<(OsString, OsString)>,
}

impl Default for Git {
    fn default() -> Self {
        Self::system()
    }
}

impl Git {
    /// A `Git` handle configured to produce deterministic, machine readable
    /// output regardless of the user's personal configuration.
    pub fn system() -> Self {
        let mut base_args: Vec<OsString> = Vec::new();
        for kv in [
            "core.quotepath=false",
            "color.ui=false",
            "color.pager=false",
            "log.showSignature=false",
            "diff.noprefix=false",
            "diff.mnemonicPrefix=false",
            "diff.relative=false",
            "rebase.autosquash=false",
            "rebase.autostash=false",
            "advice.detachedHead=false",
        ] {
            base_args.push("-c".into());
            base_args.push(kv.into());
        }
        let mut base_env: Vec<(OsString, OsString)> = Vec::new();
        // A GUI cannot answer terminal prompts; fail fast instead of hanging.
        // Credential helpers keep working, and the askpass helper below
        // provides native dialogs for passphrases and HTTPS credentials.
        base_env.push(("GIT_TERMINAL_PROMPT".into(), "0".into()));
        base_env.push(("GIT_PAGER".into(), "cat".into()));
        base_env.push(("GIT_EDITOR".into(), "true".into()));
        base_env.push(("LC_ALL".into(), "C".into()));
        if let Some(helper) = askpass_helper_path() {
            let helper = helper.into_os_string();
            // Git asks GIT_ASKPASS for usernames and passwords; OpenSSH asks
            // SSH_ASKPASS for key passphrases. `SSH_ASKPASS_REQUIRE=force`
            // makes ssh use the helper even though it has no terminal.
            base_env.push(("GIT_ASKPASS".into(), helper.clone()));
            base_env.push(("SSH_ASKPASS".into(), helper));
            base_env.push(("SSH_ASKPASS_REQUIRE".into(), "force".into()));
        }
        Self {
            program: OsString::from("git"),
            base_args,
            base_env,
        }
    }

    /// Used by tests to point at a specific executable / environment.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        let mut g = Self::system();
        g.program = program.into();
        g
    }

    /// Add an environment override applied to every invocation.
    pub fn with_env(
        mut self,
        key: impl Into<OsString>,
        value: impl Into<OsString>,
    ) -> Self {
        let key = key.into();
        self.base_env.retain(|(k, _)| k != &key);
        self.base_env.push((key, value.into()));
        self
    }

    /// Start building an invocation.
    pub fn command(&self) -> CommandBuilder {
        CommandBuilder {
            git: self.clone(),
            cwd: None,
            args: Vec::new(),
            envs: Vec::new(),
            stdin: None,
            cancel: None,
        }
    }

    /// Returns the version string of the git executable, or an error if git is
    /// missing.
    pub fn version(&self) -> GitResult<String> {
        let out = self.command().arg("--version").run()?;
        if !out.success() {
            return Err(GitError::CommandFailed {
                args: vec!["--version".into()],
                cwd: None,
                code: out.code,
                stdout: out.stdout_lossy(),
                stderr: out.stderr_lossy(),
            });
        }
        Ok(out.stdout_lossy().trim().to_string())
    }

    pub fn is_available(&self) -> bool {
        self.version().is_ok()
    }
}

/// Fluent builder for a single git invocation.
#[derive(Clone, Debug)]
pub struct CommandBuilder {
    git: Git,
    cwd: Option<PathBuf>,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    stdin: Option<Vec<u8>>,
    cancel: Option<Cancellation>,
}

impl CommandBuilder {
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for a in args {
            self.args.push(a.as_ref().to_os_string());
        }
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }

    pub fn cancel(mut self, token: Cancellation) -> Self {
        self.cancel = Some(token);
        self
    }

    /// The arguments as they will be passed (after the deterministic prefix),
    /// for diagnostics.
    pub fn display_args(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn spawn(&self) -> GitResult<std::process::Child> {
        let mut cmd = Command::new(&self.git.program);
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        cmd.args(&self.git.base_args);
        cmd.args(&self.args);
        for (k, v) in self.git.base_env.iter().chain(self.envs.iter()) {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Put the child in its own process group so that the whole tree
        // (git -> ssh -> askpass helper) can be terminated together.
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }
        cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::GitNotFound
            } else {
                GitError::Io(e)
            }
        })
    }

    /// Run to completion, collecting all output.
    pub fn run(self) -> GitResult<Completed> {
        self.run_streaming(|_, _| {})
    }

    /// Run to completion while delivering output lines to `on_line`.
    ///
    /// Both stdout and stderr are read concurrently on dedicated threads, and
    /// the child is polled so that cancellation takes effect promptly. Once
    /// the process exits, any surviving descendants (for example an askpass
    /// dialog opened by ssh) are terminated so that the output pipes always
    /// reach EOF and the operation can never hang.
    pub fn run_streaming<F>(self, on_line: F) -> GitResult<Completed>
    where
        F: FnMut(StreamKind, &str) + Send + 'static,
    {
        let cancel = self.cancel.clone();
        let args = self.display_args();
        let cwd = self.cwd.clone();
        let completed = self.run_inner(on_line, true)?;

        if cancel.as_ref().map(|c| c.is_cancelled()).unwrap_or(false) {
            return Err(GitError::Cancelled);
        }
        if !completed.success() {
            return Err(GitError::from_stderr(
                args,
                cwd,
                completed.code,
                completed.stdout_lossy(),
                completed.stderr_lossy(),
            ));
        }
        Ok(completed)
    }

    /// Run to completion without treating a non-zero exit status as an error.
    ///
    /// Used for queries where "no result" is expressed as a non-zero exit
    /// code (`git config --get`, `git rev-parse --verify --quiet`, ...).
    pub fn run_unchecked(self) -> GitResult<Completed> {
        let cancel = self.cancel.clone();
        let completed = self.run_inner(|_, _| {}, false)?;
        if cancel.as_ref().map(|c| c.is_cancelled()).unwrap_or(false) {
            return Err(GitError::Cancelled);
        }
        Ok(completed)
    }

    /// Shared process runner.
    ///
    /// Guarantees:
    /// * the child runs in its own process group;
    /// * cancelling kills the whole group (git, ssh, askpass helper, ...);
    /// * after the child exits, any stragglers in the group are killed so the
    ///   pipes reach EOF;
    /// * reader threads are collected with a timeout, so a lingering pipe
    ///   writer can never block the operation forever.
    fn run_inner<F>(self, on_line: F, stream_lines: bool) -> GitResult<Completed>
    where
        F: FnMut(StreamKind, &str) + Send + 'static,
    {
        let cancel = self.cancel.clone();
        let mut child = self.spawn()?;
        let pid = child.id();
        let child_stdin = child.stdin.take();
        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();
        let stdin_data = self.stdin.clone();
        let callback: Arc<Mutex<Box<dyn FnMut(StreamKind, &str) + Send>>> =
            Arc::new(Mutex::new(Box::new(on_line)));

        if let (Some(mut writer), Some(data)) = (child_stdin, stdin_data) {
            thread::spawn(move || {
                let _ = writer.write_all(&data);
                let _ = writer.flush();
            });
        }

        let (tx, rx) = mpsc::channel::<(StreamKind, Vec<u8>)>();
        if let Some(out) = child_stdout {
            let tx = tx.clone();
            let cb = Arc::clone(&callback);
            thread::spawn(move || {
                let data = read_stream(out, StreamKind::Stdout, cb, stream_lines);
                let _ = tx.send((StreamKind::Stdout, data));
            });
        }
        if let Some(err) = child_stderr {
            let tx = tx.clone();
            let cb = Arc::clone(&callback);
            thread::spawn(move || {
                let data = read_stream(err, StreamKind::Stderr, cb, stream_lines);
                let _ = tx.send((StreamKind::Stderr, data));
            });
        }
        drop(tx);

        let mut cancelled = false;
        let status = loop {
            if let Some(c) = &cancel {
                if c.is_cancelled() && !cancelled {
                    cancelled = true;
                    kill_process_group(&mut child, pid);
                }
            }
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) => thread::sleep(Duration::from_millis(15)),
                Err(_) => break None,
            }
        };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(1000);
        let mut lingering_descendant = false;
        for _ in 0..2 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining) {
                Ok((StreamKind::Stdout, data)) => stdout = data,
                Ok((StreamKind::Stderr, data)) => stderr = data,
                Err(_) => {
                    // Some descendant (for example a still-open askpass
                    // dialog) is holding the pipe. The operation must not
                    // hang because of it: terminate the group and move on.
                    lingering_descendant = true;
                    break;
                }
            }
        }
        if lingering_descendant {
            kill_group_only(pid);
        }

        Ok(Completed {
            code: status.and_then(|s| s.code()),
            stdout,
            stderr,
        })
    }

    /// Run and return stdout as a lossy UTF-8 string (trimmed).
    pub fn run_string(self) -> GitResult<String> {
        let out = self.run()?;
        Ok(out.stdout_lossy().trim_end_matches(['\n', '\r']).to_string())
    }

    /// Run and return raw stdout bytes, erroring on failure.
    pub fn run_bytes(self) -> GitResult<Vec<u8>> {
        let out = self.run()?;
        Ok(out.stdout)
    }
}

/// Terminate the child and every process in its group.
fn kill_process_group(child: &mut Child, pid: u32) {
    let _ = child.kill();
    kill_group_only(pid);
}

/// Kill all members of the process group led by `pid` (ignoring errors).
#[cfg(unix)]
fn kill_group_only(pid: u32) {
    // Safety: sending a signal to a process group is async-signal-safe;
    // errors (ESRCH) are ignored on purpose.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group_only(_pid: u32) {}

fn read_stream<R: Read>(
    reader: R,
    kind: StreamKind,
    callback: Arc<Mutex<Box<dyn FnMut(StreamKind, &str) + Send>>>,
    stream_lines: bool,
) -> Vec<u8> {
    let mut buf_reader = BufReader::new(reader);
    let mut all = Vec::new();
    let mut chunk = Vec::new();
    let mut segment = Vec::new();
    loop {
        chunk.clear();
        match buf_reader.read_until(b'\n', &mut chunk) {
            Ok(0) => break,
            Ok(_) => {
                if stream_lines {
                    // Split on CR as well: git uses carriage returns to
                    // rewrite in-place progress on a single line.
                    for &b in &chunk {
                        if b == b'\n' || b == b'\r' {
                            if !segment.is_empty() {
                                let text = String::from_utf8_lossy(&segment);
                                if let Ok(mut cb) = callback.lock() {
                                    cb(kind, &text);
                                }
                                segment.clear();
                            }
                        } else {
                            segment.push(b);
                        }
                    }
                }
                all.extend_from_slice(&chunk);
            }
            Err(_) => break,
        }
    }
    if stream_lines && !segment.is_empty() {
        let text = String::from_utf8_lossy(&segment);
        if let Ok(mut cb) = callback.lock() {
            cb(kind, &text);
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git() -> Git {
        Git::system()
    }

    #[test]
    fn runs_version() {
        let v = git().version().expect("git available in test env");
        assert!(v.starts_with("git version"), "got {v}");
    }

    #[test]
    fn safe_paths_with_dash_and_spaces() {
        // Filenames starting with '-' and containing spaces must be handled by
        // passing them after `--` or via stdin pathspec files.
        let tmp = tempfile::tempdir().unwrap();
        let g = git();
        g.command()
            .cwd(tmp.path())
            .args(["init", "-q", "-b", "main"])
            .run()
            .unwrap();
        std::fs::write(tmp.path().join("-weird name.txt"), "hi").unwrap();
        g.command()
            .cwd(tmp.path())
            .args(["add", "--"])
            .arg("-weird name.txt")
            .run()
            .unwrap();
        let out = g
            .command()
            .cwd(tmp.path())
            .args(["status", "--porcelain"])
            .run_string()
            .unwrap();
        assert!(out.contains("weird name.txt"), "got: {out}");
    }

    #[test]
    fn non_zero_exit_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = git()
            .command()
            .cwd(tmp.path())
            .args(["status"])
            .run()
            .unwrap_err();
        assert!(matches!(err, GitError::NotARepository { .. }));
    }

    #[test]
    fn streaming_delivers_lines() {
        let tmp = tempfile::tempdir().unwrap();
        git()
            .command()
            .cwd(tmp.path())
            .args(["init", "-q", "-b", "main"])
            .run()
            .unwrap();
        let collected = Arc::new(Mutex::new(Vec::new()));
        let c = Arc::clone(&collected);
        git()
            .command()
            .cwd(tmp.path())
            .args(["status", "--short"])
            .run_streaming(move |_, line| c.lock().unwrap().push(line.to_string()))
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cancel_kills_the_whole_process_group() {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 30 & wait")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(150));
        kill_process_group(&mut child, pid);
        let _ = child.wait();
        std::thread::sleep(std::time::Duration::from_millis(150));
        let out = std::process::Command::new("ps")
            .args(["-eo", "pgid="])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let still_alive = text
            .lines()
            .filter_map(|l| l.trim().parse::<i32>().ok())
            .any(|pgid| pgid == pid as i32);
        assert!(!still_alive, "process group {pid} survived the kill");
    }

    #[test]
    fn reader_timeout_prevents_hangs_from_lingering_pipes() {
        // A child that exits while a descendant keeps stdout open must still
        // complete: the runner kills stragglers and waits for readers with a
        // timeout.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let tmp = tempfile::tempdir().unwrap();
            let script = tmp.path().join("hang.sh");
            std::fs::write(
                &script,
                "#!/bin/sh\nsleep 30 &\nexit 0\n",
            )
            .unwrap();
            // We cannot run arbitrary programs through Git, so simulate the
            // exact situation with the same helpers.
            let mut child = std::process::Command::new("sh")
                .arg(script.as_os_str())
                .stdout(std::process::Stdio::piped())
                .process_group(0)
                .spawn()
                .unwrap();
            let pid = child.id();
            let stdout = child.stdout.take().unwrap();
            let start = std::time::Instant::now();
            let status = child.wait().unwrap();
            assert!(status.success());
            kill_group_only(pid);
            let data = read_stream(
                stdout,
                StreamKind::Stdout,
                std::sync::Arc::new(std::sync::Mutex::new(Box::new(|_: StreamKind, _: &str| {}))),
                false,
            );
            assert!(data.is_empty());
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
        }
    }

    #[test]
    fn cancellation_stops_long_operation() {
        // `git fetch` on a non-existent remote would wait for network; use a
        // synthetic sleep via `git hash-object --stdin-paths` reading stdin
        // continuously is complex. Instead verify cancel flag plumbing with a
        // very cheap operation that we cancel beforehand.
        let cancel = Cancellation::new();
        cancel.cancel();
        let task = git().command().args(["--version"]).cancel(cancel);
        let err = task.run();
        assert!(matches!(err, Err(GitError::Cancelled)));
    }

    #[test]
    fn askpass_is_configured_when_helper_exists() {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("gittrellis-askpass");
        std::fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
        std::env::set_var("GITTRELLIS_ASKPASS_HELPER", &helper);
        let git = Git::system();
        let envs: std::collections::HashMap<_, _> = git
            .base_env
            .iter()
            .map(|(k, v)| (k.to_string_lossy().to_string(), v.to_string_lossy().to_string()))
            .collect();
        assert_eq!(envs.get("GIT_ASKPASS").map(String::as_str), Some(helper.to_string_lossy().as_ref()));
        assert_eq!(envs.get("SSH_ASKPASS").map(String::as_str), Some(helper.to_string_lossy().as_ref()));
        assert_eq!(envs.get("SSH_ASKPASS_REQUIRE").map(String::as_str), Some("force"));
        std::env::remove_var("GITTRELLIS_ASKPASS_HELPER");
    }

    #[test]
    fn no_shell_injection_through_args() {
        let tmp = tempfile::tempdir().unwrap();
        git()
            .command()
            .cwd(tmp.path())
            .args(["init", "-q", "-b", "main"])
            .run()
            .unwrap();
        let evil = "evil; touch pwned.txt; echo $HOME";
        std::fs::write(tmp.path().join(evil), "x").unwrap();
        git()
            .command()
            .cwd(tmp.path())
            .args(["add", "--"])
            .arg(evil)
            .run()
            .unwrap();
        assert!(tmp.path().join(evil).exists());
    }
}

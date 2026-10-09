//! Structured application logging.
//!
//! * Logs go to stderr and to a daily file under the XDG state directory.
//! * A bounded in-memory buffer feeds the in-app diagnostics view.
//! * Secret-looking material is redacted before anything is written.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(&self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogRecord {
    pub time: String,
    pub level: Level,
    pub message: String,
}

struct LoggerState {
    records: VecDeque<LogRecord>,
    file: Option<std::fs::File>,
    file_path: Option<PathBuf>,
    level: Level,
}

static STATE: OnceLock<Mutex<LoggerState>> = OnceLock::new();
static CAPACITY: AtomicUsize = AtomicUsize::new(4000);

fn state() -> &'static Mutex<LoggerState> {
    STATE.get_or_init(|| {
        Mutex::new(LoggerState {
            records: VecDeque::new(),
            file: None,
            file_path: None,
            level: Level::Info,
        })
    })
}

/// Initialise file logging. Safe to call multiple times.
pub fn init(level: Level) {
    let state_dir = super::settings::Settings::state_dir();
    let log_dir = state_dir.join("logs");
    let file = std::fs::create_dir_all(&log_dir)
        .ok()
        .and_then(|_| {
            let name = format!("gittrellis-{}.log", chrono::Local::now().format("%Y-%m-%d"));
            let path = log_dir.join(name);
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()
                .map(|f| (f, path))
        });
    let mut guard = state().lock().unwrap();
    guard.level = level;
    if let Some((f, p)) = file {
        guard.file = Some(f);
        guard.file_path = Some(p);
    }
}

pub fn log_file_path() -> Option<PathBuf> {
    state().lock().unwrap().file_path.clone()
}

pub fn set_level(level: Level) {
    state().lock().unwrap().level = level;
}

pub fn log(level: Level, message: impl Into<String>) {
    let message = redact(&message.into());
    let record = LogRecord {
        time: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
        level,
        message,
    };
    let mut guard = state().lock().unwrap();
    if matches!(level, Level::Debug) && guard.level != Level::Debug {
        return;
    }
    let capacity = CAPACITY.load(Ordering::Relaxed);
    if guard.records.len() >= capacity {
        guard.records.pop_front();
    }
    guard.records.push_back(record.clone());
    if let Some(file) = guard.file.as_mut() {
        let _ = writeln!(
            file,
            "{} [{}] {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            record.level.label(),
            record.message
        );
    }
    if guard.level == Level::Debug || !matches!(level, Level::Debug) {
        eprintln!("[{}] {}", record.level.label(), record.message);
    }
}

pub fn records() -> Vec<LogRecord> {
    state().lock().unwrap().records.iter().cloned().collect()
}

pub fn clear() {
    state().lock().unwrap().records.clear();
}

/// Export the in-memory log buffer as text.
pub fn export_text() -> String {
    records()
        .iter()
        .map(|r| format!("{} [{}] {}", r.time, r.level.label(), r.message))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Remove likely secrets from a log message.
///
/// This is defence in depth: the application never intentionally logs
/// credentials, but Git output or URLs may contain userinfo.
pub fn redact(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for token in split_keep_whitespace(input) {
        out.push_str(&redact_token(token));
    }
    out
}

fn split_keep_whitespace(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = s.as_bytes();
    let mut in_ws = None;
    for (i, &b) in bytes.iter().enumerate() {
        let is_ws = b.is_ascii_whitespace();
        match in_ws {
            None => in_ws = Some(is_ws),
            Some(prev) if prev != is_ws => {
                parts.push(&s[start..i]);
                start = i;
                in_ws = Some(is_ws);
            }
            _ => {}
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn redact_token(token: &str) -> String {
    // https://user:password@host/...  ->  https://***@host/...
    if let Some(scheme_end) = token.find("://") {
        let rest = &token[scheme_end + 3..];
        if let Some(at) = rest.find('@') {
            let userinfo = &rest[..at];
            if userinfo.contains(':') || !userinfo.is_empty() {
                return format!("{}://***@{}", &token[..scheme_end], &rest[at + 1..]);
            }
        }
    }
    let lower = token.to_ascii_lowercase();
    for marker in ["password=", "passwd=", "token=", "oauth=", "secret=", "authorization:"] {
        if let Some(pos) = lower.find(marker) {
            let keep = &token[..pos + marker.len()];
            return format!("{keep}***");
        }
    }
    token.to_string()
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::core::logging::log($crate::core::logging::Level::Info, format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::core::logging::log($crate::core::logging::Level::Warn, format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::core::logging::log($crate::core::logging::Level::Error, format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { $crate::core::logging::log($crate::core::logging::Level::Debug, format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_url_userinfo() {
        let r = redact("fetching https://alice:hunter2@example.com/repo.git now");
        assert!(!r.contains("hunter2"), "{r}");
        assert!(r.contains("https://***@example.com/repo.git"));
    }

    #[test]
    fn redacts_ssh_userinfo() {
        let r = redact("ssh://git:secretpw@host/path");
        assert!(!r.contains("secretpw"), "{r}");
    }

    #[test]
    fn redacts_token_params() {
        let r = redact("GET /x?access_token=abc123&x=1");
        assert!(!r.contains("abc123"), "{r}");
    }

    #[test]
    fn keeps_normal_text() {
        assert_eq!(redact("plain message"), "plain message");
        assert_eq!(redact("user@example.com"), "user@example.com");
    }

    #[test]
    fn buffer_is_bounded() {
        CAPACITY.store(4, Ordering::Relaxed);
        clear();
        for i in 0..10 {
            log(Level::Info, format!("message {i}"));
        }
        let records = records();
        assert!(records.len() <= 4);
        assert!(records.last().unwrap().message.contains("message 9"));
        CAPACITY.store(4000, Ordering::Relaxed);
    }
}

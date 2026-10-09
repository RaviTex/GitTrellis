//! Persistent application settings stored in the XDG config directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git::GitError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DiffViewMode {
    #[default]
    Unified,
    SideBySide,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentRepo {
    pub path: String,
    pub name: String,
    pub last_opened: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub recent_repos: Vec<RecentRepo>,
    pub open_repos: Vec<String>,
    pub window_width: i32,
    pub window_height: i32,
    pub window_maximized: bool,
    pub clone_default_dir: String,
    pub last_clone_url: String,
    pub fetch_prune: bool,
    pub diff_view_mode: DiffViewMode,
    pub diff_context_lines: u32,
    pub show_ignored_files: bool,
    pub commit_sign_off: bool,
    pub commit_gpg_sign: bool,
    pub log_level: String,
    pub recent_commit_messages: Vec<String>,
    /// User overrides for keyboard shortcuts; an empty value disables the
    /// shortcut, a missing entry means "use the default".
    pub shortcut_overrides: std::collections::HashMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        Self {
            recent_repos: Vec::new(),
            open_repos: Vec::new(),
            window_width: 1280,
            window_height: 800,
            window_maximized: false,
            clone_default_dir: format!("{home}/Projects"),
            last_clone_url: String::new(),
            fetch_prune: false,
            diff_view_mode: DiffViewMode::Unified,
            diff_context_lines: 3,
            show_ignored_files: false,
            commit_sign_off: false,
            commit_gpg_sign: false,
            log_level: "info".into(),
            recent_commit_messages: Vec::new(),
            shortcut_overrides: std::collections::HashMap::new(),
        }
    }
}

impl Settings {
    pub fn config_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("GITTRELLIS_CONFIG_DIR") {
            return PathBuf::from(dir);
        }
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return PathBuf::from(xdg).join("gittrellis");
            }
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".config").join("gittrellis")
    }

    pub fn state_dir() -> PathBuf {
        if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
            if !xdg.is_empty() {
                return PathBuf::from(xdg).join("gittrellis");
            }
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".local").join("state").join("gittrellis")
    }

    pub fn config_file() -> PathBuf {
        Self::config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::config_file())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), GitError> {
        self.save_to(&Self::config_file())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), GitError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| GitError::Parse {
                context: "settings",
                details: e.to_string(),
            })?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn touch_recent(&mut self, path: &Path, name: &str) {
        let path_str = path.to_string_lossy().to_string();
        self.recent_repos.retain(|r| r.path != path_str);
        self.recent_repos.insert(
            0,
            RecentRepo {
                path: path_str,
                name: name.to_string(),
                last_opened: chrono::Utc::now().timestamp(),
            },
        );
        self.recent_repos.truncate(15);
    }

    pub fn remove_recent(&mut self, path: &str) {
        self.recent_repos.retain(|r| r.path != path);
    }

    pub fn add_open_repo(&mut self, path: &Path) {
        let s = path.to_string_lossy().to_string();
        if !self.open_repos.contains(&s) {
            self.open_repos.push(s);
        }
    }

    pub fn remove_open_repo(&mut self, path: &str) {
        self.open_repos.retain(|p| p != path);
    }

    pub fn remember_commit_message(&mut self, message: &str) {
        let first = message.lines().next().unwrap_or("").trim().to_string();
        if first.is_empty() {
            return;
        }
        self.recent_commit_messages.retain(|m| m != &first);
        self.recent_commit_messages.insert(0, first);
        self.recent_commit_messages.truncate(20);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut s = Settings::default();
        s.fetch_prune = true;
        s.window_width = 999;
        s.touch_recent(Path::new("/tmp/repo"), "repo");
        s.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path);
        assert!(loaded.fetch_prune);
        assert_eq!(loaded.window_width, 999);
        assert_eq!(loaded.recent_repos.len(), 1);
        assert_eq!(loaded.recent_repos[0].name, "repo");
    }

    #[test]
    fn recent_dedupes_and_orders() {
        let mut s = Settings::default();
        s.touch_recent(Path::new("/a"), "a");
        s.touch_recent(Path::new("/b"), "b");
        s.touch_recent(Path::new("/a"), "a");
        assert_eq!(s.recent_repos.len(), 2);
        assert_eq!(s.recent_repos[0].path, "/a");
    }

    #[test]
    fn commit_history_dedupes() {
        let mut s = Settings::default();
        s.remember_commit_message("fix: a\n\nbody");
        s.remember_commit_message("feat: b");
        s.remember_commit_message("fix: a");
        assert_eq!(s.recent_commit_messages, vec!["fix: a", "feat: b"]);
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::load_from(&dir.path().join("nope.json"));
        assert_eq!(s.window_width, 1280);
    }
}

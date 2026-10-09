//! Shared helpers for integration tests.
//!
//! Every test runs against real `git` in a temporary repository with the
//! system and global configuration disabled, so results never depend on the
//! developer's personal Git setup.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Once;

use gittrellis::git::{ConfigScope, Repository};

static ISOLATE: Once = Once::new();

/// Make the whole test process independent from user configuration.
pub fn isolate() {
    ISOLATE.call_once(|| {
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_TERMINAL_PROMPT", "0");
        std::env::set_var("GIT_AUTHOR_DATE", "1704067200 +0000");
        std::env::set_var("GIT_COMMITTER_DATE", "1704067200 +0000");
        let home = std::env::temp_dir().join("gittrellis-test-home");
        let _ = std::fs::create_dir_all(home.join(".config"));
        let _ = std::fs::create_dir_all(home.join(".state"));
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        std::env::set_var("XDG_STATE_HOME", home.join(".state"));
    });
}

pub struct TestRepo {
    pub dir: tempfile::TempDir,
    pub repo: Repository,
}

impl TestRepo {
    pub fn init() -> Self {
        isolate();
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path(), Some("main")).expect("init repo");
        for (k, v) in [
            ("user.name", "Test User"),
            ("user.email", "test@example.invalid"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("init.defaultBranch", "main"),
        ] {
            repo.config_set(k, v, ConfigScope::Local).expect("config");
        }
        Self { dir, repo }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.dir.path().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.dir.path().join(rel)).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(rel)).unwrap()
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.dir.path().join(rel).exists()
    }

    /// Run a raw git command in the repository and assert success.
    pub fn git(&self, args: &[&str]) -> String {
        self.repo
            .command()
            .args(args)
            .run_string()
            .unwrap_or_else(|e| panic!("git {args:?} failed: {e}\n{}", e.technical_details()))
    }

    /// Write a file, stage it and commit it. Returns the commit id.
    pub fn commit_file(&self, rel: &str, content: &str, message: &str) -> String {
        self.write(rel, content);
        self.git(&["add", "--", rel]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    pub fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    pub fn stage(&self, rel: &str) {
        self.git(&["add", "--", rel]);
    }
}

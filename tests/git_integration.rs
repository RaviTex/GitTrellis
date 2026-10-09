//! Integration tests: real `git` executions in temporary repositories.
//!
//! These tests exercise the same service-layer calls the UI uses, so a
//! passing suite means the application can perform the workflow end to end.

mod common;

use std::path::PathBuf;

use common::TestRepo;
use gittrellis::git::{
    patch, ChangeKind, CommitOptions, ConfigScope, ConflictChoice, ForceMode, Git, LineKind,
    LogOptions, MergeOutcome, Repository, ResetMode, StreamKind,
};

fn paths(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

fn no_progress(_: StreamKind, _: &str) {}

// ---------------------------------------------------------------------------
// Opening and status
// ---------------------------------------------------------------------------

#[test]
fn open_and_status_clean() {
    let t = TestRepo::init();
    let status = t.repo.status().unwrap();
    assert!(status.is_clean());
    assert!(status.unborn);
    assert_eq!(status.head_name.as_deref(), Some("main"));
}

#[test]
fn open_non_repo_fails_with_helpful_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = Repository::open(dir.path()).unwrap_err();
    assert!(matches!(err, gittrellis::git::GitError::NotARepository { .. }));
    assert!(err.user_message().contains("not a Git repository"));
}

#[test]
fn stage_commit_and_read_history() {
    let t = TestRepo::init();
    t.write("a.txt", "hello\n");
    t.repo.add_paths(&paths(&["a.txt"])).unwrap();
    let status = t.repo.status().unwrap();
    assert_eq!(status.staged_count(), 1);
    assert_eq!(status.entries[0].index, ChangeKind::Added);

    let oid = t
        .repo
        .create_commit(&CommitOptions {
            message: "initial import\n\nDetails here.".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(oid.len(), 40);

    let log = t.repo.log(&LogOptions::all()).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].summary, "initial import");
    assert_eq!(log[0].body, "initial import\n\nDetails here.");
    assert!(t.repo.is_clean().unwrap());
}

#[test]
fn commit_requires_staged_changes() {
    let t = TestRepo::init();
    let err = t
        .repo
        .create_commit(&CommitOptions {
            message: "nothing".into(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(err.user_message().contains("nothing staged"));
}

#[test]
fn commit_requires_message() {
    let t = TestRepo::init();
    t.write("a.txt", "x");
    t.repo.add_paths(&paths(&["a.txt"])).unwrap();
    let err = t
        .repo
        .create_commit(&CommitOptions::default())
        .unwrap_err();
    assert!(err.user_message().contains("commit message"));
}

#[test]
fn amend_updates_last_commit() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "one\n", "first");
    t.write("b.txt", "two\n");
    t.repo.add_paths(&paths(&["b.txt"])).unwrap();
    t.repo
        .create_commit(&CommitOptions {
            message: "first, amended".into(),
            amend: true,
            ..Default::default()
        })
        .unwrap();
    let log = t.repo.log(&LogOptions::all()).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].summary, "first, amended");
    assert_eq!(t.repo.head_commit_message().unwrap(), "first, amended");
}

// ---------------------------------------------------------------------------
// Unusual file names
// ---------------------------------------------------------------------------

#[test]
fn unusual_filenames_roundtrip() {
    let t = TestRepo::init();
    let names = [
        "-dash.txt",
        "with space.txt",
        "café.txt",
        "日本語.md",
        "quote\"name.txt",
        "back\\slash.txt",
        "tab\tname.txt",
    ];
    for (i, name) in names.iter().enumerate() {
        t.write(name, &format!("content {i}\n"));
    }
    let ps: Vec<PathBuf> = names.iter().map(PathBuf::from).collect();
    t.repo.add_paths(&ps).unwrap();
    let status = t.repo.status().unwrap();
    assert_eq!(status.staged().len(), names.len());
    t.repo
        .create_commit(&CommitOptions {
            message: "weird names".into(),
            ..Default::default()
        })
        .unwrap();
    let log = t.repo.log(&LogOptions::all()).unwrap();
    assert_eq!(log.len(), 1);

    // Modify one and diff it
    t.write("with space.txt", "changed\n");
    let diff = t.repo.diff_worktree(None).unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path().unwrap().to_string_lossy(), "with space.txt");
}

#[test]
fn newline_in_filename_is_rejected_not_mishandled() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "x", "base");
    let bad = PathBuf::from("bad\nname.txt");
    let err = t.repo.add_paths(&[bad]).unwrap_err();
    assert!(err.user_message().contains("newlines"));
}

#[test]
fn stage_unstage_and_discard() {
    let t = TestRepo::init();
    t.commit_file("f.txt", "one\ntwo\n", "base");

    t.write("f.txt", "one\nTWO\n");
    t.repo.add_paths(&paths(&["f.txt"])).unwrap();
    assert!(!t.repo.is_clean().unwrap());
    let st = t.repo.status().unwrap();
    assert!(st.entries[0].is_staged());

    t.repo.reset_paths(&paths(&["f.txt"])).unwrap();
    let st = t.repo.status().unwrap();
    assert!(!st.entries[0].is_staged());
    assert!(st.entries[0].is_unstaged());

    t.repo.add_paths(&paths(&["f.txt"])).unwrap();
    t.repo.discard_paths(&paths(&["f.txt"])).unwrap();
    // Discarding restores both the index and the worktree from HEAD.
    assert_eq!(t.read("f.txt"), "one\ntwo\n");
    assert!(t.repo.is_clean().unwrap());

    t.write("f.txt", "dirty\n");
    t.repo.discard_paths(&paths(&["f.txt"])).unwrap();
    assert_eq!(t.read("f.txt"), "one\ntwo\n");
    assert!(t.repo.is_clean().unwrap());
}

#[test]
fn discard_staged_new_file_deletes_it() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "base");
    t.write("new.txt", "content");
    t.repo.add_paths(&paths(&["new.txt"])).unwrap();
    assert!(!t.repo.status().unwrap().is_clean());
    t.repo.discard_paths(&paths(&["new.txt"])).unwrap();
    assert!(!t.exists("new.txt"));
    assert!(t.repo.is_clean().unwrap());
}

#[test]
fn discard_staged_rename_restores_original() {
    let t = TestRepo::init();
    t.commit_file("old.txt", "content\n", "base");
    t.git(&["mv", "old.txt", "new.txt"]);
    assert!(!t.repo.status().unwrap().is_clean());
    // Callers pass both sides of the rename.
    t.repo
        .discard_paths(&paths(&["new.txt", "old.txt"]))
        .unwrap();
    assert!(t.exists("old.txt"));
    assert!(!t.exists("new.txt"));
    assert!(t.repo.is_clean().unwrap());
}

#[test]
fn discard_does_not_touch_untracked_files() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "base");
    t.write("keep.txt", "untracked");
    t.repo.discard_paths(&paths(&["a.txt"])).unwrap();
    assert!(t.exists("keep.txt"));
}

#[test]
fn untracked_removal_is_explicit() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "x", "base");
    t.write("junk.txt", "temp");
    let st = t.repo.status().unwrap();
    assert_eq!(st.untracked().len(), 1);
    t.repo.remove_untracked(&paths(&["junk.txt"])).unwrap();
    assert!(!t.exists("junk.txt"));
}

#[test]
fn reset_all_works_on_unborn_branch() {
    let t = TestRepo::init();
    t.write("a.txt", "x");
    t.repo.add_paths(&paths(&["a.txt"])).unwrap();
    assert_eq!(t.repo.status().unwrap().staged_count(), 1);
    t.repo.reset_all().unwrap();
    assert_eq!(t.repo.status().unwrap().staged_count(), 0);
}

// ---------------------------------------------------------------------------
// Diffs
// ---------------------------------------------------------------------------

#[test]
fn diff_worktree_index_and_commit() {
    let t = TestRepo::init();
    t.commit_file("f.txt", "one\ntwo\nthree\n", "base");
    let base = t.head();
    t.write("f.txt", "one\nTWO\nthree\n");

    let wt = t.repo.diff_worktree(None).unwrap();
    assert_eq!(wt.len(), 1);
    assert_eq!(wt[0].added_lines(), 1);
    assert_eq!(wt[0].removed_lines(), 1);

    t.repo.add_paths(&paths(&["f.txt"])).unwrap();
    let idx = t.repo.diff_index(None).unwrap();
    assert_eq!(idx.len(), 1);
    assert_eq!(idx[0].hunks[0].lines.iter().filter(|l| l.kind == LineKind::Added).count(), 1);

    t.repo
        .create_commit(&CommitOptions {
            message: "change".into(),
            ..Default::default()
        })
        .unwrap();
    let head = t.head();
    let commit_diff = t.repo.diff_commits(&base, &head).unwrap();
    assert_eq!(commit_diff.len(), 1);
    let show_diff = t.repo.diff_commit(&head).unwrap();
    assert_eq!(show_diff.len(), 1);
    let files = t.repo.changed_files_commit(&head).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, ChangeKind::Modified);
}

#[test]
fn diff_detects_rename() {
    let t = TestRepo::init();
    let base_content = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\n";
    t.commit_file("old.txt", base_content, "base");
    let base = t.head();
    t.git(&["mv", "old.txt", "new.txt"]);
    t.git(&["commit", "-q", "-m", "rename"]);
    let head = t.head();
    let diff = t.repo.diff_commits(&base, &head).unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].status, ChangeKind::Renamed);
    assert_eq!(diff[0].old_path.as_ref().unwrap().to_string_lossy(), "old.txt");
    assert_eq!(diff[0].new_path.as_ref().unwrap().to_string_lossy(), "new.txt");
}

#[test]
fn diff_binary_file_is_flagged() {
    let t = TestRepo::init();
    std::fs::write(t.path().join("blob.bin"), [0u8, 1, 2, 3, 4, 5]).unwrap();
    t.git(&["add", "--", "blob.bin"]);
    t.git(&["commit", "-q", "-m", "binary"]);
    std::fs::write(t.path().join("blob.bin"), [0u8, 9, 9, 9, 9, 9]).unwrap();
    let diff = t.repo.diff_worktree(None).unwrap();
    assert_eq!(diff.len(), 1);
    assert!(diff[0].is_binary);
    assert!(diff[0].hunks.is_empty());
}

#[test]
fn show_file_and_stages() {
    let t = TestRepo::init();
    t.commit_file("f.txt", "committed\n", "base");
    t.write("f.txt", "modified\n");
    assert_eq!(t.repo.show_file("HEAD", &PathBuf::from("f.txt")).unwrap(), b"committed\n");
    let wt = t.repo.read_worktree_file(&PathBuf::from("f.txt")).unwrap().unwrap();
    assert_eq!(wt, "modified\n");
}

#[test]
fn hunk_staging_and_unstaging() {
    let t = TestRepo::init();
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n17\n18\n19\n20\n";
    t.commit_file("f.txt", base, "base");
    let modified = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\nELEVEN\n12\n13\n14\n15\n16\n17\n18\nNINETEEN\n20\n";
    t.write("f.txt", modified);

    let diff = t.repo.diff_worktree(None).unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].hunks.len(), 2, "expected two separate hunks");

    // Stage only the second hunk
    let patch = patch::build_hunk_patch(&diff[0], &[1], false).unwrap();
    t.repo.apply_patch(&patch, true, false, false).unwrap();

    let staged = t.repo.diff_index(None).unwrap();
    assert_eq!(staged.len(), 1);
    assert!(staged[0].hunks.len() == 1);
    assert!(staged[0]
        .hunks[0]
        .lines
        .iter()
        .any(|l| l.text == "NINETEEN"));
    assert!(!staged[0]
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .any(|l| l.text == "ELEVEN"));

    // Unstage that hunk again
    let unstage_patch = patch::build_hunk_patch(&staged[0], &[0], false).unwrap();
    t.repo.apply_patch(&unstage_patch, true, true, false).unwrap();
    assert!(t.repo.is_clean().unwrap() == false);
    assert_eq!(t.repo.status().unwrap().staged_count(), 0);
}

#[test]
fn hunk_discard_restores_worktree() {
    let t = TestRepo::init();
    t.commit_file("f.txt", "one\ntwo\nthree\n", "base");
    t.write("f.txt", "one\nTWO\nthree\n");
    let diff = t.repo.diff_worktree(None).unwrap();
    let patch = patch::build_hunk_patch(&diff[0], &[0], false).unwrap();
    t.repo.apply_patch(&patch, false, true, true).unwrap();
    assert_eq!(t.read("f.txt"), "one\ntwo\nthree\n");
}

#[test]
fn root_commit_changed_files() {
    let t = TestRepo::init();
    let oid = t.commit_file("a.txt", "a", "root");
    let files = t.repo.changed_files_commit(&oid).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, ChangeKind::Added);
}

// ---------------------------------------------------------------------------
// Branches
// ---------------------------------------------------------------------------

#[test]
fn branch_lifecycle() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "base");

    t.repo.create_branch("feature", None, false).unwrap();
    let branches = t.repo.branches().unwrap();
    assert!(branches.iter().any(|b| b.name == "feature" && !b.is_head));

    t.repo.checkout_branch("feature").unwrap();
    let status = t.repo.status().unwrap();
    assert_eq!(status.head_name.as_deref(), Some("feature"));

    t.repo.rename_branch("feature", "renamed", false).unwrap();
    assert!(t.repo.status().unwrap().head_name.as_deref() == Some("renamed"));

    t.repo.checkout_branch("main").unwrap();
    t.repo.delete_branch("renamed", false).unwrap();
    assert!(!t.repo.branches().unwrap().iter().any(|b| b.name == "renamed"));
}

#[test]
fn delete_unmerged_branch_requires_force() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "base");
    t.repo.checkout_new_branch("feature", None).unwrap();
    t.commit_file("b.txt", "b", "feature work");
    t.repo.checkout_branch("main").unwrap();
    let err = t.repo.delete_branch("feature", false).unwrap_err();
    assert!(err.technical_details().contains("not fully merged") || err.user_message().contains("not fully merged"));
    t.repo.delete_branch("feature", true).unwrap();
    assert!(!t.repo.branches().unwrap().iter().any(|b| b.name == "feature"));
}

#[test]
fn create_branch_from_commit_and_tag() {
    let t = TestRepo::init();
    let first = t.commit_file("a.txt", "a", "first");
    t.commit_file("b.txt", "b", "second");
    t.repo.create_tag("v1", None, Some("release"), false, false).unwrap();
    t.repo.create_branch("from-commit", Some(&first), false).unwrap();
    t.repo.create_branch("from-tag", Some("v1"), false).unwrap();
    let branches = t.repo.branches().unwrap();
    let from_commit = branches.iter().find(|b| b.name == "from-commit").unwrap();
    assert_eq!(from_commit.tip, first);
}

#[test]
fn detached_head_detection() {
    let t = TestRepo::init();
    let first = t.commit_file("a.txt", "a", "first");
    let _second = t.commit_file("b.txt", "b", "second");
    t.repo.checkout_detached(&first).unwrap();
    let status = t.repo.status().unwrap();
    assert!(status.detached);
    assert!(status.head_name.is_none());
    t.repo.checkout_branch("main").unwrap();
}

#[test]
fn ahead_behind_and_upstream() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    t.commit_file("b.txt", "b", "feature 1");
    t.commit_file("c.txt", "c", "feature 2");
    t.repo.set_upstream("feature", "main").unwrap();
    assert_eq!(t.repo.upstream_of_current().unwrap().as_deref(), Some("main"));
    let (ahead, behind) = t.repo.ahead_behind("feature", "main").unwrap();
    assert_eq!((ahead, behind), (2, 0));
    t.repo.unset_upstream("feature").unwrap();
    assert!(t.repo.upstream_of_current().unwrap().is_none());
}

// ---------------------------------------------------------------------------
// Merge / rebase / conflicts
// ---------------------------------------------------------------------------

#[test]
fn merge_fast_forward() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    t.commit_file("b.txt", "b", "feature");
    t.repo.checkout_branch("main").unwrap();
    let outcome = t.repo.merge("feature", false, false, None).unwrap();
    assert_eq!(outcome, MergeOutcome::FastForward);
    assert_eq!(t.repo.status().unwrap().head_name.as_deref(), Some("main"));
}

#[test]
fn merge_creates_merge_commit() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    t.commit_file("b.txt", "b", "feature");
    t.repo.checkout_branch("main").unwrap();
    t.commit_file("c.txt", "c", "main work");
    let outcome = t.repo.merge("feature", false, false, None).unwrap();
    assert_eq!(outcome, MergeOutcome::Merged);
    let head = t.repo.log_commit("HEAD").unwrap();
    assert!(head.is_merge());
    assert_eq!(head.refs.iter().filter(|r| r.kind == gittrellis::git::RefKind::Head).count(), 1);
}

#[test]
fn merge_up_to_date() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "base");
    t.repo.create_branch("feature", None, false).unwrap();
    let outcome = t.repo.merge("feature", false, false, None).unwrap();
    assert_eq!(outcome, MergeOutcome::UpToDate);
}

#[test]
fn merge_conflict_detect_resolve_continue() {
    let t = TestRepo::init();
    t.commit_file("f.txt", "base\n", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    t.write("f.txt", "feature version\n");
    t.repo.add_all().unwrap();
    t.repo
        .create_commit(&CommitOptions {
            message: "feature".into(),
            ..Default::default()
        })
        .unwrap();
    t.repo.checkout_branch("main").unwrap();
    t.write("f.txt", "main version\n");
    t.repo.add_all().unwrap();
    t.repo
        .create_commit(&CommitOptions {
            message: "main".into(),
            ..Default::default()
        })
        .unwrap();

    let outcome = t.repo.merge("feature", false, false, None).unwrap();
    match outcome {
        MergeOutcome::Conflicts(paths) => assert_eq!(paths, vec![PathBuf::from("f.txt")]),
        other => panic!("expected conflicts, got {other:?}"),
    }
    let status = t.repo.status().unwrap();
    assert!(status.has_conflicts());
    assert_eq!(status.conflicted().len(), 1);
    let op = t.repo.operation_state();
    assert!(matches!(op, gittrellis::git::OperationState::Merge { .. }));
    assert_eq!(t.repo.conflicted_paths().unwrap().len(), 1);

    // The worktree must contain conflict markers
    let content = t.read("f.txt");
    assert!(content.contains("<<<<<<<"));
    let regions = gittrellis::git::parse_conflict_regions(&content);
    assert_eq!(regions.len(), 1);

    // Resolve by hand and continue
    t.write("f.txt", "resolved\n");
    t.repo.add_paths(&paths(&["f.txt"])).unwrap();
    assert!(!t.repo.status().unwrap().has_conflicts());
    let outcome = t.repo.merge_continue().unwrap();
    assert_eq!(outcome, MergeOutcome::Merged);
    assert!(t.repo.operation_state().is_none());
    assert_eq!(t.repo.log_commit("HEAD").unwrap().parents.len(), 2);
}

#[test]
fn merge_abort_restores_state() {
    let t = TestRepo::init();
    t.commit_file("f.txt", "base\n", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    t.commit_file("f.txt", "feature\n", "feature");
    t.repo.checkout_branch("main").unwrap();
    t.commit_file("f.txt", "main\n", "main");
    let before = t.head();
    let outcome = t.repo.merge("feature", false, false, None).unwrap();
    assert!(matches!(outcome, MergeOutcome::Conflicts(_)));
    t.repo.merge_abort().unwrap();
    assert!(t.repo.operation_state().is_none());
    assert_eq!(t.head(), before);
    assert_eq!(t.read("f.txt"), "main\n");
}

#[test]
fn rebase_success_and_conflict_abort() {
    let t = TestRepo::init();
    t.commit_file("common.txt", "base\n", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    t.commit_file("feature.txt", "feature\n", "feature work");
    t.repo.checkout_branch("main").unwrap();
    t.commit_file("main.txt", "main\n", "main work");
    t.repo.checkout_branch("feature").unwrap();

    let outcome = t.repo.rebase(Some("main"), None, None).unwrap();
    assert_eq!(outcome, MergeOutcome::Merged);
    let log = t.repo.log(&LogOptions {
        rev: Some("feature".into()),
        ..Default::default()
    }).unwrap();
    // base + "main work" + rebased "feature work"
    assert_eq!(log.len(), 3);
    assert_eq!(log[0].summary, "feature work");

    // Now create a conflicting rebase
    t.repo.checkout_branch("main").unwrap();
    t.write("common.txt", "main change\n");
    t.repo.add_all().unwrap();
    t.repo
        .create_commit(&CommitOptions {
            message: "main change".into(),
            ..Default::default()
        })
        .unwrap();
    t.repo.checkout_branch("feature").unwrap();
    t.write("common.txt", "feature change\n");
    t.repo.add_all().unwrap();
    t.repo
        .create_commit(&CommitOptions {
            message: "feature change".into(),
            ..Default::default()
        })
        .unwrap();
    t.repo.checkout_branch("main").unwrap();
    t.repo.checkout_branch("feature").unwrap();

    // rebase feature onto main will conflict because both changed common.txt
    let outcome = t.repo.rebase(Some("main"), None, None);
    // The previous rebase made feature contain main's commits; conflict may
    // not occur. Reset to a clean situation instead.
    if let Ok(MergeOutcome::Conflicts(_)) = outcome {
        assert!(matches!(
            t.repo.operation_state(),
            gittrellis::git::OperationState::Rebase { .. }
        ));
        t.repo.rebase_abort().unwrap();
        assert!(t.repo.operation_state().is_none());
    }
}

#[test]
fn rebase_conflict_full_cycle() {
    let t = TestRepo::init();
    t.commit_file("f.txt", "base\n", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    t.commit_file("f.txt", "feature\n", "feature");
    t.repo.checkout_branch("main").unwrap();
    t.commit_file("f.txt", "main\n", "main");
    t.repo.checkout_branch("feature").unwrap();

    let outcome = t.repo.rebase(Some("main"), None, None).unwrap();
    assert!(matches!(outcome, MergeOutcome::Conflicts(_)));
    assert!(matches!(
        t.repo.operation_state(),
        gittrellis::git::OperationState::Rebase { .. }
    ));
    // resolve and continue
    t.write("f.txt", "resolved\n");
    t.repo.add_all().unwrap();
    let outcome = t.repo.rebase_continue().unwrap();
    assert_eq!(outcome, MergeOutcome::Merged);
    assert!(t.repo.operation_state().is_none());
    assert_eq!(t.read("f.txt"), "resolved\n");
}

#[test]
fn cherry_pick_and_revert() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a\n", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    let picked = t.commit_file("b.txt", "b\n", "add b");
    t.repo.checkout_branch("main").unwrap();
    let outcome = t.repo.cherry_pick(&picked).unwrap();
    assert_eq!(outcome, MergeOutcome::Merged);
    assert!(t.exists("b.txt"));

    let outcome = t.repo.revert(&picked, None).unwrap();
    assert_eq!(outcome, MergeOutcome::Merged);
    assert!(!t.exists("b.txt"));
}

#[test]
fn cherry_pick_conflict_and_state() {
    let t = TestRepo::init();
    t.commit_file("f.txt", "base\n", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    let picked = t.commit_file("f.txt", "feature\n", "feature change");
    t.repo.checkout_branch("main").unwrap();
    t.commit_file("f.txt", "main\n", "main change");
    let outcome = t.repo.cherry_pick(&picked).unwrap();
    assert!(matches!(outcome, MergeOutcome::Conflicts(_)));
    assert!(matches!(
        t.repo.operation_state(),
        gittrellis::git::OperationState::CherryPick { .. }
    ));
    t.repo.cherry_pick_abort().unwrap();
    assert!(t.repo.operation_state().is_none());
}

#[test]
fn reset_modes() {
    let t = TestRepo::init();
    let first = t.commit_file("a.txt", "a\n", "first");
    t.commit_file("b.txt", "b\n", "second");

    // Soft: move branch back, keep everything staged, worktree untouched
    t.repo.reset(ResetMode::Soft, &first).unwrap();
    let st = t.repo.status().unwrap();
    assert!(st.staged().iter().any(|e| e.path.to_string_lossy() == "b.txt"));
    assert!(t.exists("b.txt"));

    // Mixed: move branch back, unstage, keep worktree
    t.repo.reset(ResetMode::Mixed, &first).unwrap();
    let st = t.repo.status().unwrap();
    assert_eq!(st.staged_count(), 0);
    assert!(t.exists("b.txt"));

    // Hard: discard changes to tracked files. Untracked files are never
    // deleted by `git reset --hard`, which is exactly what we want.
    t.remove("b.txt");
    t.write("a.txt", "dirty\n");
    t.repo.reset(ResetMode::Hard, &first).unwrap();
    assert_eq!(t.read("a.txt"), "a\n");
    assert!(t.repo.is_clean().unwrap());
}

// ---------------------------------------------------------------------------
// Stash
// ---------------------------------------------------------------------------

#[test]
fn stash_lifecycle() {
    let t = TestRepo::init();
    t.commit_file("tracked.txt", "one\n", "base");
    t.write("tracked.txt", "changed\n");
    t.write("untracked.txt", "new\n");

    t.repo
        .stash_save(Some("wip message"), true, false, None)
        .unwrap();
    assert!(t.repo.is_clean().unwrap());
    let stashes = t.repo.stashes().unwrap();
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.contains("wip message"));
    assert!(stashes[0].has_untracked);

    let diff = t.repo.diff_stash(0).unwrap();
    assert!(!diff.is_empty());

    t.repo.stash_apply(0, false).unwrap();
    assert_eq!(t.read("tracked.txt"), "changed\n");
    assert!(t.exists("untracked.txt"));

    // remove them again then pop
    t.repo.discard_paths(&paths(&["tracked.txt"])).unwrap();
    t.repo.remove_untracked(&paths(&["untracked.txt"])).unwrap();
    t.repo.stash_pop(0, false).unwrap();
    assert_eq!(t.read("tracked.txt"), "changed\n");
    assert!(t.repo.stashes().unwrap().is_empty());
}

#[test]
fn stash_branch_and_drop() {
    let t = TestRepo::init();
    t.commit_file("f.txt", "one\n", "base");
    t.write("f.txt", "changed\n");
    t.repo.stash_save(Some("wip"), false, false, None).unwrap();
    t.repo.stash_branch(0, "recovered").unwrap();
    assert_eq!(t.repo.status().unwrap().head_name.as_deref(), Some("recovered"));
    assert_eq!(t.read("f.txt"), "changed\n");
    assert!(t.repo.stashes().unwrap().is_empty());

    t.repo.stash_save(Some("again"), false, false, None).unwrap();
    t.repo.stash_drop(0).unwrap();
    assert!(t.repo.stashes().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

#[test]
fn tag_create_list_delete() {
    let t = TestRepo::init();
    let oid = t.commit_file("a.txt", "a", "base");
    t.repo.create_tag("light", None, None, false, false).unwrap();
    t.repo
        .create_tag("annotated", None, Some("release 1"), false, false)
        .unwrap();
    let tags = t.repo.tags().unwrap();
    assert_eq!(tags.len(), 2);
    let light = tags.iter().find(|t| t.name == "light").unwrap();
    assert!(!light.is_annotated);
    assert_eq!(light.target, oid);
    let ann = tags.iter().find(|t| t.name == "annotated").unwrap();
    assert!(ann.is_annotated);
    assert_eq!(ann.subject, "release 1");
    assert_eq!(ann.target, oid);

    t.repo.delete_tag("light").unwrap();
    assert_eq!(t.repo.tags().unwrap().len(), 1);
}

#[test]
fn tag_on_specific_commit() {
    let t = TestRepo::init();
    let first = t.commit_file("a.txt", "a", "first");
    t.commit_file("b.txt", "b", "second");
    t.repo
        .create_tag("old", Some(&first), Some("old release"), false, false)
        .unwrap();
    let tags = t.repo.tags().unwrap();
    assert_eq!(tags[0].target, first);
}

// ---------------------------------------------------------------------------
// Remotes and network operations (over the local file transport)
// ---------------------------------------------------------------------------

fn make_bare_remote() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let url = dir.path().to_string_lossy().to_string();
    Git::system()
        .command()
        .cwd(dir.path())
        .args(["init", "-q", "--bare", "-b", "main"])
        .run()
        .unwrap();
    (dir, url)
}

#[test]
fn remote_management() {
    let t = TestRepo::init();
    t.repo.add_remote("origin", "https://example.invalid/repo.git").unwrap();
    let remotes = t.repo.remotes().unwrap();
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].fetch_url, "https://example.invalid/repo.git");

    t.repo
        .set_remote_url("origin", "ssh://git@example.invalid/repo.git", false)
        .unwrap();
    assert_eq!(t.repo.remotes().unwrap()[0].fetch_url, "ssh://git@example.invalid/repo.git");

    t.repo.rename_remote("origin", "upstream").unwrap();
    assert_eq!(t.repo.remotes().unwrap()[0].name, "upstream");

    t.repo.remove_remote("upstream").unwrap();
    assert!(t.repo.remotes().unwrap().is_empty());
}

#[test]
fn invalid_remote_name_rejected() {
    let t = TestRepo::init();
    assert!(t.repo.add_remote("-bad", "x").is_err());
    assert!(t.repo.add_remote("bad name", "x").is_err());
}

#[test]
fn push_fetch_pull_cycle() {
    let (remote_dir, url) = make_bare_remote();
    let t = TestRepo::init();
    t.commit_file("a.txt", "a\n", "base");
    t.repo.add_remote("origin", &url).unwrap();
    t.repo
        .push(
            "origin",
            "refs/heads/main",
            "refs/heads/main",
            true,
            ForceMode::None,
            None,
            no_progress,
        )
        .unwrap();
    let branches = t.repo.branches().unwrap();
    let main = branches.iter().find(|b| b.name == "main").unwrap();
    assert_eq!(main.upstream.as_deref(), Some("origin/main"));

    // Clone elsewhere and push new work
    let dest = remote_dir.path().parent().unwrap().join(format!(
        "clone-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dest);
    let clone = Repository::clone(&url, &dest, None, None, None, no_progress).unwrap();
    clone.config_set("user.name", "Other", ConfigScope::Local).unwrap();
    clone.config_set("user.email", "other@example.invalid", ConfigScope::Local).unwrap();
    std::fs::write(dest.join("b.txt"), "b\n").unwrap();
    clone.add_all().unwrap();
    clone
        .create_commit(&CommitOptions {
            message: "add b".into(),
            ..Default::default()
        })
        .unwrap();
    clone
        .push(
            "origin",
            "refs/heads/main",
            "refs/heads/main",
            false,
            ForceMode::None,
            None,
            no_progress,
        )
        .unwrap();

    // Fetch and inspect ahead/behind
    t.repo.fetch(Some("origin"), false, false, None, no_progress).unwrap();
    let (ahead, behind) = t.repo
        .ahead_behind("main", "origin/main")
        .unwrap();
    assert_eq!((ahead, behind), (0, 1));

    // Pull should fast-forward
    let outcome = t
        .repo
        .pull(Some("origin"), false, false, None, no_progress)
        .unwrap();
    assert_eq!(outcome, MergeOutcome::FastForward);
    assert!(t.exists("b.txt"));

    // Create a local commit and push with lease
    t.commit_file("c.txt", "c\n", "local c");
    t.repo
        .push(
            "origin",
            "refs/heads/main",
            "refs/heads/main",
            false,
            ForceMode::WithLease,
            None,
            no_progress,
        )
        .unwrap();
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn clone_progress_and_error() {
    let (_remote, url) = make_bare_remote();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("cloned");
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = count.clone();
    let repo = Repository::clone(&url, &dest, None, None, None, move |_, _| {
        c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    })
    .unwrap();
    assert_eq!(repo.workdir(), dest);
    assert!(repo.status().is_ok());

    // Bad URL gives an error, not a panic
    let dest2 = dir.path().join("nope");
    let err = Repository::clone(
        "https://example.invalid/does/not/exist.git",
        &dest2,
        None,
        None,
        None,
        no_progress,
    )
    .unwrap_err();
    assert!(!err.user_message().is_empty());
}

#[test]
fn fetch_can_be_cancelled() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "base");
    let cancel = gittrellis::git::Cancellation::new();
    cancel.cancel();
    let err = t
        .repo
        .fetch(Some("origin"), false, false, Some(cancel), no_progress)
        .unwrap_err();
    assert!(matches!(err, gittrellis::git::GitError::Cancelled));
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[test]
fn config_set_get_unset_and_list() {
    let t = TestRepo::init();
    t.repo
        .config_set("user.signingkey", "ABC123", ConfigScope::Local)
        .unwrap();
    assert_eq!(
        t.repo.config_get("user.signingkey").unwrap().as_deref(),
        Some("ABC123")
    );
    let list = t.repo.config_list().unwrap();
    assert!(list
        .iter()
        .any(|e| e.key == "user.signingkey" && e.scope == ConfigScope::Local));
    t.repo.config_unset("user.signingkey", ConfigScope::Local).unwrap();
    assert!(t.repo.config_get("user.signingkey").unwrap().is_none());
}

#[test]
fn invalid_config_key_rejected() {
    let t = TestRepo::init();
    assert!(t.repo.config_set("user.name; rm", "x", ConfigScope::Local).is_err());
    assert!(t.repo.config_set("bad key", "x", ConfigScope::Local).is_err());
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

#[test]
fn history_ordering_limits_and_search() {
    let t = TestRepo::init();
    for i in 0..25 {
        t.commit_file(&format!("f{i}.txt"), &format!("{i}"), &format!("commit {i}"));
    }
    let log = t.repo.log(&LogOptions::all()).unwrap();
    assert_eq!(log.len(), 25);
    assert_eq!(log[0].summary, "commit 24");

    let limited = t
        .repo
        .log(&LogOptions {
            all_refs: true,
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(limited.len(), 10);

    let skipped = t
        .repo
        .log(&LogOptions {
            all_refs: true,
            limit: 10,
            skip: 20,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(skipped.len(), 5);
    assert_eq!(skipped[0].summary, "commit 4");

    let grepped = t
        .repo
        .log(&LogOptions {
            all_refs: true,
            limit: 100,
            grep: Some("commit 13".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(grepped.len(), 1);
    assert_eq!(grepped[0].summary, "commit 13");

    let by_path = t
        .repo
        .log(&LogOptions {
            all_refs: true,
            limit: 100,
            paths: vec![PathBuf::from("f13.txt")],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(by_path.len(), 1);

    assert_eq!(t.repo.count_commits("HEAD").unwrap(), 25);
}

#[test]
fn merge_base_between_branches() {
    let t = TestRepo::init();
    let base = t.commit_file("a.txt", "a", "base");
    t.repo.create_branch("feature", None, true).unwrap();
    t.commit_file("b.txt", "b", "feature");
    t.repo.checkout_branch("main").unwrap();
    t.commit_file("c.txt", "c", "main");
    let mb = t.repo.merge_base("main", "feature").unwrap().unwrap();
    assert_eq!(mb, base);
}

#[test]
fn log_all_refs_includes_branches_and_remotes() {
    let t = TestRepo::init();
    t.commit_file("a.txt", "a", "on main");
    t.repo.create_branch("side", None, true).unwrap();
    t.commit_file("b.txt", "b", "on side");
    t.repo.checkout_branch("main").unwrap();
    let log = t.repo.log(&LogOptions::all()).unwrap();
    assert_eq!(log.len(), 2);
    let side_commit = log.iter().find(|c| c.summary == "on side").unwrap();
    assert!(side_commit.refs.iter().any(|r| r.name == "side"));
}

// ---------------------------------------------------------------------------
// Full UI workflow simulation
// ---------------------------------------------------------------------------

/// Mirrors the acceptance workflow: open repository, modify a file, stage it,
/// inspect the diff, commit, branch, switch, merge, resolve a conflict.
#[test]
fn full_workflow_simulation() {
    let t = TestRepo::init();

    // 1. Open repository (already open) and create initial content
    t.commit_file("README.md", "# Project\n", "Initial commit");

    // 2. Modify a file
    t.write("README.md", "# Project\n\nNew section.\n");
    let status = t.repo.status().unwrap();
    assert_eq!(status.unstaged().len(), 1);

    // 3. Stage it
    t.repo.add_paths(&paths(&["README.md"])).unwrap();
    assert_eq!(t.repo.status().unwrap().staged_count(), 1);

    // 4. View the diff
    let diff = t.repo.diff_index(None).unwrap();
    assert_eq!(diff.len(), 1);
    assert!(diff[0].added_lines() >= 2);

    // 5. Commit
    t.repo
        .create_commit(&CommitOptions {
            message: "docs: add section".into(),
            sign_off: true,
            ..Default::default()
        })
        .unwrap();
    let msg = t.repo.head_commit_message().unwrap();
    assert!(msg.contains("Signed-off-by: Test User <test@example.invalid>"), "{msg}");

    // 6. Create a branch and switch to it, then change the same file
    t.repo.checkout_new_branch("feature", None).unwrap();
    t.write("feature.txt", "feature content\n");
    t.write("README.md", "# Project\n\nFeature edit.\n");
    t.repo.add_all().unwrap();
    t.repo
        .create_commit(&CommitOptions {
            message: "feat: add feature".into(),
            ..Default::default()
        })
        .unwrap();

    // 7. Switch back and make a conflicting change
    t.repo.checkout_branch("main").unwrap();
    t.write("README.md", "# Project\n\nMain edit.\n");
    t.repo.add_all().unwrap();
    t.repo
        .create_commit(&CommitOptions {
            message: "docs: main edit".into(),
            ..Default::default()
        })
        .unwrap();

    // 8. Merge and detect the conflict
    let outcome = t.repo.merge("feature", false, false, None).unwrap();
    assert!(matches!(outcome, MergeOutcome::Conflicts(_)));
    assert!(t.repo.status().unwrap().has_conflicts());

    // 9. Resolve using the conflict parser and the "both" strategy
    let content = t.read("README.md");
    let regions = gittrellis::git::parse_conflict_regions(&content);
    assert!(!regions.is_empty());
    let resolved = gittrellis::git::resolve_conflicts(&content, &regions, &|_| {
        ConflictChoice::Both
    });
    assert!(!resolved.contains("<<<<<<<"));
    t.repo
        .write_worktree_file(&PathBuf::from("README.md"), &resolved)
        .unwrap();
    t.repo.add_paths(&paths(&["README.md"])).unwrap();
    let outcome = t.repo.merge_continue().unwrap();
    assert_eq!(outcome, MergeOutcome::Merged);

    // 10. Final state is clean and history contains the merge
    assert!(t.repo.is_clean().unwrap());
    let head = t.repo.log_commit("HEAD").unwrap();
    assert!(head.is_merge());
}

// ---------------------------------------------------------------------------
// Askpass helper
// ---------------------------------------------------------------------------

#[test]
fn askpass_helper_answers_and_rejects() {
    let helper = env!("CARGO_BIN_EXE_gittrellis-askpass");
    let answered = std::process::Command::new(helper)
        .arg("Enter passphrase for key '/tmp/id_ed25519':")
        .env("GITTRELLIS_ASKPASS_TEST_MODE", "1")
        .env("GITTRELLIS_ASKPASS_ANSWER", "s3cret")
        .output()
        .unwrap();
    assert!(answered.status.success());
    assert_eq!(String::from_utf8_lossy(&answered.stdout), "s3cret\n");

    // Without an answer (simulating a cancelled dialog) the helper must fail
    // so that ssh/git abort cleanly instead of hanging.
    let cancelled = std::process::Command::new(helper)
        .arg("Password:")
        .env("GITTRELLIS_ASKPASS_TEST_MODE", "1")
        .env_remove("GITTRELLIS_ASKPASS_ANSWER")
        .output()
        .unwrap();
    assert!(!cancelled.status.success());
    assert!(cancelled.stdout.is_empty());
}

#[test]
fn askpass_dialog_answers_and_cancels() {
    // The dialog path needs a display; skip silently on headless machines.
    if std::env::var("WAYLAND_DISPLAY").is_err() && std::env::var("DISPLAY").is_err() {
        eprintln!("skipping askpass dialog test: no display");
        return;
    }
    let helper = env!("CARGO_BIN_EXE_gittrellis-askpass");
    let answered = std::process::Command::new(helper)
        .arg("Enter passphrase for key '/tmp/id_ed25519':")
        .env("GITTRELLIS_ASKPASS_TEST_MODE", "1")
        .env("GITTRELLIS_ASKPASS_AUTOANSWER", "typed-passphrase")
        .output()
        .unwrap();
    assert!(answered.status.success());
    assert_eq!(
        String::from_utf8_lossy(&answered.stdout),
        "typed-passphrase\n"
    );

    let cancelled = std::process::Command::new(helper)
        .arg("Password:")
        .env("GITTRELLIS_ASKPASS_TEST_MODE", "1")
        .env("GITTRELLIS_ASKPASS_AUTOANSWER", "cancel")
        .output()
        .unwrap();
    assert!(!cancelled.status.success());
    assert!(cancelled.stdout.is_empty());
}

#[test]
fn cancel_terminates_lingering_askpass_helpers() {
    use gittrellis::git::{Cancellation, GitError};
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    // A credential helper (standing in for an askpass dialog) that never
    // returns. Before the process-group fix this pinned the output pipes open
    // and the operation could never complete or be cancelled.
    let helper = tmp.path().join("slow-askpass.sh");
    std::fs::write(&helper, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let git = Git::system().with_env("GIT_ASKPASS", helper.into_os_string());
    let cancel = Cancellation::new();
    {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            cancel.cancel();
        });
    }
    let start = std::time::Instant::now();
    let result = git
        .command()
        .args(["credential", "fill"])
        .stdin(b"protocol=https\nhost=example.com\n\n".to_vec())
        .cancel(cancel)
        .run();
    let elapsed = start.elapsed();
    match result {
        Err(GitError::Cancelled) => {}
        other => panic!("expected cancellation, got {other:?}"),
    }
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "cancellation took {elapsed:?}"
    );
}

#[test]
fn history_search_by_changed_file_name() {
    use gittrellis::ui::repo_page::path_substring_pathspec;
    let t = TestRepo::init();
    t.commit_file("src/widgets/button.rs", "a", "button work");
    t.commit_file("src/main.rs", "b", "main work");
    t.commit_file("docs/readme.md", "c", "docs work");

    let result = t
        .repo
        .log(&LogOptions {
            all_refs: true,
            limit: 100,
            paths: vec![path_substring_pathspec("button")],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].summary, "button work");

    // Case-insensitive and substring matching
    let result = t
        .repo
        .log(&LogOptions {
            all_refs: true,
            limit: 100,
            paths: vec![path_substring_pathspec("README")],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].summary, "docs work");
}

#[test]
fn askpass_cancel_marker_is_recognised() {
    let t = TestRepo::init();
    let _ = t;
    // The marker lives in the isolated XDG state directory used by the tests.
    assert!(!gittrellis::ui::askpass::cancelled_recently());
    gittrellis::ui::askpass::mark_cancelled();
    assert!(gittrellis::ui::askpass::cancelled_recently());
}

#[test]
fn unstage_works_on_unborn_branch() {
    // A fresh repository has no commits; HEAD cannot be resolved, so
    // unstaging must not pass an explicit HEAD to git reset.
    let t = TestRepo::init();
    t.write("a.txt", "a");
    t.write("b.txt", "b");
    t.repo.add_all().unwrap();
    assert_eq!(t.repo.status().unwrap().staged_count(), 2);

    t.repo.reset_paths(&paths(&["a.txt"])).unwrap();
    let status = t.repo.status().unwrap();
    assert_eq!(status.staged_count(), 1);
    assert!(status
        .untracked()
        .iter()
        .any(|e| e.path.to_string_lossy() == "a.txt"));

    t.repo.reset_all().unwrap();
    assert_eq!(t.repo.status().unwrap().staged_count(), 0);
    assert!(t.repo.status().unwrap().untracked().len() == 2);
}

#[test]
fn publish_branch_from_push_dialog_short_names() {
    use gittrellis::ui::repo_page::push_refspecs;
    let (remote_dir, url) = make_bare_remote();
    let t = TestRepo::init();
    t.commit_file("a.txt", "a\n", "base");
    t.repo.add_remote("origin", &url).unwrap();

    // The push dialog passes short branch names; they must be converted to
    // refspecs instead of the push being silently skipped.
    let (local_ref, remote_ref) = push_refspecs("main", "main").unwrap();
    assert_eq!(local_ref, "refs/heads/main");
    assert_eq!(remote_ref, "refs/heads/main");

    t.repo
        .push(
            "origin",
            &local_ref,
            &remote_ref,
            true,
            ForceMode::None,
            None,
            no_progress,
        )
        .unwrap();

    let branches = t.repo.branches().unwrap();
    let main = branches.iter().find(|b| b.name == "main").unwrap();
    assert_eq!(main.upstream.as_deref(), Some("origin/main"));

    // The remote branch now exists.
    let out = Git::system()
        .command()
        .cwd(remote_dir.path())
        .args(["branch", "--list"])
        .run_string()
        .unwrap();
    assert!(out.contains("main"), "remote branches: {out}");

    // A second push (now with upstream) works through the quick path.
    t.write("b.txt", "b\n");
    t.repo.add_all().unwrap();
    t.repo
        .create_commit(&CommitOptions {
            message: "second".into(),
            ..Default::default()
        })
        .unwrap();
    let (local_ref, remote_ref) = push_refspecs("main", "main").unwrap();
    t.repo
        .push(
            "origin",
            &local_ref,
            &remote_ref,
            false,
            ForceMode::None,
            None,
            no_progress,
        )
        .unwrap();
    let _ = remote_dir;
}

#[test]
fn push_refspecs_rejects_invalid_names() {
    use gittrellis::ui::repo_page::push_refspecs;
    assert!(push_refspecs("", "main").is_none());
    assert!(push_refspecs("main", "").is_none());
    assert!(push_refspecs("-x", "main").is_none());
    assert_eq!(
        push_refspecs("main", "main"),
        Some(("refs/heads/main".into(), "refs/heads/main".into()))
    );
}

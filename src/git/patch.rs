//! Patch construction for hunk-level staging, unstaging and discarding.
//!
//! The generated patches are fed to `git apply` through stdin, never to a
//! shell. Only whole hunks are selected here; line-level staging can be added
//! later without changing the interface.

use super::types::{ChangeKind, FileDiff, LineKind};

/// Build a unified diff for the selected hunks of a file.
///
/// * `hunk_indices` selects hunks by index; pass all indices to reproduce the
///   full file diff.
/// * `reverse` is not used to transform the patch (git apply handles
///   `--reverse`); it only documents intent in the function name for callers.
pub fn build_hunk_patch(file: &FileDiff, hunk_indices: &[usize], reverse: bool) -> Option<String> {
    let _ = reverse;
    let mut out = String::new();
    let old_path = file
        .old_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let new_path = file
        .new_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Paths that Git would C-quote cannot be safely round-tripped in a patch
    // header by this generator; callers fall back to whole-file operations.
    if requires_c_quoting(&old_path) || requires_c_quoting(&new_path) {
        return None;
    }

    let a = if old_path.is_empty() {
        "/dev/null".to_string()
    } else {
        format!("a/{}", marker_path(&old_path))
    };
    let b = if new_path.is_empty() {
        "/dev/null".to_string()
    } else {
        format!("b/{}", marker_path(&new_path))
    };

    let display_old = if old_path.is_empty() {
        new_path.clone()
    } else {
        old_path.clone()
    };
    let display_new = if new_path.is_empty() {
        old_path.clone()
    } else {
        new_path.clone()
    };
    out.push_str(&format!(
        "diff --git a/{} b/{}\n",
        marker_path(&display_old),
        marker_path(&display_new)
    ));
    match file.status {
        ChangeKind::Added => {
            out.push_str("new file mode 100644\n");
        }
        ChangeKind::Deleted => {
            out.push_str("deleted file mode 100644\n");
        }
        _ => {}
    }
    out.push_str(&format!("--- {a}\n"));
    out.push_str(&format!("+++ {b}\n"));

    let mut wrote_any = false;
    for &idx in hunk_indices {
        let Some(hunk) = file.hunks.get(idx) else {
            continue;
        };
        wrote_any = true;
        out.push_str(&hunk.header());
        out.push('\n');
        for line in &hunk.lines {
            match line.kind {
                LineKind::Context => {
                    out.push(' ');
                    out.push_str(&line.text);
                    out.push('\n');
                }
                LineKind::Added => {
                    out.push('+');
                    out.push_str(&line.text);
                    out.push('\n');
                }
                LineKind::Removed => {
                    out.push('-');
                    out.push_str(&line.text);
                    out.push('\n');
                }
                LineKind::NoNewline => {
                    out.push_str("\\ No newline at end of file\n");
                }
            }
        }
    }
    if wrote_any {
        Some(out)
    } else {
        None
    }
}

/// True when a path contains bytes Git would C-quote in diff headers.
fn requires_c_quoting(path: &str) -> bool {
    path.chars()
        .any(|c| c == '"' || c == '\\' || (c as u32) < 0x20 || c == '\u{7f}')
}

/// Append the tab separator Git uses when a path contains spaces.
fn marker_path(path: &str) -> String {
    if path.contains(' ') {
        format!("{path}\t")
    } else {
        path.to_string()
    }
}

/// Build a patch containing every hunk of the diff (whole file).
pub fn build_full_patch(file: &FileDiff) -> Option<String> {
    let indices: Vec<usize> = (0..file.hunks.len()).collect();
    if file.hunks.is_empty() {
        return None;
    }
    build_hunk_patch(file, &indices, false)
}

/// All hunks of every file in a diff, concatenated. Returns `None` when the
/// selection contains nothing that can be expressed as a text patch.
pub fn build_multi_file_patch(files: &[FileDiff], selection: &[Vec<usize>]) -> Option<String> {
    let mut out = String::new();
    for (file, hunks) in files.iter().zip(selection.iter()) {
        if hunks.is_empty() || file.is_binary {
            continue;
        }
        if let Some(p) = build_hunk_patch(file, hunks, false) {
            out.push_str(&p);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::parse::parse_diff;

    const SAMPLE: &str = "\
diff --git a/f.txt b/f.txt
index 1111111..2222222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,3 +1,3 @@
 one
-two
+TWO
 three
@@ -10,3 +10,4 @@
 ten
-eleven
+ELEVEN
+twelve
 thirteen
";

    #[test]
    fn first_hunk_only() {
        let files = parse_diff(SAMPLE.as_bytes()).unwrap();
        let patch = build_hunk_patch(&files[0], &[0], false).unwrap();
        assert!(patch.contains("-two"));
        assert!(patch.contains("+TWO"));
        assert!(!patch.contains("ELEVEN"));
        assert!(patch.contains("--- a/f.txt"));
        assert!(patch.contains("+++ b/f.txt"));
    }

    #[test]
    fn new_file_patch_has_dev_null() {
        let diff = "diff --git a/n.txt b/n.txt\nnew file mode 100644\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1 @@\n+hi\n";
        let files = parse_diff(diff.as_bytes()).unwrap();
        let patch = build_full_patch(&files[0]).unwrap();
        assert!(patch.contains("--- /dev/null"));
        assert!(patch.contains("+hi"));
    }

    #[test]
    fn no_hunks_yields_none() {
        let diff = "diff --git a/b.bin b/b.bin\nBinary files a/b.bin and b/b.bin differ\n";
        let files = parse_diff(diff.as_bytes()).unwrap();
        assert!(build_full_patch(&files[0]).is_none());
    }

    #[test]
    fn no_newline_marker_preserved() {
        let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n";
        let files = parse_diff(diff.as_bytes()).unwrap();
        let patch = build_full_patch(&files[0]).unwrap();
        assert!(patch.contains("\\ No newline at end of file"));
    }
}

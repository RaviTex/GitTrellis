//! Pure parsers for Git output. No process execution happens here, which
//! makes every function trivially unit testable with canned input.

use std::ffi::OsStr;
use std::path::PathBuf;

use super::error::{GitError, GitResult};
use super::types::*;

/// Convert raw path bytes to a PathBuf, preserving non-UTF8 bytes on Unix.
pub fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Decode a C-style quoted path as produced by `core.quotepath`.
pub fn unquote_path(s: &str) -> String {
    if !s.starts_with('"') || !s.ends_with('"') || s.len() < 2 {
        return s.to_string();
    }
    let inner = &s[1..s.len() - 1];
    let bytes = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let c = bytes[i + 1];
            match c {
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                }
                b'"' => {
                    out.push(b'"');
                    i += 2;
                }
                b'0'..=b'7' => {
                    let mut val: u32 = 0;
                    let mut n = 0;
                    let mut j = i + 1;
                    while n < 3 && j < bytes.len() && (b'0'..=b'7').contains(&bytes[j]) {
                        val = val * 8 + (bytes[j] - b'0') as u32;
                        j += 1;
                        n += 1;
                    }
                    out.push(val as u8);
                    i = j;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// git status --porcelain=v2 --branch -z
// ---------------------------------------------------------------------------

pub fn parse_status_v2(data: &[u8]) -> GitResult<RepoStatus> {
    let mut status = RepoStatus::default();
    let fields: Vec<&[u8]> = data.split(|&b| b == 0).collect();
    let mut i = 0usize;
    while i < fields.len() {
        let field = fields[i];
        i += 1;
        if field.is_empty() {
            continue;
        }
        match field[0] {
            b'#' => parse_status_header(field, &mut status)?,
            b'1' => status.entries.push(parse_ordinary(field)?),
            b'2' => {
                // rename/copy: path is in this field, original path is the
                // next NUL separated field.
                let orig = fields.get(i).copied().unwrap_or(b"");
                i += 1;
                status.entries.push(parse_renamed(field, orig)?);
            }
            b'u' => status.entries.push(parse_unmerged(field)?),
            b'?' => {
                let path = &field[2..];
                status.entries.push(StatusEntry {
                    path: path_from_bytes(path),
                    orig_path: None,
                    index: ChangeKind::Untracked,
                    worktree: ChangeKind::Untracked,
                    conflict: None,
                    submodule: String::new(),
                });
            }
            b'!' => {
                let path = &field[2..];
                status.entries.push(StatusEntry {
                    path: path_from_bytes(path),
                    orig_path: None,
                    index: ChangeKind::Ignored,
                    worktree: ChangeKind::Ignored,
                    conflict: None,
                    submodule: String::new(),
                });
            }
            _ => {}
        }
    }
    Ok(status)
}

fn parse_status_header(field: &[u8], status: &mut RepoStatus) -> GitResult<()> {
    let line = String::from_utf8_lossy(field);
    let line = line.trim_end();
    if let Some(rest) = line.strip_prefix("# branch.oid ") {
        if rest == "(initial)" {
            status.unborn = true;
            status.head_oid = None;
        } else {
            status.head_oid = Some(rest.to_string());
        }
    } else if let Some(rest) = line.strip_prefix("# branch.head ") {
        if rest == "(detached)" {
            status.detached = true;
        } else {
            status.head_name = Some(rest.to_string());
        }
    } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
        status.upstream = Some(rest.to_string());
    } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
        for token in rest.split_whitespace() {
            if let Some(n) = token.strip_prefix('+') {
                status.ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = token.strip_prefix('-') {
                status.behind = n.parse().unwrap_or(0);
            }
        }
    }
    Ok(())
}

fn parse_ordinary(field: &[u8]) -> GitResult<StatusEntry> {
    // 1 XY sub mH mI mW hH hI path
    let parts: Vec<&[u8]> = splitn_bytes(field, b' ', 9);
    if parts.len() < 9 {
        return Err(GitError::Parse {
            context: "status (ordinary entry)",
            details: String::from_utf8_lossy(field).into_owned(),
        });
    }
    let xy = parts[1];
    Ok(StatusEntry {
        path: path_from_bytes(parts[8]),
        orig_path: None,
        index: ChangeKind::from_code(xy.first().copied().unwrap_or(b'.')),
        worktree: ChangeKind::from_code(xy.get(1).copied().unwrap_or(b'.')),
        conflict: None,
        submodule: String::from_utf8_lossy(parts[2]).into_owned(),
    })
}

fn parse_renamed(field: &[u8], orig: &[u8]) -> GitResult<StatusEntry> {
    // 2 XY sub mH mI mW hH hI Xscore path
    let parts: Vec<&[u8]> = splitn_bytes(field, b' ', 10);
    if parts.len() < 10 {
        return Err(GitError::Parse {
            context: "status (rename entry)",
            details: String::from_utf8_lossy(field).into_owned(),
        });
    }
    let xy = parts[1];
    Ok(StatusEntry {
        path: path_from_bytes(parts[9]),
        orig_path: Some(path_from_bytes(orig)),
        index: ChangeKind::from_code(xy.first().copied().unwrap_or(b'.')),
        worktree: ChangeKind::from_code(xy.get(1).copied().unwrap_or(b'.')),
        conflict: None,
        submodule: String::from_utf8_lossy(parts[2]).into_owned(),
    })
}

fn parse_unmerged(field: &[u8]) -> GitResult<StatusEntry> {
    // u XY sub m1 m2 m3 mW h1 h2 h3 path
    let parts: Vec<&[u8]> = splitn_bytes(field, b' ', 11);
    if parts.len() < 11 {
        return Err(GitError::Parse {
            context: "status (unmerged entry)",
            details: String::from_utf8_lossy(field).into_owned(),
        });
    }
    let xy = parts[1];
    let x = xy.first().copied().unwrap_or(b'U');
    let y = xy.get(1).copied().unwrap_or(b'U');
    Ok(StatusEntry {
        path: path_from_bytes(parts[10]),
        orig_path: None,
        index: ChangeKind::Unmerged,
        worktree: ChangeKind::Unmerged,
        conflict: ConflictKind::from_xy(x, y).or(Some(ConflictKind::BothModified)),
        submodule: String::from_utf8_lossy(parts[2]).into_owned(),
    })
}

fn splitn_bytes(data: &[u8], sep: u8, n: usize) -> Vec<&[u8]> {
    let mut out = Vec::with_capacity(n);
    let mut start = 0usize;
    for (idx, &b) in data.iter().enumerate() {
        if b == sep && out.len() < n - 1 {
            out.push(&data[start..idx]);
            start = idx + 1;
        }
    }
    out.push(&data[start..]);
    out
}

// ---------------------------------------------------------------------------
// git log
// ---------------------------------------------------------------------------

pub const LOG_SEP: char = '\u{1f}';

/// Format string used with `git log -z`. `%B` (raw body) is deliberately the
/// last field so that a message containing the unit separator cannot corrupt
/// earlier fields.
pub const LOG_FORMAT: &str =
    "%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%s%x1f%D%x1f%B";

pub fn parse_log(data: &[u8]) -> GitResult<Vec<Commit>> {
    let mut commits = Vec::new();
    for record in data.split(|&b| b == 0) {
        if record.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        let text = String::from_utf8_lossy(record);
        let parts: Vec<&str> = text.splitn(12, LOG_SEP).collect();
        if parts.len() < 12 {
            return Err(GitError::Parse {
                context: "git log",
                details: format!("expected 12 fields, got {}", parts.len()),
            });
        }
        commits.push(Commit {
            id: parts[0].to_string(),
            short_id: parts[1].to_string(),
            parents: parts[2].split_whitespace().map(str::to_string).collect(),
            author_name: parts[3].to_string(),
            author_email: parts[4].to_string(),
            author_time: parts[5].parse().unwrap_or(0),
            committer_name: parts[6].to_string(),
            committer_email: parts[7].to_string(),
            commit_time: parts[8].parse().unwrap_or(0),
            summary: parts[9].to_string(),
            refs: parse_ref_labels(parts[10]),
            body: parts[11].trim_end_matches('\n').to_string(),
        });
    }
    Ok(commits)
}

/// Parse the `%D` decoration string, e.g. `HEAD -> main, tag: v1, origin/main`.
pub fn parse_ref_labels(decoration: &str) -> Vec<RefLabel> {
    let mut refs = Vec::new();
    for token in decoration.split(", ") {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(branch) = token.strip_prefix("HEAD -> ") {
            refs.push(RefLabel {
                name: "HEAD".into(),
                kind: RefKind::Head,
            });
            refs.push(RefLabel {
                name: unquote_ref(branch),
                kind: RefKind::Branch,
            });
        } else if token == "HEAD" {
            refs.push(RefLabel {
                name: "HEAD".into(),
                kind: RefKind::Head,
            });
        } else if let Some(tag) = token.strip_prefix("tag: ") {
            refs.push(RefLabel {
                name: unquote_ref(tag),
                kind: RefKind::Tag,
            });
        } else if token.ends_with("/HEAD") {
            // symbolic ref such as origin/HEAD; skip, it is noise.
        } else if token.contains('/') {
            refs.push(RefLabel {
                name: unquote_ref(token),
                kind: RefKind::RemoteBranch,
            });
        } else {
            refs.push(RefLabel {
                name: unquote_ref(token),
                kind: RefKind::Branch,
            });
        }
    }
    refs
}

fn unquote_ref(s: &str) -> String {
    s.trim_matches('"').to_string()
}

// ---------------------------------------------------------------------------
// for-each-ref based listings
// ---------------------------------------------------------------------------

const FS: char = '\u{1f}';

pub const BRANCH_FORMAT: &str = "%(refname)%1f%(objectname)%1f%(HEAD)%1f%(upstream:short)%1f%(upstream:track)%1f%(contents:subject)%1f%(authorname)%1f%(committerdate:unix)";

pub fn parse_branches(text: &str) -> Vec<Branch> {
    let mut branches = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.splitn(8, FS).collect();
        if f.len() < 8 {
            continue;
        }
        let full_ref = f[0].to_string();
        let is_remote = full_ref.starts_with("refs/remotes/");
        let name = short_ref_name(&full_ref);
        let (ahead, behind, gone) = parse_track(f[4]);
        let remote = if is_remote {
            name.split('/').next().map(str::to_string)
        } else {
            None
        };
        branches.push(Branch {
            is_head: f[2].trim() == "*",
            upstream: {
                let u = f[3].trim();
                if u.is_empty() {
                    None
                } else {
                    Some(u.to_string())
                }
            },
            ahead,
            behind,
            gone,
            tip: f[1].to_string(),
            tip_summary: f[5].to_string(),
            tip_author: f[6].to_string(),
            tip_time: f[7].trim().parse().unwrap_or(0),
            is_remote,
            remote,
            name,
            full_ref,
        });
    }
    branches
}

fn parse_track(track: &str) -> (u32, u32, bool) {
    let t = track.trim();
    if t.contains("gone") {
        return (0, 0, true);
    }
    let mut ahead = 0;
    let mut behind = 0;
    for part in t.trim_matches(['[', ']']).split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

fn short_ref_name(full: &str) -> String {
    full.strip_prefix("refs/heads/")
        .or_else(|| full.strip_prefix("refs/remotes/"))
        .or_else(|| full.strip_prefix("refs/tags/"))
        .unwrap_or(full)
        .to_string()
}

pub const TAG_FORMAT: &str =
    "%(refname:short)%00%(objectname)%00%(objecttype)%00%(*objectname)%00%(taggername)%00%(taggerdate:unix)%00%(contents:subject)";

pub fn parse_tags(text: &str) -> Vec<Tag> {
    let mut tags = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\u{0}').collect();
        if f.len() < 7 {
            continue;
        }
        let is_annotated = f[2] == "tag";
        let target = if is_annotated && !f[3].is_empty() {
            f[3].to_string()
        } else {
            f[1].to_string()
        };
        tags.push(Tag {
            name: f[0].to_string(),
            oid: f[1].to_string(),
            target,
            is_annotated,
            tagger: if f[4].is_empty() {
                None
            } else {
                Some(f[4].to_string())
            },
            time: f[5].trim().parse().ok(),
            subject: f[6].to_string(),
        });
    }
    tags
}

pub fn parse_remotes(text: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(name), Some(url)) = (it.next(), it.next()) else {
            continue;
        };
        // git prints "(fetch)" / "(push)"
        let kind = it.next().unwrap_or("").trim_matches(['(', ')']);
        match kind {
            "fetch" => match remotes.iter_mut().find(|r| r.name == name) {
                Some(r) => r.fetch_url = url.to_string(),
                None => remotes.push(Remote {
                    name: name.to_string(),
                    fetch_url: url.to_string(),
                    push_url: url.to_string(),
                }),
            },
            "push" => match remotes.iter_mut().find(|r| r.name == name) {
                Some(r) => r.push_url = url.to_string(),
                None => remotes.push(Remote {
                    name: name.to_string(),
                    fetch_url: url.to_string(),
                    push_url: url.to_string(),
                }),
            },
            _ => {}
        }
    }
    remotes
}

pub const STASH_FORMAT: &str = "%gd%x1f%H%x1f%gs%x1f%ct%x1f%P";

pub fn parse_stash_list(text: &str) -> Vec<StashEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\u{1f}').collect();
        if f.len() < 5 {
            continue;
        }
        let reflog = f[0];
        let index = reflog
            .strip_prefix("stash@{")
            .and_then(|s| s.strip_suffix('}'))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let message = f[2].to_string();
        let parents: Vec<&str> = f[4].split_whitespace().collect();
        entries.push(StashEntry {
            index,
            oid: f[1].to_string(),
            branch: parse_stash_branch(&message),
            message,
            time: f[3].trim().parse().ok(),
            has_untracked: parents.len() >= 3,
        });
    }
    entries
}

fn parse_stash_branch(message: &str) -> Option<String> {
    if let Some(rest) = message.strip_prefix("WIP on ") {
        return rest.split(':').next().map(str::to_string);
    }
    if let Some(rest) = message.strip_prefix("On ") {
        return rest.split(':').next().map(str::to_string);
    }
    None
}

// ---------------------------------------------------------------------------
// diffs
// ---------------------------------------------------------------------------

pub fn parse_diff(data: &[u8]) -> GitResult<Vec<FileDiff>> {
    let text = String::from_utf8_lossy(data);
    let mut files: Vec<FileDiff> = Vec::new();
    let mut current: Option<FileDiff> = None;
    let mut current_hunk: Option<Hunk> = None;
    let mut in_hunks = false;

    // Keep track of how many lines are expected in the current hunk so that
    // trailing garbage is tolerated.
    let finish_hunk = |file: &mut Option<FileDiff>, hunk: &mut Option<Hunk>| {
        if let (Some(f), Some(h)) = (file.as_mut(), hunk.take()) {
            f.hunks.push(h);
        }
    };

    for raw_line in text.split_inclusive('\n') {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        if let Some(rest) = line.strip_prefix("diff --git ") {
            finish_hunk(&mut current, &mut current_hunk);
            in_hunks = false;
            if let Some(f) = current.take() {
                files.push(f);
            }
            let (old, new) = parse_diff_git_header(rest);
            current = Some(FileDiff {
                old_path: old,
                new_path: new,
                status: ChangeKind::Modified,
                old_mode: None,
                new_mode: None,
                is_binary: false,
                similarity: None,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };
        if in_hunks {
            if let Some(hunk) = current_hunk.as_mut() {
                if let Some(rest) = line.strip_prefix("\\ ") {
                    let _ = rest;
                    hunk.lines.push(DiffLine {
                        kind: LineKind::NoNewline,
                        text: String::new(),
                    });
                } else if let Some(rest) = line.strip_prefix('+') {
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Added,
                        text: rest.to_string(),
                    });
                } else if let Some(rest) = line.strip_prefix('-') {
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Removed,
                        text: rest.to_string(),
                    });
                } else if line.is_empty() {
                    // Tolerate producers that strip the leading space of
                    // empty context lines.
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Context,
                        text: String::new(),
                    });
                } else if let Some(rest) = line.strip_prefix(' ') {
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Context,
                        text: rest.to_string(),
                    });
                } else if line.starts_with("@@") {
                    // New hunk directly (rare in unified diff but tolerated).
                    finish_hunk(&mut current, &mut current_hunk);
                    current_hunk = Some(parse_hunk_header(line)?);
                }
                continue;
            }
        }
        if let Some(rest) = line.strip_prefix("@@") {
            finish_hunk(&mut current, &mut current_hunk);
            in_hunks = true;
            current_hunk = Some(parse_hunk_header(&format!("@@{rest}"))?);
            continue;
        }
        if let Some(mode) = line.strip_prefix("old mode ") {
            file.old_mode = Some(mode.to_string());
            continue;
        }
        if let Some(mode) = line.strip_prefix("new mode ") {
            file.new_mode = Some(mode.to_string());
            continue;
        }
        if let Some(mode) = line.strip_prefix("new file mode ") {
            file.new_mode = Some(mode.to_string());
            file.status = ChangeKind::Added;
            continue;
        }
        if let Some(mode) = line.strip_prefix("deleted file mode ") {
            file.old_mode = Some(mode.to_string());
            file.status = ChangeKind::Deleted;
            continue;
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            file.old_path = Some(PathBuf::from(unquote_path(path)));
            file.status = ChangeKind::Renamed;
            continue;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            file.new_path = Some(PathBuf::from(unquote_path(path)));
            file.status = ChangeKind::Renamed;
            continue;
        }
        if let Some(path) = line.strip_prefix("copy from ") {
            file.old_path = Some(PathBuf::from(unquote_path(path)));
            file.status = ChangeKind::Copied;
            continue;
        }
        if let Some(path) = line.strip_prefix("copy to ") {
            file.new_path = Some(PathBuf::from(unquote_path(path)));
            file.status = ChangeKind::Copied;
            continue;
        }
        if let Some(sim) = line.strip_prefix("similarity index ") {
            file.similarity = sim
                .trim_end_matches('%')
                .parse()
                .ok();
            continue;
        }
        if let Some(path) = line.strip_prefix("--- ") {
            file.old_path = parse_diff_marker_path(path, "a/");
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            file.new_path = parse_diff_marker_path(path, "b/");
            continue;
        }
        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.is_binary = true;
            continue;
        }
    }
    finish_hunk(&mut current, &mut current_hunk);
    if let Some(f) = current.take() {
        files.push(f);
    }
    Ok(files)
}

fn parse_diff_git_header(rest: &str) -> (Option<PathBuf>, Option<PathBuf>) {
    if let Some(pos) = rest.find(" b/") {
        let a = &rest[..pos];
        let b = &rest[pos + 1..];
        let a = a.strip_prefix("a/").unwrap_or(a);
        let b = b.strip_prefix("b/").unwrap_or(b);
        (
            Some(PathBuf::from(unquote_path(a))),
            Some(PathBuf::from(unquote_path(b))),
        )
    } else {
        (None, None)
    }
}

fn parse_diff_marker_path(path: &str, prefix: &str) -> Option<PathBuf> {
    if path.starts_with("/dev/null") {
        return None;
    }
    // Unquote first: a C-quoted path includes the `a/`/`b/` prefix inside the
    // quotes, e.g. "\"a/we\\\"ird.txt\"".
    let unquoted = unquote_path(path);
    let raw = unquoted.strip_prefix(prefix).unwrap_or(&unquoted);
    // git may append a tab when the path contains spaces
    let raw = raw.strip_suffix('\t').unwrap_or(raw);
    Some(PathBuf::from(raw))
}

pub fn parse_hunk_header(line: &str) -> GitResult<Hunk> {
    // @@ -old[,count] +new[,count] @@ section
    let err = || GitError::Parse {
        context: "diff hunk header",
        details: line.to_string(),
    };
    let rest = line.strip_prefix("@@ ").ok_or_else(err)?;
    let end = rest.find(" @@").ok_or_else(err)?;
    let ranges = &rest[..end];
    let section = rest[end + 3..].trim().to_string();
    let mut parts = ranges.split_whitespace();
    let old = parts.next().ok_or_else(err)?;
    let new = parts.next().ok_or_else(err)?;
    let (old_start, old_count) = parse_range(old.strip_prefix('-').ok_or_else(err)?);
    let (new_start, new_count) = parse_range(new.strip_prefix('+').ok_or_else(err)?);
    Ok(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
        section,
        lines: Vec::new(),
    })
}

fn parse_range(s: &str) -> (u32, u32) {
    match s.split_once(',') {
        Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
        None => (s.parse().unwrap_or(0), 1),
    }
}

/// Parse `git diff --name-status -z` output.
pub fn parse_name_status(data: &[u8]) -> Vec<ChangedFile> {
    let mut out = Vec::new();
    let tokens: Vec<&[u8]> = data.split(|&b| b == 0).collect();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        i += 1;
        if tok.is_empty() {
            continue;
        }
        let status_char = tok[0];
        let status = ChangeKind::from_code(status_char);
        let (path, orig) = if status == ChangeKind::Renamed || status == ChangeKind::Copied {
            let orig = tokens.get(i).copied().unwrap_or(b"");
            i += 1;
            let path = tokens.get(i).copied().unwrap_or(b"");
            i += 1;
            (path_from_bytes(path), Some(path_from_bytes(orig)))
        } else {
            let path = tokens.get(i).copied().unwrap_or(b"");
            i += 1;
            (path_from_bytes(path), None)
        };
        out.push(ChangedFile {
            status,
            path,
            orig_path: orig,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// conflicts
// ---------------------------------------------------------------------------

/// Parse conflict markers from a file's text. Handles both the `merge` and
/// `diff3`/`zdiff3` styles.
pub fn parse_conflict_regions(text: &str) -> Vec<ConflictRegion> {
    let lines: Vec<&str> = text.lines().collect();
    let mut regions = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        if !line.starts_with("<<<<<<<") {
            i += 1;
            continue;
        }
        let ours_label = line.trim_start_matches('<').trim().to_string();
        let start_line = i;
        let mut j = i + 1;
        let mut ours: Vec<String> = Vec::new();
        let mut base: Vec<String> = Vec::new();
        let mut theirs: Vec<String> = Vec::new();
        let mut base_label: Option<String> = None;
        let mut theirs_label = String::new();
        let mut section = 0; // 0=ours, 1=base, 2=theirs
        let mut end_line = None;
        while j < lines.len() {
            let l = lines[j];
            if l.starts_with("|||||||") {
                base_label = Some(l.trim_start_matches('|').trim().to_string());
                section = 1;
                j += 1;
                continue;
            }
            if l.starts_with("=======") {
                section = 2;
                j += 1;
                continue;
            }
            if l.starts_with(">>>>>>>") {
                theirs_label = l.trim_start_matches('>').trim().to_string();
                end_line = Some(j + 1);
                break;
            }
            match section {
                0 => ours.push(l.to_string()),
                1 => base.push(l.to_string()),
                _ => theirs.push(l.to_string()),
            }
            j += 1;
        }
        let Some(end) = end_line else {
            break;
        };
        regions.push(ConflictRegion {
            start_line,
            end_line: end,
            ours_label,
            theirs_label,
            base_label,
            ours,
            theirs,
            base,
        });
        i = end;
    }
    regions
}

/// Resolve all conflict regions using the requested choice, returning the new
/// file content. Regions are replaced; anything outside them is preserved.
pub fn resolve_conflicts(
    text: &str,
    regions: &[ConflictRegion],
    choices: &dyn Fn(&ConflictRegion) -> ConflictChoice,
) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    for region in regions {
        while cursor < region.start_line && cursor < lines.len() {
            out.push(lines[cursor].to_string());
            cursor += 1;
        }
        match choices(region) {
            ConflictChoice::Ours => out.extend(region.ours.iter().cloned()),
            ConflictChoice::Theirs => out.extend(region.theirs.iter().cloned()),
            ConflictChoice::Both => {
                out.extend(region.ours.iter().cloned());
                out.extend(region.theirs.iter().cloned());
            }
            ConflictChoice::BothTheirsFirst => {
                out.extend(region.theirs.iter().cloned());
                out.extend(region.ours.iter().cloned());
            }
        }
        cursor = region.end_line;
    }
    while cursor < lines.len() {
        out.push(lines[cursor].to_string());
        cursor += 1;
    }
    let mut result = out.join("\n");
    if text.ends_with('\n') || !result.is_empty() {
        result.push('\n');
    }
    result
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

/// Parse `git config --list --show-origin --show-scope -z` output.
pub fn parse_config_list(data: &[u8]) -> Vec<ConfigEntry> {
    // Layout: <scope>\0<origin>\0<key>\n<value>\0<scope>\0 ...
    let text = String::from_utf8_lossy(data);
    let mut fields = text.split('\0');
    let mut out = Vec::new();
    loop {
        let Some(scope) = fields.next() else { break };
        if scope.is_empty() {
            continue;
        }
        let Some(origin) = fields.next() else { break };
        let Some(kv) = fields.next() else { break };
        let (key, value) = match kv.split_once('\n') {
            Some((k, v)) => (k, v),
            None => (kv, ""),
        };
        if key.is_empty() {
            continue;
        }
        out.push(ConfigEntry {
            scope: ConfigScope::from_str(scope),
            origin: origin.to_string(),
            key: key.to_string(),
            value: value.to_string(),
        });
    }
    out
}

/// Parse `git rev-parse` style single-line ref lookups.
pub fn nonempty_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(parts: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        for p in parts {
            v.extend_from_slice(p.as_bytes());
            v.push(0);
        }
        v
    }

    #[test]
    fn status_headers_and_entries() {
        let data = z(&[
            "# branch.oid 1234567890abcdef1234567890abcdef12345678",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -3",
            "1 M. N... 100644 100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb src/app.rs",
            "1 .M N... 100644 100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa README.md",
            "? new file.txt",
        ]);
        let s = parse_status_v2(&data).unwrap();
        assert_eq!(s.head_name.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (2, 3));
        assert_eq!(s.entries.len(), 3);
        assert!(s.entries[0].is_staged());
        assert!(!s.entries[0].is_unstaged());
        assert_eq!(s.entries[1].worktree, ChangeKind::Modified);
        assert!(s.entries[2].is_untracked());
        assert_eq!(s.staged().len(), 1);
        assert_eq!(s.untracked().len(), 1);
    }

    #[test]
    fn status_detached_unborn() {
        let data = z(&[
            "# branch.oid (initial)",
            "# branch.head (detached)",
        ]);
        let s = parse_status_v2(&data).unwrap();
        assert!(s.detached);
        assert!(s.unborn);
        assert!(s.head_name.is_none());
    }

    #[test]
    fn status_rename_with_spaces() {
        let data = z(&[
            "# branch.head main",
            "2 R. N... 100644 100644 100644 aaa bbb R100 new name.txt",
            "old name.txt",
        ]);
        let s = parse_status_v2(&data).unwrap();
        assert_eq!(s.entries.len(), 1);
        let e = &s.entries[0];
        assert_eq!(e.index, ChangeKind::Renamed);
        assert_eq!(e.path.to_string_lossy(), "new name.txt");
        assert_eq!(e.orig_path.as_ref().unwrap().to_string_lossy(), "old name.txt");
    }

    #[test]
    fn status_conflicts() {
        let data = z(&[
            "# branch.head main",
            "u UU N... 100644 100644 100644 100644 aaa bbb ccc conflicted.txt",
            "u AA N... 100644 100644 100644 100644 aaa bbb ccc both-added.txt",
        ]);
        let s = parse_status_v2(&data).unwrap();
        assert!(s.has_conflicts());
        assert_eq!(s.conflicted().len(), 2);
        assert_eq!(s.entries[0].conflict, Some(ConflictKind::BothModified));
        assert_eq!(s.entries[1].conflict, Some(ConflictKind::BothAdded));
    }

    #[test]
    fn status_unicode_and_spaces() {
        let data = z(&[
            "# branch.head main",
            "? café menu.txt",
            "? 日本語.md",
        ]);
        let s = parse_status_v2(&data).unwrap();
        assert_eq!(s.entries[0].path.to_string_lossy(), "café menu.txt");
        assert_eq!(s.entries[1].path.to_string_lossy(), "日本語.md");
    }

    fn log_record(fields: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(fields.join("\u{1f}").as_bytes());
        v.push(0);
        v
    }

    #[test]
    fn log_parsing_with_body_and_refs() {
        let mut data = log_record(&[
            "aa5e486e83c63b0ce471db93782373c0cd2eba86",
            "aa5e486",
            "7f813733d2786807bef6301b37285f6b665a054a",
            "Ann",
            "a@a",
            "1791399460",
            "Bob",
            "b@b",
            "1791399999",
            "second",
            "HEAD -> main, tag: v1, origin/main",
            "second\n\nbody line1\nbody line2\n",
        ]);
        data.extend(log_record(&[
            "7f813733d2786807bef6301b37285f6b665a054a",
            "7f81373",
            "",
            "Ann",
            "a@a",
            "1791399000",
            "Ann",
            "a@a",
            "1791399000",
            "first commit",
            "",
            "first commit\n",
        ]));
        let commits = parse_log(&data).unwrap();
        assert_eq!(commits.len(), 2);
        let c = &commits[0];
        assert_eq!(c.summary, "second");
        assert_eq!(c.body, "second\n\nbody line1\nbody line2");
        assert_eq!(c.parents.len(), 1);
        assert!(c.is_merge() == false);
        assert_eq!(c.refs.len(), 4);
        assert_eq!(c.refs[0].kind, RefKind::Head);
        assert!(c.refs.iter().any(|r| r.kind == RefKind::Tag && r.name == "v1"));
        assert!(c.refs.iter().any(|r| r.kind == RefKind::RemoteBranch));
        assert!(commits[1].parents.is_empty());
    }

    #[test]
    fn merge_commit_refs() {
        let mut data = log_record(&[
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "aaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb cccccccccccccccccccccccccccccccccccccccc",
            "Ann",
            "a@a",
            "1791399460",
            "Ann",
            "a@a",
            "1791399460",
            "Merge branch 'feature'",
            "HEAD",
            "",
        ]);
        data.extend(log_record(&[
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "bbbbbbb",
            "",
            "Ann",
            "a@a",
            "1791399000",
            "Ann",
            "a@a",
            "1791399000",
            "x",
            "",
            "",
        ]));
        let commits = parse_log(&data).unwrap();
        assert!(commits[0].is_merge());
        assert_eq!(commits[0].parents.len(), 2);
        assert_eq!(commits[0].head_ref().unwrap().name, "HEAD");
    }

    #[test]
    fn branches_with_tracking() {
        let text = "refs/heads/main\u{1f}aaaa\u{1f}*\u{1f}origin/main\u{1f}[ahead 2, behind 1]\u{1f}Fix thing\u{1f}Ann\u{1f}1791399000\nrefs/heads/feature\u{1f}bbbb\u{1f} \u{1f}\u{1f}\u{1f}WIP\u{1f}Bob\u{1f}1791398000\nrefs/remotes/origin/main\u{1f}aaaa\u{1f} \u{1f}\u{1f}\u{1f}Fix thing\u{1f}Ann\u{1f}1791399000\n";
        let branches = parse_branches(text);
        assert_eq!(branches.len(), 3);
        let main = &branches[0];
        assert!(main.is_head);
        assert_eq!(main.name, "main");
        assert!(!main.is_remote);
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert_eq!((main.ahead, main.behind), (2, 1));
        let feature = &branches[1];
        assert!(feature.upstream.is_none());
        let remote = &branches[2];
        assert!(remote.is_remote);
        assert_eq!(remote.name, "origin/main");
        assert_eq!(remote.remote.as_deref(), Some("origin"));
    }

    #[test]
    fn branch_gone_upstream() {
        let text = "refs/heads/x\u{1f}aaaa\u{1f} \u{1f}origin/x\u{1f}[gone]\u{1f}s\u{1f}a\u{1f}0\n";
        let branches = parse_branches(text);
        assert!(branches[0].gone);
    }

    #[test]
    fn tags_annotated_and_light() {
        let text = "v1\u{0}aaaaaaaa\u{0}tag\u{0}bbbbbbbb\u{0}Ann\u{0}1791399000\u{0}release one\nlight\u{0}cccccccc\u{0}commit\u{0}\u{0}\u{0}\u{0}subject\n";
        let tags = parse_tags(text);
        assert_eq!(tags.len(), 2);
        assert!(tags[0].is_annotated);
        assert_eq!(tags[0].target, "bbbbbbbb");
        assert_eq!(tags[0].tagger.as_deref(), Some("Ann"));
        assert!(!tags[1].is_annotated);
        assert_eq!(tags[1].target, "cccccccc");
        assert_eq!(tags[1].subject, "subject");
    }

    #[test]
    fn remotes_merge_push_url() {
        let text = "origin\thttps://example.com/repo.git (fetch)\norigin\thttps://example.com/repo.git (push)\nfork\tgit@github.com:me/repo.git (fetch)\nfork\tgit@github.com:me/repo.git (push)\n";
        let remotes = parse_remotes(text);
        assert_eq!(remotes.len(), 2);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[1].push_url, "git@github.com:me/repo.git");
    }

    #[test]
    fn stash_list_parsing() {
        let text = "stash@{0}\u{1f}aaaa\u{1f}WIP on main: 1234567 fix thing\u{1f}1791399000\u{1f}1111 2222\nstash@{1}\u{1f}bbbb\u{1f}On feature: manual message\u{1f}1791398000\u{1f}1111 2222 3333\n";
        let stashes = parse_stash_list(text);
        assert_eq!(stashes.len(), 2);
        assert_eq!(stashes[0].index, 0);
        assert_eq!(stashes[0].branch.as_deref(), Some("main"));
        assert!(!stashes[0].has_untracked);
        assert_eq!(stashes[1].branch.as_deref(), Some("feature"));
        assert!(stashes[1].has_untracked);
    }

    #[test]
    fn diff_modify_with_hunks() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,6 @@ fn main() {
 use std::io;
-old line
+new line
+extra line
 context
 tail
";
        let files = parse_diff(diff.as_bytes()).unwrap();
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.status, ChangeKind::Modified);
        assert_eq!(f.path().unwrap().to_string_lossy(), "src/main.rs");
        assert_eq!(f.hunks.len(), 1);
        assert_eq!(f.added_lines(), 2);
        assert_eq!(f.removed_lines(), 1);
        let h = &f.hunks[0];
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count), (1, 5, 1, 6));
        assert_eq!(h.section, "fn main() {");
        assert_eq!(h.lines[0].kind, LineKind::Context);
        assert_eq!(h.lines[1].kind, LineKind::Removed);
        assert_eq!(h.lines[1].text, "old line");
        assert_eq!(h.lines[2].kind, LineKind::Added);
    }

    #[test]
    fn diff_new_and_deleted_and_binary() {
        let diff = "\
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+hello
+world
diff --git a/gone.bin b/gone.bin
deleted file mode 100644
index 1111111..0000000
Binary files a/gone.bin and b/gone.bin differ
";
        let files = parse_diff(diff.as_bytes()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status, ChangeKind::Added);
        assert!(files[0].old_path.is_none());
        assert_eq!(files[0].hunks[0].old_count, 0);
        assert_eq!(files[1].status, ChangeKind::Deleted);
        assert!(files[1].is_binary);
    }

    #[test]
    fn diff_rename_and_spaces() {
        let diff = "\
diff --git a/old name.txt b/new name.txt
similarity index 95%
rename from old name.txt
rename to new name.txt
index 1111111..2222222 100644
--- a/old name.txt
+++ b/new name.txt
@@ -1 +1 @@
-old
+new
";
        let files = parse_diff(diff.as_bytes()).unwrap();
        assert_eq!(files[0].status, ChangeKind::Renamed);
        assert_eq!(files[0].similarity, Some(95));
        assert_eq!(files[0].old_path.as_ref().unwrap().to_string_lossy(), "old name.txt");
        assert_eq!(files[0].new_path.as_ref().unwrap().to_string_lossy(), "new name.txt");
    }

    #[test]
    fn diff_quoted_paths_and_unicode() {
        let diff = "\
diff --git \"a/we\\\"ird.txt\" \"b/we\\\"ird.txt\"
index 1111111..2222222 100644
--- \"a/we\\\"ird.txt\"
+++ \"b/we\\\"ird.txt\"
@@ -1 +1 @@
-a
+b
diff --git a/café.txt b/café.txt
";
        let files = parse_diff(diff.as_bytes()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path().unwrap().to_string_lossy(), "we\"ird.txt");
        assert_eq!(files[1].path().unwrap().to_string_lossy(), "café.txt");
    }

    #[test]
    fn name_status_with_rename() {
        let data = z(&["M", "a.txt", "A", "b.txt", "R100", "old.txt", "new.txt", "D", "gone.txt"]);
        let files = parse_name_status(&data);
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].status, ChangeKind::Modified);
        assert_eq!(files[2].status, ChangeKind::Renamed);
        assert_eq!(files[2].orig_path.as_ref().unwrap().to_string_lossy(), "old.txt");
        assert_eq!(files[2].path.to_string_lossy(), "new.txt");
    }

    #[test]
    fn hunk_header_variants() {
        let h = parse_hunk_header("@@ -1 +1 @@").unwrap();
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count), (1, 1, 1, 1));
        let h = parse_hunk_header("@@ -0,0 +1,5 @@ heading").unwrap();
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count), (0, 0, 1, 5));
        assert_eq!(h.section, "heading");
        assert!(parse_hunk_header("garbage").is_err());
    }

    #[test]
    fn conflict_merge_style() {
        let text = "line1\n<<<<<<< HEAD\nours a\nours b\n=======\ntheirs a\n>>>>>>> feature\nline2\n";
        let regions = parse_conflict_regions(text);
        assert_eq!(regions.len(), 1);
        let r = &regions[0];
        assert_eq!(r.ours_label, "HEAD");
        assert_eq!(r.theirs_label, "feature");
        assert_eq!(r.ours, vec!["ours a", "ours b"]);
        assert_eq!(r.theirs, vec!["theirs a"]);
        assert!(r.base.is_empty());
        let resolved = resolve_conflicts(text, &regions, &|_| ConflictChoice::Ours);
        assert_eq!(resolved, "line1\nours a\nours b\nline2\n");
        let resolved = resolve_conflicts(text, &regions, &|_| ConflictChoice::Theirs);
        assert_eq!(resolved, "line1\ntheirs a\nline2\n");
        let resolved = resolve_conflicts(text, &regions, &|_| ConflictChoice::Both);
        assert_eq!(resolved, "line1\nours a\nours b\ntheirs a\nline2\n");
    }

    #[test]
    fn conflict_diff3_style_with_base() {
        let text = "<<<<<<< ours\nA\n||||||| base\nB\n=======\nC\n>>>>>>> theirs\n";
        let regions = parse_conflict_regions(text);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].base, vec!["B"]);
        assert_eq!(regions[0].base_label.as_deref(), Some("base"));
        let resolved = resolve_conflicts(text, &regions, &|_| ConflictChoice::Theirs);
        assert_eq!(resolved, "C\n");
    }

    #[test]
    fn conflict_multiple_regions_and_unterminated() {
        let text = "a\n<<<<<<< HEAD\n1\n=======\n2\n>>>>>>> x\nmid\n<<<<<<< HEAD\n3\n=======\n4\n>>>>>>> y\nz\n";
        let regions = parse_conflict_regions(text);
        assert_eq!(regions.len(), 2);
        let resolved = resolve_conflicts(text, &regions, &|_| ConflictChoice::BothTheirsFirst);
        assert_eq!(resolved, "a\n2\n1\nmid\n4\n3\nz\n");
        // An unterminated region must not panic or lose data
        let broken = "a\n<<<<<<< HEAD\n1\n=======\n2\n";
        let regions = parse_conflict_regions(broken);
        assert!(regions.is_empty());
    }

    #[test]
    fn config_list_parsing() {
        // Output of `git config --list --show-origin --show-scope -z`
        let text = "system\u{0}/etc/gitconfig\u{0}core.autocrlf\ntrue\u{0}\
                    global\u{0}/home/u/.gitconfig\u{0}user.name\nAnn\u{0}\
                    local\u{0}/home/u/repo/.git/config\u{0}remote.origin.url\nhttps://x/y.git\u{0}";
        let entries = parse_config_list(text.as_bytes());
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].scope, ConfigScope::System);
        assert_eq!(entries[0].value, "true");
        assert_eq!(entries[1].scope, ConfigScope::Global);
        assert_eq!(entries[1].key, "user.name");
        assert_eq!(entries[1].value, "Ann");
        assert_eq!(entries[2].scope, ConfigScope::Local);
        assert_eq!(entries[2].value, "https://x/y.git");
    }

    #[test]
    fn unquote_escapes() {
        assert_eq!(unquote_path("plain"), "plain");
        assert_eq!(unquote_path("\"a\\tb\""), "a\tb");
        assert_eq!(unquote_path("\"caf\\303\\251\""), "café");
        assert_eq!(unquote_path("\"quote\\\"x\""), "quote\"x");
    }

    #[test]
    fn empty_diff_is_empty() {
        assert!(parse_diff(b"").unwrap().is_empty());
        assert!(parse_name_status(b"").is_empty());
    }

    #[test]
    fn crlf_and_no_newline_marker() {
        let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n";
        let files = parse_diff(diff.as_bytes()).unwrap();
        assert_eq!(files[0].hunks[0].lines.len(), 3);
        assert_eq!(files[0].hunks[0].lines[2].kind, LineKind::NoNewline);
    }
}

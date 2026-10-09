# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-10-09 (alpha)

First alpha release. The application is renamed from Stratum to GitTrellis;
settings now live in `~/.config/gittrellis` and logs in
`~/.local/state/gittrellis`.

### Added

- Keyboard shortcuts are now listed and editable under Settings → Shortcuts,
  with per-action capture, disable (Backspace) and reset to default.
- Ctrl+1..6 switch between Changes, History, Branches, Tags, Stashes and
  Remotes; Ctrl+F performs a direct fetch.
- Automatic refresh: the active repository's status is polled every few
  seconds while the window is focused and no operation is running.
- History search can match changed file names using a case-insensitive
  substring ("File path" field).
- The askpass dialog offers “Add the key to SSH agent for this session”.

### Changed

- The sidebar header now shows the application name followed by small
  Initialize, Open and Clone icons; the large Initialize button and the
  context-aware sync button were removed.
- Fetch from the toolbar/shortcut is direct; the Remotes tab keeps the fetch
  options dialog (remote, prune, tags).
- Cancelling the authentication prompt now reports “Authentication cancelled”
  instead of a misleading permission error.

### Fixed

- Publishing a branch from the push dialog now works: short branch names
  (`main`) were rejected by an over-strict guard, so pressing Push did
  nothing and reported nothing. Short names are converted to full refspecs,
  and invalid names now produce a visible message.

### Fixed (changes view)

- Unstaging files works in a repository without any commits yet (an unborn
  `HEAD` made `git reset HEAD` fail).
- The Recent repositories list was never populated.
- Clicking an already selected change now deselects it and clears the diff;
  multi-selection is preserved across automatic refreshes.
- Fetching or pushing no longer risks a stuck progress bar when an askpass
  process lingers (see the process-group fixes above).

### Fixed

- The askpass dialog now focuses the password field immediately, submits on
  Enter and no longer ignores an empty answer silently — this is why entering
  a passphrase could appear to "do nothing".
- Cancelling a fetch/push now terminates the entire process group (git, ssh
  and the askpass helper), so the progress bar stops promptly and any open
  passphrase dialog closes.
- A lingering askpass process can no longer hold the output pipes open:
  operations always complete, so the progress bar can no longer get stuck.
- Removed a spurious `GtkWindowControls` baseline warning printed by the
  askpass dialog.

### Fixed (previous)

- Passphrase-protected SSH keys and HTTPS credential prompts no longer fail
  with `ssh_askpass: exec(): No such file or directory`. A native askpass
  helper is installed next to the main binary and wired into `SSH_ASKPASS`
  and `GIT_ASKPASS` with `SSH_ASKPASS_REQUIRE=force`.
- Authentication error messages now explain the passphrase/ssh-agent situation
  and how to verify a key.

### Added

- One-click push and pull: the header buttons and Ctrl+P / Ctrl+Shift+P act
  directly when the branch has an upstream, with the full dialogs still
  available from the Remotes tab and the branch menu.
- A context-aware Sync button showing `Pull N`, `Push N`, `Sync ↑N ↓M` or
  `Publish Branch`, with one-click pull-then-push when the branch has
  diverged.

## [0.1.0] - 2026-10-07

### Added

- Git service layer built on the system `git` executable with a deterministic
  configuration prefix, NUL-safe pathspec handling, structured error reporting
  and cancellation support for long-running operations.
- Structured parsers for status (v2), log, branches, tags, remotes, stashes,
  diffs, name-status, conflicts and configuration, with unit tests.
- Changes view grouping conflicts, staged, unstaged, untracked and ignored
  files, with stage/unstage/discard/delete/add-to-.gitignore actions.
- Diff viewer with unified and side-by-side layouts, line numbers, word-level
  highlighting, binary detection, large-diff truncation, and hunk-level
  stage/unstage/discard.
- Commit panel with amend, sign-off and GPG/SSH signing support.
- History view with a drawn commit graph, commit details, file diffs, search
  and commit actions (copy, branch, tag, cherry-pick, revert, reset, checkout,
  compare).
- Branch management: checkout, create, rename, delete, merge, rebase, set
  upstream, push, compare.
- Remote operations with progress and cancellation: fetch (with prune), pull
  (merge/rebase/ff-only), push (normal, force-with-lease, force), delete
  remote branches, push tags.
- Conflict resolution: per-region accept current/incoming/both, mark resolved,
  continue/abort/skip for merge, rebase, cherry-pick and revert.
- Stash management: create, apply, apply with index, pop, drop, branch from
  stash, view diff.
- Tag management: lightweight and annotated, checkout, push, delete locally
  and remotely.
- Clone dialog with progress and cancellation, and repository initialization.
- Multi-repository sidebar with recent repositories persisted between
  sessions.
- Application settings (diff layout, context lines, ignored files, fetch
  prune, commit defaults, log level) and repository-scoped Git configuration.
- Diagnostics window with the in-memory log, copy support and the log file
  location; Git output console at the bottom of the window.
- Keyboard shortcuts and a shortcut reference (F1).
- Fedora RPM spec, vendoring script and Flatpak manifest, desktop entry,
  AppStream metadata and application icon.

[0.2.0]: https://example.invalid/gittrellis/releases/tag/v0.2.0
[0.1.0]: https://example.invalid/gittrellis/releases/tag/v0.1.0

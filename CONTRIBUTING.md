# Contributing to GitTrellis

Thanks for considering a contribution. This document describes how the
project is structured, the conventions we follow, and how to verify changes.

## Getting started

```bash
sudo dnf install git gtk4-devel libadwaita-devel desktop-file-utils \
    libappstream-glib rust cargo
git clone <repository-url> gittrellis
cd gittrellis
cargo test
cargo run -- /path/to/a/repository
```

`scripts/make-demo-repo.sh` creates a repository with commits, a branch,
uncommitted changes and an untracked file, which is handy for manual testing.

## Architecture rules

These are not style preferences; they keep the codebase maintainable:

1. **No Git invocation outside `src/git/`.** The UI and core layers must never
   run `git` themselves or build command strings.
2. **No raw output in the UI.** Views consume the structured types from
   `src/git/types.rs`.
3. **No shell, ever.** Arguments are passed as vectors; path lists go through
   `--pathspec-from-file=- --pathspec-file-nul`.
4. **No blocking the main thread.** Long operations use `core::task` helpers.
5. **No silent data loss.** Destructive actions need explicit confirmation
   (`ui::confirm::confirm_dangerous` for the worst ones).
6. **No new dependencies without a reason.** Prefer the standard library,
   GTK/libadwaita and the crates already in `Cargo.toml`.
7. **No comments that restate the code.** Comments explain *why*.

## Adding a feature

- If it needs Git output that is not parsed yet, add a parser to
  `src/git/parse.rs` with unit tests using realistic fixtures, then wire it to
  a method on `Repository`.
- Add the high-level operation to `src/git/ops.rs` (validating refs/paths,
  never concatenating commands).
- Add a `UiRequest` variant and handle it in `RepoPage::handle_request`,
  updating the relevant view's render method.
- Extend `tests/git_integration.rs` with a real-repository test.

## Testing

```bash
cargo test                       # unit + integration
cargo test --test git_integration
cargo build --release            # must succeed without warnings
```

Integration tests run the real `git` binary in temporary repositories with
`GIT_CONFIG_NOSYSTEM=1` and a private `HOME`, so they are deterministic. Do
not add tests that depend on the developer's global Git configuration.

For UI changes, follow `docs/MANUAL_TESTING.md` and attach screenshots to the
merge request when the change is visual.

## Code style

- `cargo fmt` before committing.
- Keep functions small; prefer explicit types over clever generics.
- Errors: use `GitError` for anything Git related; every user-facing error
  must have a human-readable message and copyable technical details.
- Logging: use the `log_info!`, `log_warn!`, `log_error!` macros. Never log
  credentials; the logging layer redacts, but do not rely on it alone.

## Packaging

`scripts/make-source-tarball.sh` vendors dependencies; `scripts/build-rpm.sh`
builds the RPM. If you add data files (desktop entry, icon, metadata), install
them from the spec and the Flatpak manifest as well.

## Security

- Never store passwords, tokens or private keys.
- Never enable Git's terminal prompt; authentication failures must surface as
  errors.
- Treat every path that comes from Git status or diffs as untrusted input and
  validate it with `Repository::resolve_in_workdir` before touching the
  filesystem.

## Commit messages

Follow conventional commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`).
Keep the subject under 72 characters.

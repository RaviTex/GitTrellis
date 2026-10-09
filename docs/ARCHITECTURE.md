# Architecture

GitTrellis is deliberately layered so that Git logic never leaks into the UI and
the UI is never driven by parsing raw command output.

```
┌──────────────────────────────────────────────────────────────┐
│ UI (src/ui)                                                  │
│   window → workspace → [changes | history | branches | …]    │
│   Views emit UiRequest values; the RepoPage executes them.   │
└───────────────────────────┬──────────────────────────────────┘
                            │ structured types, GitResult
┌───────────────────────────▼──────────────────────────────────┐
│ Core (src/core)                                              │
│   settings (XDG), logging (redacting), snapshots,            │
│   background task runner (worker threads + main-thread hops) │
└───────────────────────────┬──────────────────────────────────┘
                            │
┌───────────────────────────▼──────────────────────────────────┐
│ Git service (src/git)                                        │
│   Repository / ops / parse / patch / command                 │
│   The only place that invokes the git executable.            │
└──────────────────────────────────────────────────────────────┘
```

## Why the Git executable instead of a library

`libgit2`/`git2` would avoid a subprocess per operation, but it re-implements
authentication, credential helpers, signing, hooks and edge cases that users
already depend on. GitTrellis shells out to the system `git` because:

- SSH agent, credential helpers, GPG/SSH signing and `known_hosts` work
  exactly as they do on the command line — no parallel credential system;
- behaviour matches the CLI, including hooks, `.gitattributes`, filters and
  `safe.directory` protections;
- correctness for unusual repositories (replace refs, alternates, worktrees)
  comes for free.

The cost is process spawning, which is acceptable because all reads and writes
are explicit, small commands and nothing runs on the UI thread.

### Deterministic invocation

`Git::system()` builds every invocation with

- `-c core.quotepath=false`, `-c color.ui=false`, `-c log.showSignature=false`,
  `-c diff.noprefix=false`, `-c diff.mnemonicPrefix=false`,
  `-c rebase.autosquash=false`, `-c rebase.autostash=false` so parsing never
  depends on the user's configuration;
- `GIT_TERMINAL_PROMPT=0` so a GUI can never hang on a password prompt;
- `LC_ALL=C` for stable, grep-able English diagnostics;
- `GIT_PAGER=cat` and no shell, ever.

Arguments are always passed as an `OsString` vector. Path lists are sent via
`--pathspec-from-file=- --pathspec-file-nul`, which is correct for spaces,
Unicode, tabs, quotes, leading dashes and bytes that are not valid UTF-8.

### Parsing

All parsers live in `src/git/parse.rs` and operate on bytes where needed:

- status: `git status --porcelain=v2 --branch -z`
- history: `git log -z` with `%x1f` field separators; the raw body is the last
  field so it cannot corrupt earlier fields
- branches/tags: `git for-each-ref` with NUL/`%x1f` separators
- configuration: `git config --list --show-origin --show-scope -z`
- diffs: unified diff parsing with rename/copy/binary/mode handling
- conflicts: merge and diff3 marker parsing plus a resolver

Non-UTF-8 paths are preserved as `PathBuf` through `OsStr::from_bytes` on
Unix and only lossily converted for display.

## Concurrency model

- `core::task::run_blocking` runs a closure on a worker thread and delivers
  the result to the GTK main thread through `glib::spawn_future_local` +
  `async_channel`.
- `core::task::run_streaming` additionally forwards progress lines and exposes
  a `Cancellation` token; it polls on a short main-thread timer so cancelling a
  fetch/push/clone is immediate.
- Reads return a `RepoSnapshot` (status + branches + tags + remotes + stashes)
  loaded section by section; a failing section is reported without discarding
  the rest.
- Views never call the Git layer. They emit `UiRequest` values; `RepoPage`
  runs the corresponding operation, refreshes state and re-renders. This keeps
  Git logic in one place and makes every destructive action pass through one
  confirmation path.

## Rendering

- The commit graph uses a pure lane-assignment algorithm (`src/ui/graph.rs`,
  unit tested) and custom Cairo drawing inside a `GtkListView` factory, so
  only visible rows are drawn.
- Diffs are rendered in `GtkTextView` with text tags; colours are looked up
  from the current theme (`success_color`, `error_color`, `accent_color`) so
  dark and light modes both work.
- Side-by-side diffs are built from paired remove/add runs, with word-level
  highlighting on paired lines.

## Safety rules encoded in the code

- Destructive operations (discard, delete untracked, hard reset, branch/tag
  deletion, stash drop, force push, aborting an operation) require an explicit
  confirmation; the most dangerous ones require ticking “I understand this
  cannot be undone”.
- `Repository::resolve_in_workdir` refuses absolute paths and `..` escapes
  before any file is written or deleted.
- Ref and remote names are validated before use.
- `git reset <target>` is invoked without a trailing `--` so it cannot
  degrade into a pathspec reset that silently leaves untracked files behind.

## Testing strategy

- Unit tests cover every parser and the graph layout.
- Integration tests exercise the real binary in temporary repositories with
  an isolated environment; see `tests/git_integration.rs`.
- The full UI workflow (open → modify → stage → diff → commit → branch →
  merge → resolve conflict) is covered end to end at the service-layer level
  by `full_workflow_simulation`, and manually by
  `docs/MANUAL_TESTING.md`.

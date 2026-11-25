# GitTrellis feature audit

The audit ran on Fedora 45 x86_64 with Git 2.56.1, GTK 4.24.1, libadwaita
1.10.0, and VTE 0.84.1. Desktop checks used an isolated X11 Xvfb display
and a native Wayland session on a virtual KWin compositor. Git operations
used disposable repositories and were checked against refs, the index,
working files, or verified signatures. SSH and HTTPS used loopback servers
and temporary credentials.

The display-backed test suite passed **74 unit and 65 integration tests**
on both Wayland and X11. The baseline was 71 unit and 59 integration
tests. Repeated runs passed after isolating the askpass cancellation marker.
With GTK required and both display variables absent, the Settings test
failed with the expected initialization error. The hosted Fedora job also
passed with the GTK display required and skipped publication because the
Cargo version stayed the same. [Fedora CI](https://github.com/RaviTex/GitTrellis/actions/runs/45663588809)

## Feature results

**Pass** applies to the checks in the evidence column. The last column
lists the GUI paths that still need direct coverage where a feature was
verified through the Git service.

| Feature | Status | Evidence | GTK coverage still needed |
| --- | --- | --- | --- |
| Open, initialize, clone, and recent repositories | Pass | GTK opened a disposable repository with the correct title and branch. The Git service initialized and cloned repos, rejected a bare repo and invalid paths, and produced a one-commit shallow clone. | Folder-picker initialization, GUI clone cancellation, and recent-session restore. |
| Changes, selection, and file actions | Pass | GTK showed the selected file's diff, staged only the selected file, unstaged it from its row, and staged tracked and untracked files together. Tests covered renames, untracked deletion, staged and unstaged discard, unborn branches, and unusual paths. | Range selection, ignored-file filtering, editor and folder launch, and destructive dialog recovery. |
| Diff and hunk actions | Pass | GTK rendered unified and side-by-side diffs. Selecting and staging the first of two separated hunks changed only that hunk in the index. Unstaging restored both edits to the worktree without discarding either. Service tests also checked binary detection, renames, worktree/staged/commit diffs, and discarding individual hunks. | Large-diff handling and the discard confirmation in GTK. |
| Commit, signing, and amend | Pass | GTK blocked an empty commit and preserved the subject, body, and Signed-off-by trailer. Git verified a GTK-created SSH-signed commit and an OpenPGP-signed commit from the Git service. A typed amend replaced HEAD and preserved its parent. | Recent commit messages and the OpenPGP path from the GTK editor. |
| Amend message prefill | Fail | Toggling Amend left the editor empty. Entering a revised message still completed the amend. The manual checklist now reflects that behavior. | — |
| History graph, details, and search | Pass | GTK showed the slashed local ref separately from the remote-tracking ref. Service tests covered graph topology, ordering, pagination, and message and path filtering. | Large-graph pagination and author filter interaction. See the separate Hash result. |
| History Hash search | Fail | Typing an exact commit SHA in X11 showed the right details beside a graph reporting zero commits. The graph uses commit-message matching, and pagination drops the Hash filter. | — |
| Compare branches and commits | Pass | In GTK, the branch dialog started with distinct base and target branches, and both branch and commit comparison showed read-only results. Git verified that neither operation moved a ref. | — |
| Header branch management | Pass | GTK created and switched to a slashed local branch, opened the menu with Ctrl+3, filtered the grouped local and remote refs, checked out a remote branch with the correct upstream, and cancelled then confirmed remote deletion. | Exercise push, copy, rename, delete, and upstream editing independently in the relocated menu. The Git service covers those underlying operations. |
| History drag and keyboard integration | Pass | Native Wayland pointer drag displayed the correct confirmation, Cancel preserved both refs, and Continue fast-forwarded the intended target. X11 accepted distinct branches on the same tip and rejected a self-drop and a remote-badge drop. An X11 keyboard flow rebased the source onto the target. Git verified both tips and the checked-out branch. A drag after scrolling a 160-commit History away and back used the correct rebound refs. Regression tests cover typed and cross-repository payload rejection, stale tips, external checkout, dirty checkout, and conflicts. | Physical drag between two repository pages. |
| Conflict resolution | Pass | A History merge opened Conflicts in GTK. Accept Incoming wrote the expected content, Mark Resolved cleared unmerged entries, and Continue created a clean merge commit with the right parents. Service tests covered marker parsing, merge and rebase abort/continue, cherry-pick conflict and abort, and revert conflict with abort and continue. | Exercise the other region choices and the Abort and Skip controls in GTK across operation types. |
| Tags and stashes | Pass | Tests exercised lightweight and annotated tags, tags at a selected commit, stash creation with untracked files, apply/pop/drop, and branch from stash. The Git service preserved staged data with keep-index, restored the index, and pushed and deleted a remote tag. | Exercise the tag, stash, and index-restoration actions from their GTK menus. |
| Remotes and sync | Pass | The real Git service exercised remote setup, tracking, ahead/behind, clone, fetch and cancellation, pull and push, publishing, remote deletion and prune, fast-forward-only refusal, pull rebase, and force-with-lease rejection before fetch and success afterward. GTK fetched through a TLS-authenticated loopback remote. | Exercise the full GTK quick-sync state matrix. |
| Authentication and cancellation | Pass | Real loopback HTTPS prompted for username and password and fetched refs. The SSH dialog accepted an encrypted Ed25519 key's passphrase, populated a fresh agent, and reused it on a later fetch. GUI Cancel dismissed both prompts. SSH reported cancellation. The test credentials were absent from the app logs. | Exercise an external credential helper and an unknown-host failure separately. |
| Settings, keyboard, and native integration | Pass | Display-backed tests opened Settings directly on the requested page. Automated tests exercised settings persistence and shortcut defaults and conflicts. A saved custom Branches shortcut opened the new menu on restart. The terminal started in the repo, accepted a shell command, and hid when Git output opened. Diagnostics copied the log to the clipboard and cleared the in-memory view. Branch workflows ran on native Wayland and X11. | Live theme switching and scaling. |
| Fedora RPM | Pass | Commit `a6a1d01` built an offline, vendored RPM. A fresh Fedora 45 container installed it, checked the digests and installed files, validated the desktop entry and AppStream data, and opened `rpm-qa-demo` from the installed GUI under Xvfb. | — |
| Flatpak | Blocked | In a fresh Fedora container, the documented install could not find `org.freedesktop.Sdk.Extension.rust-stable//47`. Flathub marks GNOME SDK 47 end-of-life and points to the Freedesktop extension branch `24.08`. The unmodified builder then could not resolve the SDK. With a clean Cargo home, its offline Cargo command also failed to find `libadwaita`. | Re-run packaging after updating the runtime and declaring the offline sources. |
| Website and release link | Pass | A push to the separate public website repository automatically deployed to the same Vercel project and domain. Public HTML, CSS, and images matched the site source, and GitHub Releases still offered `v0.3.0`. The app-only cleanup passed Fedora CI and triggered zero Vercel deployments. | — |

## Improvement priorities

1. **High** — Add reproducible desktop tests for branch drag and keyboard
   integration, conflict recovery, hunk selection, and the sync state matrix.
   Keep the display-backed Fedora check for each app change.
2. **Medium** — Unify Hash search in the History list and details, and preserve
   the same filter during pagination.
3. **Medium** — Load the previous commit message when Amend is enabled and
   the editor is empty.
4. **Medium** — Update the Flatpak GNOME runtime and Rust SDK extension
   together, declare the offline Cargo sources, and exercise an actual
   install.
5. **Low** — Give the commit editor an explicit accessible name and avoid
   duplicate GTK text tag warnings when refreshing the diff.

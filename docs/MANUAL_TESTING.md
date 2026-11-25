# Manual UI test checklist

These steps mirror the automated end-to-end test at the service layer and
verify the actual GTK interface. Create the fixture repository first:

```bash
./scripts/make-demo-repo.sh /tmp/gittrellis-demo
```

Then start the application:

```bash
cargo run -- /tmp/gittrellis-demo
```

## 1. Open repository

- [ ] The repository appears in the sidebar with its name and path.
- [ ] The header shows the repository name and the current branch (`main`).
- [ ] The window title is `gittrellis-demo — GitTrellis`.

## 2. Changes view

- [ ] `README.md` appears under **Changes** and `notes.txt` under
      **Untracked Files**.
- [ ] Click `README.md`: the diff is shown with added lines highlighted and
      line numbers.
- [ ] Click `notes.txt` and then `README.md` again: only the last clicked file
      stays highlighted and its diff is shown.
- [ ] Ctrl-click a second file: both files stay selected.
- [ ] Right-click a change row: the same actions menu opens with a short
      description under each entry.
- [ ] Click the `+` button on the `README.md` row: the file moves to
      **Staged Changes** and the commit button becomes sensitive.
- [ ] Discard a change and delete an untracked file: each confirmation has
      only Cancel and the destructive action, and the window does not freeze
      after confirming.
- [ ] Stage `notes.txt` as well.
- [ ] Right-click style menu (⋮) → **Open File** opens the file in the default
      editor; **Open Containing Folder** opens the file manager.
- [ ] Add to `.gitignore` on an untracked file removes it and updates
      `.gitignore` (verify in History).

## 3. Hunk staging

- [ ] Create two well-separated edits in a file, refresh.
- [ ] Click the `@@` hunk header of one hunk; **Stage Hunk** becomes enabled.
- [ ] Stage the hunk; only that change moves to the index (check the diff of
      the staged file).
- [ ] Select the staged hunk and use **Unstage Hunk**.

## 4. Commit

- [ ] Type a commit message; Ctrl+Enter commits.
- [ ] The message box clears, the file lists empty, and a toast confirms.
- [ ] Toggle **Amend**, enter a revised commit message and submit. Verify that
      HEAD is replaced and the number of commits stays the same.

## 5. History

- [ ] Switch to **History**: commits are listed newest first with the graph
      lanes, ref badges and relative times.
- [ ] Click a commit: details, changed files and the file diff appear.
- [ ] Right-click a commit: the same actions menu as the three-dot button
      opens, with descriptions, and the clicked commit becomes selected.
- [ ] Copy SHA copies the full hash.
- [ ] **Create branch here…** creates and switches to a branch.
- [ ] **Cherry-pick** on a commit from another branch adds it to the current
      branch.
- [ ] Search commits by message, author and hash returns filtered results.

## 6. Branch menu and History integration

- [ ] The standalone Branches tab is gone. Click the branch name or press
      Ctrl+3 to open the header menu. A saved custom Branches shortcut opens
      the same menu.
- [ ] Filter local and remote branches by name. Verify the current branch,
      upstream, ahead/behind counts, and a missing-upstream indicator.
- [ ] **New Branch…** offers an optional start point and switch checkbox.
      Create a branch with and without switching, then check it out from the
      list. A remote-tracking branch can be checked out as a local branch.
- [ ] Per-branch menus offer checkout, rename, push, upstream, copy, compare,
      merge, rebase and delete where applicable. Verify destructive
      confirmations by cancelling once and checking that the branch still exists.
- [ ] Drag a local branch badge in History onto another local badge. Verify the
      highlight and the source and target names. Cancel and check both refs
      with `git rev-parse` to confirm that their tips are unchanged.
- [ ] On separate disposable fixtures, confirm a merge and a rebase. Verify
      the checked-out branch, both ref tips, and the graph afterward. Repeat
      with conflicting commits and resolve or abort from Conflicts.
- [ ] Drag between two distinct branches on the same commit, including names
      with slashes. Try self-drops, remote and tag badges, external text and
      another repository. Scroll enough to recycle rows before repeating.
- [ ] Activate a local badge or choose **Integrate Branches…** from History
      using the keyboard. Select a branch outside the visible graph. While a
      confirmation is open, change a selected tip in a terminal and check that
      the operation reports the stale selection. Test a dirty checkout and
      confirm that the worktree retains its edits.
- [ ] **Compare Branches…** starts with distinct base and target branches, and
      distinguishes local and remote refs with the same short name. The result
      shows the expected file changes and no commit actions. Repeat with
      **Compare Commits…** by choosing a base and then a target.

## 7. Conflict resolution

- [ ] Create a conflicting merge (edit the same file on two branches).
- [ ] After merging, the **Conflicts** tab appears with the file listed and a
      banner offering *Continue* / *Abort*.
- [ ] **Accept Current**, **Accept Incoming** and **Accept Both** rewrite the
      file; the markers disappear.
- [ ] **Mark Resolved** stages the file; *Continue* finishes the merge.

## 8. Remotes

- [ ] Add a remote pointing at a local bare repository.
- [ ] **Fetch** shows progress and can be cancelled.
- [ ] **Push** with *Set upstream* publishes the branch; the branch shows a
      tracking badge with ahead/behind counts.
- [ ] **Pull** merges or rebases depending on the chosen options.

## 9. Stashes and tags

- [ ] Stash changes including untracked files; the working tree becomes clean.
- [ ] Apply, pop, drop and “branch from stash” work as described.
- [ ] Create a lightweight and an annotated tag; push and delete them.

## 10. Keyboard, theme and windowing

- [ ] The terminal button opens a shell in the repository root; ``Ctrl+` ``
      does the same. Opening Git output collapses the terminal and vice versa.
- [ ] Typing `pwd` in the terminal prints the repository path.
- [ ] Ctrl+S stages the selected file, Ctrl+Shift+S stages everything,
      Ctrl+R refreshes, Ctrl+P pushes or opens push options, Ctrl+F fetches,
      Ctrl+Shift+F searches, and F1 shows the shortcut reference.
- [ ] Switching the system between dark and light mode restyles the app.
- [ ] Opening a second repository adds a sidebar entry; switching between
      repositories preserves each one's state.
- [ ] `gittrellis /path/to/repo` from a terminal opens that repository.

## 11. Authentication prompts

- [ ] With a passphrase-protected key that is not in the agent, fetching from
      an SSH remote shows the **Authentication required** dialog with the ssh
      prompt text and a passphrase field.
- [ ] Entering the passphrase continues the fetch; cancelling fails cleanly
      with an authentication error and no hang.
- [ ] The dialog suggests `ssh-add`; after running `ssh-add <key>`, the same
      operation no longer prompts.
- [ ] An HTTPS remote that needs a username/password opens the same dialog for
      each prompt.

## 12. One-click sync

- [ ] After committing with an upstream configured, the header shows
      `Push N` highlighted; one click pushes and the button returns to
      `Up to date`.
- [ ] When the remote has new commits, the header shows `Pull N`; one click
      pulls (conflicts switch to the Conflicts tab).
- [ ] With both ahead and behind, the header shows `Sync ↑N ↓M`; one click
      pulls and then pushes.
- [ ] A branch without upstream shows `Publish Branch`; clicking opens the
      push dialog with *Set upstream* enabled.
- [ ] Ctrl+P and Ctrl+Shift+P perform the same quick actions; `Pull with
      options…` in the branch menu still opens the full dialog.

## 13. Error handling

- [ ] Opening a plain directory shows “This directory is not a Git
      repository” with a recovery suggestion.
- [ ] Pulling with no upstream explains how to set one.
- [ ] Pushing to an unreachable remote shows the Git error with copyable
      technical details.
- [ ] The **Git output** expander at the bottom shows the raw command output
      for the last operations.

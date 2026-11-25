---
name: gittrellis-release
description: Prepare and publish a GitTrellis alpha on RaviTex/GitTrellis. Use for requests to release a version, push a release tag, or finish a release whose assets are missing.
---

# Release GitTrellis

Prepare the release documentation and publish the Fedora 45 x86_64 RPM,
vendored source tarball, and SHA-256 checksums. The release workflow is
[.github/workflows/fedora.yml](../../../.github/workflows/fedora.yml).
Use read-only checks for status or diagnosis requests. Run the publishing
steps when the user requests a release.

## Prepare the release

1. Confirm the repository is `RaviTex/GitTrellis` and inspect the latest
   `origin/main`, the current Cargo version, and existing GitHub releases and
   remote tags. Use the requested `X.Y.Z` or an unpublished version already
   prepared in the source. Ask for the version if neither identifies this
   release.
2. Check any existing `vX.Y.Z` tag against the intended commit. For an
   annotated tag, use its peeled commit from `git ls-remote --tags`. Reuse a
   tag that points at the correct release commit. If the tag points elsewhere,
   show its commit and package version. Get explicit authorization before
   replacing a pushed tag.
3. Follow the [packaging instructions](../../../CONTRIBUTING.md#packaging).
   Keep the package version aligned in `Cargo.toml`, the root package entry
   in `Cargo.lock`, the Fedora spec, and the first AppStream release entry.
   Move the relevant `Unreleased` entries into a dated `X.Y.Z` changelog
   section and restore an empty `Unreleased` heading. Use the app diff
   since the previous release to fill any gaps in the notes. Update user-facing
   documentation when commands or installation steps changed.
4. Review the release diff and run `cargo test --locked` in the project's
   Fedora build environment. When `rpmspec` is available, also run
   `python3 scripts/release-info.py --tag vX.Y.Z` from the repository root with
   `GITHUB_OUTPUT` set to a temporary file. The script checks package metadata
   and extracts the changelog notes used by CI. Commit the release changes,
   keeping unrelated working-tree edits out of the release commit.

## Tag, publish, and verify

- Check that the release commit includes the tag-triggered workflow. Create
  an annotated `vX.Y.Z` tag on that tested commit. When pushing
  a new release commit and a new tag together, use
  `git push --atomic origin HEAD:refs/heads/main refs/tags/vX.Y.Z`.
  If the commit is already on `origin/main`, push just the tag. When a matching
  remote tag already exists, use its release or its workflow run.
- GitHub Actions tests the tagged commit, builds the assets, and publishes a
  prerelease. Find the Fedora run by commit and tag with `gh run list` and
  follow it through completion. A version bump and tag can both queue a run
  for the same commit, so check the release before retrying.
- Verify that the GitHub prerelease is public, its tag points at the tested
  commit, and it has the Fedora 45 x86_64 RPM, vendored source tarball, and
  `SHA256SUMS`. Download the assets to a temporary directory and run
  `sha256sum --check SHA256SUMS` there. Return the release and workflow links.
- If publication fails, inspect the failed job and retry after correcting the
  cause. For a version-bump recovery, use the manual run described in
  [CONTRIBUTING.md](../../../CONTRIBUTING.md#packaging) when the release commit
  is still the tip of `main`. If a tag predates the workflow or its metadata
  does not match, report the exact blocker and the tag's current commit.

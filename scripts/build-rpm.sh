#!/usr/bin/env bash
# Build the RPM for the current version using rpmbuild and the vendored
# source tarball. Run from the repository root.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

version="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
topdir="$PWD/dist/rpmbuild"

echo ">> creating source tarball"
./scripts/make-source-tarball.sh "$PWD/dist"

echo ">> preparing rpmbuild tree"
mkdir -p "$topdir"/{SPECS,SOURCES,BUILD,RPMS,SRPMS}
cp "dist/stratum-git-${version}.tar.gz" "$topdir/SOURCES/"
cp packaging/fedora/stratum-git.spec "$topdir/SPECS/"

echo ">> building RPM"
# shellcheck disable=SC2086
rpmbuild -bb ${RPMBUILD_ARGS:-} \
    --define "_topdir $topdir" \
    "$topdir/SPECS/stratum-git.spec"

echo ">> done. Packages:"
find "$topdir/RPMS" -name '*.rpm' -print

#!/usr/bin/env bash
# Build a source tarball containing the vendored Rust dependencies.
# The resulting tarball is what the RPM spec consumes, so builds never need
# network access.
set -euo pipefail

version="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
name="stratum-git-${version}"
outdir="${1:-dist}"
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

echo ">> vendoring dependencies for ${name}"
cargo vendor --quiet "${workdir}/vendor" > "${workdir}/vendor-config.toml"

echo ">> assembling source tree"
mkdir -p "${workdir}/${name}"
for entry in Cargo.toml Cargo.lock src tests data packaging flatpak docs scripts \
             README.md CONTRIBUTING.md CHANGELOG.md LICENSE; do
    if [ -e "$entry" ]; then
        cp -r "$entry" "${workdir}/${name}/"
    fi
done

mkdir -p "${workdir}/${name}/.cargo"
{
    echo "[source.crates-io]"
    echo 'replace-with = "vendored-sources"'
    echo
    echo '[source."vendored-sources"]'
    echo 'directory = "vendor"'
} > "${workdir}/${name}/.cargo/config.toml"

mv "${workdir}/vendor" "${workdir}/${name}/vendor"

mkdir -p "$outdir"
tar -C "$workdir" -czf "${outdir}/${name}.tar.gz" "${name}"
echo ">> wrote ${outdir}/${name}.tar.gz"

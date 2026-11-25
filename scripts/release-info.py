#!/usr/bin/env python3
"""Gate alpha publication on an increase in the pushed Cargo package version."""

import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET


def package_version(manifest):
    version = tomllib.loads(manifest)["package"]["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise SystemExit(f"Use a numeric X.Y.Z package version for GitHub releases: {version}")
    return version


def version_key(version):
    return tuple(map(int, version.split(".")))


def release_notes(version):
    changelog = Path("CHANGELOG.md").read_text(encoding="utf-8")
    heading = rf"^## \[?{re.escape(version)}\]?(?=\s|$)[^\n]*\n"
    section = re.search(heading + r"(.*?)(?=^## |\Z)", changelog, re.MULTILINE | re.DOTALL)
    if section is None or not section[1].strip():
        raise SystemExit(f"Add release notes for {version} to CHANGELOG.md")
    return section[1].strip() + "\n"


def main(before):
    current = package_version(Path("Cargo.toml").read_text(encoding="utf-8"))
    previous_manifest = subprocess.check_output(
        ["git", "show", f"{before}:Cargo.toml"], text=True
    )
    if version_key(current) <= version_key(package_version(previous_manifest)):
        print(f"Cargo version did not increase to {current}. Skipping publication.")
        return

    rpm_version = subprocess.check_output(
        ["rpmspec", "-q", "--srpm", "--qf", "%{VERSION}", "packaging/fedora/gittrellis.spec"],
        text=True,
    )
    appstream = ET.parse("data/io.github.gittrellis.GitTrellis.metainfo.xml")
    latest = appstream.find("./releases/release")
    appstream_version = latest.get("version") if latest is not None else None
    if rpm_version != current or appstream_version != current:
        raise SystemExit(
            f"Match Cargo version {current} in the RPM spec ({rpm_version}) "
            f"and first AppStream release ({appstream_version})"
        )

    output = Path("dist/release")
    output.mkdir(parents=True, exist_ok=True)
    (output / "notes.md").write_text(release_notes(current), encoding="utf-8")
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as outputs:
        outputs.write(f"version={current}\n")


if __name__ == "__main__":
    main(sys.argv[1])

#!/usr/bin/python3
"""Build the Debian source package that gets uploaded to a PPA.

A Launchpad builder has no network, so the Rust crates must travel inside the
source package. This script does three things in order:

1. vendors the daemon's crates (``scripts/vendor-rust.py``);
2. packs the upstream tree into ``horsepower_1.0.0.orig.tar.gz``;
3. runs ``dpkg-source -b`` to produce the ``.dsc`` and ``.debian.tar.xz``.

    python3 scripts/make-source.py                # into dist/
    python3 scripts/make-source.py --sign KEYID   # sign the .dsc with a key

The upload itself is ``dput``; see docs/PUBLISHING.md.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

#: Paths (relative to the repository root) that never enter the source package.
EXCLUDED_PATHS = {
    ".git",
    ".github",
    "dist",
    "debian/horsepower",
    "rust/horsepowerd/target",
    ".mypy_cache",
    ".ruff_cache",
    ".pytest_cache",
}

#: Basenames that never enter the source package.
EXCLUDED_NAMES = {"__pycache__", "preview.png", ".gitignore"}

EXCLUDED_SUFFIXES = (".pyc", ".deb", ".dsc", ".buildinfo", ".changes")


def read_field(name: str) -> str:
    """Read a field from debian/changelog."""
    output = subprocess.run(
        ["dpkg-parsechangelog", "-l", str(ROOT / "debian/changelog"), "-S", name],
        check=True,
        capture_output=True,
        text=True,
    )
    return output.stdout.strip()


def _ignore(directory: str, names: list[str]) -> set[str]:
    """Filter entries by their path relative to the repository root."""
    skipped: set[str] = set()
    relative_dir = Path(directory).resolve().relative_to(ROOT)
    for name in names:
        relative = (relative_dir / name).as_posix()
        if (
            relative in EXCLUDED_PATHS
            or name in EXCLUDED_NAMES
            or name.endswith(EXCLUDED_SUFFIXES)
        ):
            skipped.add(name)
    return skipped


def vendor() -> None:
    vendor_dir = ROOT / "rust" / "horsepowerd" / "vendor"
    if vendor_dir.is_dir():
        print(f"Using existing {vendor_dir.relative_to(ROOT)}")
        return
    print("Vendoring crates (first run only, ~120 MB) …")
    subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "vendor-rust.py")], check=True
    )


def stage_source_tree(source: str, version: str, destination: Path) -> Path:
    """Copy the tree, with debian/, into ``<source>-<upstream-version>/``."""
    upstream = version.split("-")[0]
    workdir = destination / f"{source}-{upstream}"
    if workdir.exists():
        shutil.rmtree(workdir)
    shutil.copytree(ROOT, workdir, ignore=_ignore, symlinks=True)
    return workdir


def build_orig(workdir: Path, source: str, version: str, destination: Path) -> Path:
    """Tar the upstream tree, leaving ``debian/`` out of the orig tarball."""
    upstream = version.split("-")[0]
    tarball = destination / f"{source}_{upstream}.orig.tar.gz"
    subprocess.run(
        [
            "tar",
            "--create",
            "--gzip",
            "--file",
            str(tarball),
            "--directory",
            str(destination),
            "--exclude",
            f"{workdir.name}/debian",
            workdir.name,
        ],
        check=True,
    )
    size = tarball.stat().st_size / 1_048_576
    print(f"{tarball.name}: {size:.1f} MB")
    return tarball


def build_source(workdir: Path, destination: Path, sign: str | None) -> int:
    # `dpkg-source -b` finds the matching `../<source>_<version>.orig.tar.gz`
    # next to the work directory. Signing is `-k<keyid>`, not an env var.
    command = ["dpkg-source", "-b", str(workdir)]
    if sign:
        command.append(f"-k{sign}")
    print("Running:", " ".join(command))
    return subprocess.run(command, cwd=destination).returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output", type=Path, default=ROOT / "dist", help="where to write files"
    )
    parser.add_argument(
        "--sign",
        metavar="KEYID",
        help="GPG key id to sign the .dsc with (required for Launchpad)",
    )
    arguments = parser.parse_args()

    for tool in ("dpkg-source", "dpkg-parsechangelog", "tar"):
        if shutil.which(tool) is None:
            print(f"{tool} is required", file=sys.stderr)
            return 1

    destination = arguments.output.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    for stale in destination.glob("horsepower_*"):
        stale.unlink()
    for stale in destination.glob("horsepower-*"):
        if stale.is_dir():
            shutil.rmtree(stale)

    vendor()

    source = read_field("Source")
    version = read_field("Version")
    print(f"{source} {version}, series {read_field('Distribution')}")

    workdir = stage_source_tree(source, version, destination)
    build_orig(workdir, source, version, destination)
    return build_source(workdir, destination, arguments.sign)


if __name__ == "__main__":
    raise SystemExit(main())

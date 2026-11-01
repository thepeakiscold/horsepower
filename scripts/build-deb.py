#!/usr/bin/python3
"""Build the HorsePower Debian package without debhelper.

The script stages an installed tree, writes the control metadata, runs
dpkg-deb and leaves the .deb in ./dist.

    python3 scripts/build-deb.py            # -> dist/horsepower_1.0.0_all.deb
    python3 scripts/build-deb.py --keep     # keep the staging tree

It intentionally has no build dependencies beyond dpkg-deb and gzip, so the
package can be produced on a minimal machine. Systems with debhelper can also
build the same file list through ./debian/rules.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# The build helper is a sibling script, not an installed module.
from hpbuild import (  # noqa: E402
    PACKAGE,
    ROOT,
    VERSION,
    dpkg_architecture,
    stage_tree,
)


def main() -> int:
    parser = argparse.ArgumentParser(description="Build the HorsePower .deb")
    parser.add_argument("--keep", action="store_true", help="keep the staging tree")
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "dist",
        help="directory for the finished package (default: dist)",
    )
    arguments = parser.parse_args()

    if shutil.which("dpkg-deb") is None:
        print("dpkg-deb is required to build the package", file=sys.stderr)
        return 1

    output = arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    architecture = dpkg_architecture()
    temporary = Path(tempfile.mkdtemp(prefix="horsepower-build-"))
    stage = temporary / "stage"

    try:
        stage.mkdir()
        stage_tree(stage, architecture=architecture)
        package = output / f"{PACKAGE}_{VERSION}_{architecture}.deb"
        subprocess.run(
            [
                "dpkg-deb",
                "--root-owner-group",
                "-Zxz",
                "--build",
                str(stage),
                str(package),
            ],
            check=True,
        )
        print(f"Built {package}")
        if arguments.keep:
            kept = output / "stage"
            shutil.rmtree(kept, ignore_errors=True)
            shutil.copytree(stage, kept, symlinks=True)
            print(f"Staging tree kept at {kept}")
        return 0
    finally:
        shutil.rmtree(temporary, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())

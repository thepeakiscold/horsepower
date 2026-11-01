#!/usr/bin/python3
"""Install HorsePower's file tree into a destination directory.

Used by debian/rules; scripts/build-deb.py uses the same code path.

    python3 scripts/stage.py debian/horsepower
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from hpbuild import stage_tree  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument(
        "--clean", action="store_true", help="remove the destination first"
    )
    parser.add_argument(
        "--with-control",
        action="store_true",
        help="also write DEBIAN/ metadata (stand-alone builds only)",
    )
    arguments = parser.parse_args()

    destination = arguments.destination.resolve()
    if arguments.clean:
        shutil.rmtree(destination, ignore_errors=True)
    destination.mkdir(parents=True, exist_ok=True)
    stage_tree(destination, include_control=arguments.with_control)
    print(f"Staged into {destination}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

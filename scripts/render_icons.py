#!/usr/bin/python3
"""Render the scalable icon into the PNG sizes expected by icon themes."""

from __future__ import annotations

from pathlib import Path

import gi

gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "data" / "icons" / "scalable" / "apps" / "org.horsepower.meter.svg"
SIZES = (16, 24, 32, 48, 64, 128, 256, 512)


def main() -> int:
    for size in SIZES:
        target = (
            ROOT
            / "data"
            / "icons"
            / "hicolor"
            / f"{size}x{size}"
            / "apps"
            / "org.horsepower.meter.png"
        )
        target.parent.mkdir(parents=True, exist_ok=True)
        pixbuf = GdkPixbuf.Pixbuf.new_from_file_at_scale(str(SOURCE), size, size, True)
        pixbuf.savev(str(target), "png", [], [])
        print(f"{target.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

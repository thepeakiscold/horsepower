#!/usr/bin/python3
"""Create a synthetic RAPL tree that a development daemon can read.

    python3 scripts/fake-sensor.py /tmp/fake-sysfs [--watts 35]

Run it in a terminal and point the daemon at the directory:

    rust/horsepowerd/target/release/horsepowerd --bus session \
        --db /tmp/horsepower.db --sysfs-root /tmp/fake-sysfs

The counter is advanced every 100 ms from a slowly wandering load, which is
enough to exercise the gauge, the chart and the cost estimate.
"""

from __future__ import annotations

import argparse
import math
import shutil
import time
from pathlib import Path

MAX_RANGE_UJ = 65_532_610_987
TICK_SECONDS = 0.1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="sysfs stand-in directory")
    parser.add_argument("--watts", type=float, default=35.0, help="average load")
    parser.add_argument(
        "--package-only",
        action="store_true",
        help="expose package-0 instead of psys",
    )
    parser.add_argument("--duration", type=float, default=0.0, help="stop after N seconds")
    arguments = parser.parse_args()

    powercap = arguments.root / "powercap" / "intel-rapl:0"
    shutil.rmtree(arguments.root / "powercap", ignore_errors=True)
    powercap.mkdir(parents=True)
    (powercap / "name").write_text(
        "package-0" if arguments.package_only else "psys", encoding="ascii"
    )
    (powercap / "max_energy_range_uj").write_text(str(MAX_RANGE_UJ), encoding="ascii")
    energy_file = powercap / "energy_uj"

    energy = 1_000_000.0
    started = time.monotonic()
    last = started
    print(f"Fake sensor in {powercap} (~{arguments.watts:g} W). Ctrl-C to stop.")
    try:
        while True:
            now = time.monotonic()
            elapsed = now - last
            last = now
            t = now - started
            load = arguments.watts * (
                1.0
                + 0.45 * math.sin(t / 7.0)
                + 0.18 * math.sin(t / 1.3)
                + 0.06 * math.sin(t / 0.37)
            )
            energy = (energy + load * 1e6 * elapsed) % MAX_RANGE_UJ
            energy_file.write_text(str(int(energy)), encoding="ascii")
            if arguments.duration and t >= arguments.duration:
                break
            time.sleep(TICK_SECONDS)
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

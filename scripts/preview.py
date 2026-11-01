#!/usr/bin/python3
"""Render the HorsePower window to a PNG without needing a screenshot tool.

This is a development helper. It opens the window briefly, fills it with a
realistic sample so the layout can be reviewed, then renders it offscreen with
the Cairo GSK renderer.

Usage:
    python3 scripts/preview.py [output.png] [--light]
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gsk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import (  # noqa: E402
    Adw,
    Gdk,
    Gio,
    GLib,
    GObject,
    Graphene,
    Gsk,
    Gtk,
)

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "src"))

from horsepower.constants import APP_ID  # noqa: E402
from horsepower.settings import Settings  # noqa: E402
from horsepower.ui.window import PowerWindow  # noqa: E402

_NOW = int(time.time() // 60) * 60
SAMPLE_HISTORY = [
    (
        _NOW - (57 - index) * 60,
        18 + 22 * abs((index % 9) - 4) / 4 + index % 3,
        0.4,
    )
    for index in range(58)
]


SAMPLE_STATUS = {
    "live_watts": 96.4,
    "today_wh": 412.7,
    "total_wh": 38_914.2,
    "components": [
        ("cpu", "CPU", 52.1),
        ("gpu", "GPU", 23.6),
        ("ram", "Memory", 14.2),
        ("storage", "Storage", 4.5),
        ("other", "Rest of system", 2.0),
    ],
    "source": "Intel RAPL · platform + package + dram · GeForce RTX 4060 Ti",
    "scope": "system",
    "detail": "psys, package-0, dram; gpu NVIDIA GeForce RTX 4060 Ti",
    "available": True,
    "message": "",
    "sample_interval": 1.0,
    "retention_days": 90,
    "first_seen": 1_771_000_000,
    "version": "1.0.0",
    "updated_at": 1_790_000_000.0,
}


class PreviewClient(GObject.Object):
    """Stands in for the D-Bus client while rendering a preview."""

    __gsignals__ = {
        "status-updated": (GObject.SignalFlags.RUN_FIRST, None, (object,)),
        "history-updated": (GObject.SignalFlags.RUN_FIRST, None, (object,)),
        "availability-changed": (GObject.SignalFlags.RUN_FIRST, None, (bool,)),
        "history-invalidated": (GObject.SignalFlags.RUN_FIRST, None, ()),
    }

    def __init__(self, available: bool = True):
        super().__init__()
        self.available = available

    def get_status(self) -> None:
        pass

    def get_history(self, minutes: int = 60) -> None:
        pass


class PreviewApplication(Adw.Application):
    def __init__(self, output: Path, light: bool, offline: bool, no_sensor: bool):
        super().__init__(
            application_id=APP_ID + ".Preview",
            flags=Gio.ApplicationFlags.NON_UNIQUE,
        )
        self.output = output
        self.offline = offline
        self.no_sensor = no_sensor
        self.window: PowerWindow | None = None

    def do_startup(self) -> None:
        Adw.Application.do_startup(self)
        provider = Gtk.CssProvider()
        provider.load_from_path(str(ROOT / "src/horsepower/ui/style.css"))
        Gtk.StyleContext.add_provider_for_display(
            Gdk.Display.get_default(),
            provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def do_activate(self) -> None:
        client = PreviewClient(available=not self.offline)
        self.window = PowerWindow(self, client, Settings(tariff=0.31, currency="€"))
        self.window.set_default_size(560, 800)
        self.window.present()

        if self.offline:
            client.emit("availability-changed", False)
        elif self.no_sensor:
            status = dict(SAMPLE_STATUS)
            status["available"] = False
            status["source"] = "No sensor"
            status["message"] = (
                "The energy counter is root-only on this system. The HorsePower "
                "background service must run to read it."
            )
            client.emit("status-updated", status)
        else:
            client.emit("status-updated", dict(SAMPLE_STATUS))
            client.emit("history-updated", list(SAMPLE_HISTORY))
            self.window._gauge._displayed = 96.4  # noqa: SLF001 - preview only
            self.window._gauge._target = 96.4  # noqa: SLF001
        GLib.timeout_add(600, self._capture)

    def _capture(self) -> bool:
        window = self.window
        assert window is not None
        width, height = window.get_width(), window.get_height()
        paintable = Gtk.WidgetPaintable.new(window)
        snapshot = Gtk.Snapshot()
        paintable.snapshot(snapshot, width, height)
        node = snapshot.to_node()
        assert node is not None
        renderer = Gsk.CairoRenderer.new()
        renderer.realize_for_display(Gdk.Display.get_default())
        texture = renderer.render_texture(
            node, Graphene.Rect().init(0, 0, width, height)
        )
        texture.save_to_png(str(self.output))
        renderer.unrealize()
        print(f"Wrote {self.output} ({texture.get_width()}x{texture.get_height()})")
        window.close()
        self.quit()
        return GLib.SOURCE_REMOVE


def main() -> int:
    arguments = [arg for arg in sys.argv[1:] if not arg.startswith("--")]
    output = Path(arguments[0] if arguments else "preview.png").resolve()
    Adw.StyleManager.get_default().set_color_scheme(
        Adw.ColorScheme.FORCE_LIGHT
        if "--light" in sys.argv
        else Adw.ColorScheme.FORCE_DARK
    )
    app = PreviewApplication(
        output,
        light="--light" in sys.argv,
        offline="--offline" in sys.argv,
        no_sensor="--no-sensor" in sys.argv,
    )
    GLib.timeout_add(8000, app.quit)
    return app.run([sys.argv[0]])


if __name__ == "__main__":
    raise SystemExit(main())

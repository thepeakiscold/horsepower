"""Application entry point."""

from __future__ import annotations

import logging
import os
import sys
from pathlib import Path

import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, Gio, Gtk  # noqa: E402

from ..client import PowerClient  # noqa: E402
from ..constants import APP_ID, VERSION  # noqa: E402
from ..settings import Settings  # noqa: E402
from .window import PowerWindow  # noqa: E402

log = logging.getLogger(__name__)


class HorsePowerApplication(Adw.Application):
    def __init__(self):
        super().__init__(
            application_id=APP_ID,
            flags=Gio.ApplicationFlags.DEFAULT_FLAGS,
        )
        self._window: PowerWindow | None = None
        self._client: PowerClient | None = None
        self._settings = Settings.load()

    def do_startup(self) -> None:
        Adw.Application.do_startup(self)
        self._load_css()
        self._setup_actions()

    def _check_cairo(self) -> bool:
        """The gauge and chart paint with Cairo through PyGObject.

        Ubuntu ships the Cairo GObject glue in a separate package, so a
        missing python3-gi-cairo would otherwise show up as a flood of
        'foreign struct converter' errors while drawing.
        """
        try:
            gi.require_foreign("cairo")
        except ImportError:
            log.error("PyGObject was built without Cairo support")
            return False
        return True

    def _load_css(self) -> None:
        css = Path(__file__).with_name("style.css")
        if not css.exists():
            log.warning("Stylesheet not found at %s", css)
            return
        provider = Gtk.CssProvider()
        try:
            provider.load_from_path(str(css))
        except Exception:
            log.exception("Could not load the stylesheet")
            return
        display = Gdk.Display.get_default()
        if display is None:
            return
        Gtk.StyleContext.add_provider_for_display(
            display, provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
        )

        # In a source checkout the icon is not installed yet.  Adding the
        # local icon directory keeps development builds recognisable.
        local_icons = Path(__file__).resolve().parents[3] / "data" / "icons"
        if local_icons.is_dir() and "/usr/" not in str(local_icons):
            Gtk.IconTheme.get_for_display(display).add_search_path(
                str(local_icons)
            )

    def _setup_actions(self) -> None:
        preferences = Gio.SimpleAction.new("preferences", None)
        preferences.connect("activate", self._show_preferences)
        self.add_action(preferences)

        about = Gio.SimpleAction.new("about", None)
        about.connect("activate", self._show_about)
        self.add_action(about)

        quit_action = Gio.SimpleAction.new("quit", None)
        quit_action.connect("activate", lambda *_: self.quit())
        self.add_action(quit_action)

        self.set_accels_for_action("app.preferences", ["<Control>comma"])
        self.set_accels_for_action("app.about", ["F1"])
        self.set_accels_for_action("app.quit", ["<Control>q"])

    def do_activate(self) -> None:
        if not self._check_cairo():
            self._report_missing_cairo()
            return
        if self._window is None:
            bus_type = (
                Gio.BusType.SESSION
                if os.environ.get("HORSEPOWER_BUS") == "session"
                else Gio.BusType.SYSTEM
            )
            self._client = PowerClient(bus_type)
            self._window = PowerWindow(self, self._client, self._settings)
        self._window.present()

    def _report_missing_cairo(self) -> None:
        dialog = Adw.MessageDialog(
            transient_for=self._window,
            heading="Cairo support is missing",
            body=(
                "HorsePower draws its gauge with Cairo, but the PyGObject "
                "Cairo bindings are not installed. Install them with:\n\n"
                "    sudo apt install python3-gi-cairo\n\n"
                "then start HorsePower again."
            ),
        )
        dialog.add_response("close", "Close")
        dialog.connect("response", lambda *_: self.quit())
        dialog.present()

    def _show_preferences(self, *_args) -> None:
        if self._window is not None:
            self._window.show_preferences()

    def _show_about(self, *_args) -> None:
        if self._window is not None:
            self._window.show_about()


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv if argv is None else argv)
    if "--version" in argv:
        print(VERSION)
        return 0
    logging.basicConfig(level=logging.WARNING)
    _hook_logging()
    app = HorsePowerApplication()
    return app.run(argv)


def _hook_logging() -> None:
    """Send application warnings to the developer console, not stdout."""
    if os.environ.get("HORSEPOWER_DEBUG"):
        logging.getLogger().setLevel(logging.DEBUG)


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())

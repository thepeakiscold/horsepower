"""Small helpers for matching the system font in custom drawing."""

from __future__ import annotations

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Pango", "1.0")
from gi.repository import Gtk, Pango  # noqa: E402


def font_family() -> str:
    settings = Gtk.Settings.get_default()
    name = settings.get_property("gtk-font-name") if settings else None
    if name:
        family = Pango.FontDescription.from_string(name).get_family()
        if family:
            return family
    return "Sans"

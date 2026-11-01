"""A restrained one-hour history chart."""

from __future__ import annotations

import time

import cairo
import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Pango", "1.0")
gi.require_version("PangoCairo", "1.0")
from gi.repository import Gtk, Pango, PangoCairo  # noqa: E402

from .fonts import font_family  # noqa: E402
from .palette import Palette  # noqa: E402

_WINDOW_SECONDS = 3600
_GAP_SECONDS = 150


class HistoryChart(Gtk.DrawingArea):
    """Draws the last hour as an area chart with honest gaps."""

    def __init__(self, palette: Palette):
        super().__init__()
        self.set_content_height(104)
        self.set_draw_func(self._draw)
        self._palette = palette
        self._points: list[tuple[int, float, float]] = []
        self._live: float | None = None
        self._maximum = 100.0
        self._placeholder = "COLLECTING THE FIRST MINUTE"

    def set_placeholder(self, text: str) -> None:
        if text != self._placeholder:
            self._placeholder = text
            if not self._points:
                self.queue_draw()

    def set_palette(self, palette: Palette) -> None:
        self._palette = palette
        self.queue_draw()

    def set_maximum(self, maximum: float) -> None:
        maximum = max(1.0, float(maximum))
        if abs(maximum - self._maximum) > 0.01:
            self._maximum = maximum
            self.queue_draw()

    def set_data(self, points: list[tuple[int, float, float]]) -> None:
        self._points = sorted(points, key=lambda point: point[0])
        self.queue_draw()

    def set_live(self, watts: float) -> None:
        self._live = max(0.0, float(watts))
        self.queue_draw()

    def _draw(self, _area: Gtk.DrawingArea, cr, width: int, height: int) -> None:
        palette = self._palette
        left = 1.0
        right = width - 1.0
        top = 5.0
        bottom = height - 2.0
        plot_width = max(1.0, right - left)
        plot_height = max(1.0, bottom - top)

        # Grid ------------------------------------------------------------
        for fraction in (0.25, 0.5, 0.75):
            y = top + plot_height * (1.0 - fraction)
            cr.set_line_width(1.0)
            cr.set_source_rgba(*palette.rgba(palette.grid, 0.85))
            cr.move_to(left, y + 0.5)
            cr.line_to(right, y + 0.5)
            cr.stroke()

        cr.set_source_rgba(*palette.rgba(palette.faint, 0.9))
        cr.set_line_width(1.0)
        cr.move_to(left, bottom + 0.5)
        cr.line_to(right, bottom + 0.5)
        cr.stroke()

        now = time.time()
        start = now - _WINDOW_SECONDS
        visible = [
            point for point in self._points if point[0] >= start - _GAP_SECONDS
        ]
        if (
            self._live is not None
            and visible
            and now - visible[-1][0] < _GAP_SECONDS
        ):
            visible.append((int(now), self._live, 0.0))

        if not visible:
            self._draw_empty(cr, width, height, palette)
            return

        segments: list[list[tuple[float, float]]] = []
        current: list[tuple[float, float]] = []
        previous_bucket: int | None = None
        for bucket, watts, _wh in visible:
            if (
                previous_bucket is not None
                and bucket - previous_bucket > _GAP_SECONDS
            ):
                if current:
                    segments.append(current)
                current = []
            x = left + (bucket - start) / _WINDOW_SECONDS * plot_width
            x = min(max(x, left), right)
            fraction = min(max(watts / self._maximum, 0.0), 1.0)
            y = top + plot_height * (1.0 - fraction)
            current.append((x, y))
            previous_bucket = bucket
        if current:
            segments.append(current)

        for segment in segments:
            if len(segment) < 2:
                x, y = segment[0]
                cr.set_source_rgb(*palette.rgb(palette.accent))
                cr.arc(x, y, 2.5, 0, 6.28318)
                cr.fill()
                continue

            gradient = cairo.LinearGradient(0, top, 0, bottom)
            gradient.add_color_stop_rgba(
                0.0, *palette.rgba(palette.accent, 0.30)
            )
            gradient.add_color_stop_rgba(
                1.0, *palette.rgba(palette.accent, 0.01)
            )
            cr.move_to(segment[0][0], bottom)
            for x, y in segment:
                cr.line_to(x, y)
            cr.line_to(segment[-1][0], bottom)
            cr.close_path()
            cr.set_source(gradient)
            cr.fill()

            cr.set_line_width(1.8)
            cr.set_line_join(cairo.LINE_JOIN_ROUND)
            cr.set_line_cap(cairo.LINE_CAP_ROUND)
            cr.set_source_rgb(*palette.rgb(palette.accent))
            cr.move_to(*segment[0])
            for x, y in segment[1:]:
                cr.line_to(x, y)
            cr.stroke()

        # Emphasise the most recent reading only when it is recent.
        if segments and now - visible[-1][0] < _GAP_SECONDS:
            x, y = segments[-1][-1]
            cr.set_source_rgba(*palette.rgba(palette.accent, 0.18))
            cr.arc(x, y, 6.0, 0, 6.28318)
            cr.fill()
            cr.set_source_rgb(*palette.rgb(palette.accent))
            cr.arc(x, y, 2.6, 0, 6.28318)
            cr.fill()

    def _draw_empty(
        self, cr, width: int, height: int, palette: Palette
    ) -> None:
        layout = PangoCairo.create_layout(cr)
        description = Pango.FontDescription()
        description.set_family(font_family())
        description.set_absolute_size(9.0 * Pango.SCALE)
        description.set_weight(Pango.Weight.SEMIBOLD)
        layout.set_font_description(description)
        attributes = Pango.AttrList()
        attributes.insert(Pango.attr_letter_spacing_new(int(1.5 * Pango.SCALE)))
        layout.set_attributes(attributes)
        layout.set_text(self._placeholder, -1)
        text_width, text_height = layout.get_pixel_size()
        cr.set_source_rgba(*palette.rgba(palette.muted, 0.85))
        cr.move_to(
            (width - text_width) / 2.0,
            (height - text_height) / 2.0,
        )
        PangoCairo.show_layout(cr, layout)

"""Per-component power breakdown, drawn as a compact bar list."""

from __future__ import annotations

from dataclasses import dataclass

import cairo
import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Pango", "1.0")
gi.require_version("PangoCairo", "1.0")
from gi.repository import Gtk, Pango, PangoCairo  # noqa: E402

from .fonts import font_family  # noqa: E402
from .palette import Palette  # noqa: E402

ROW_HEIGHT = 23.0
LABEL_WIDTH = 104.0
VALUE_WIDTH = 58.0
BAR_HEIGHT = 3.0
TOP_PAD = 2.0


@dataclass(frozen=True)
class Row:
    key: str
    label: str
    watts: float


class Breakdown(Gtk.DrawingArea):
    """Rows of ``label ─── bar ─── watts``.

    Bars are scaled to the largest component rather than to the total, so a
    small GPU draw stays visible next to a large CPU draw. The numbers, not
    the bar lengths, carry the exact values.
    """

    def __init__(self, palette: Palette):
        super().__init__()
        self.set_draw_func(self._draw)
        self._palette = palette
        self._family = font_family()
        self._rows: list[Row] = []
        self._caption = ""
        self.set_content_height(int(TOP_PAD))

    def set_palette(self, palette: Palette) -> None:
        self._palette = palette
        self.queue_draw()

    def set_rows(self, rows: list[Row], caption: str) -> None:
        self._rows = rows
        self._caption = caption
        self.set_content_height(
            int(TOP_PAD + max(1, len(rows)) * ROW_HEIGHT) if rows else 0
        )
        self.queue_draw()

    def _layout(self, text: str, size: float, weight: Pango.Weight, spacing: float = 0.0):
        layout = PangoCairo.create_layout(self._cr)
        description = Pango.FontDescription()
        description.set_family(self._family)
        description.set_absolute_size(size * Pango.SCALE)
        description.set_weight(weight)
        layout.set_font_description(description)
        attributes = Pango.AttrList()
        if spacing:
            attributes.insert(
                Pango.attr_letter_spacing_new(int(spacing * Pango.SCALE))
            )
        attributes.insert(Pango.attr_font_features_new("tnum"))
        layout.set_attributes(attributes)
        layout.set_text(text, -1)
        return layout

    def _draw(self, _area: Gtk.DrawingArea, cr, width: int, height: int) -> None:
        self._cr = cr
        palette = self._palette
        if not self._rows or width < 120:
            return

        maximum = max((row.watts for row in self._rows), default=0.0)
        bar_left = LABEL_WIDTH
        bar_right = max(bar_left + 20.0, width - VALUE_WIDTH)
        bar_width = bar_right - bar_left
        cr.set_line_cap(cairo.LINE_CAP_ROUND)

        for index, row in enumerate(self._rows):
            top = TOP_PAD + index * ROW_HEIGHT
            center = top + ROW_HEIGHT / 2.0

            label = self._layout(
                row.label.upper(), 9.0, Pango.Weight.SEMIBOLD, spacing=1.2
            )
            # Never let a long machine-specific name run into its own bar.
            label.set_width(int((LABEL_WIDTH - 8.0) * Pango.SCALE))
            label.set_ellipsize(Pango.EllipsizeMode.END)
            _, label_height = label.get_pixel_size()
            cr.set_source_rgb(*palette.rgb(palette.muted))
            cr.move_to(0, center - label_height / 2.0)
            PangoCairo.show_layout(cr, label)

            cr.set_line_width(BAR_HEIGHT)
            cr.set_source_rgba(*palette.rgba(palette.faint, 0.85))
            cr.move_to(bar_left, center)
            cr.line_to(bar_right, center)
            cr.stroke()

            fraction = (row.watts / maximum) if maximum > 0.05 else 0.0
            fraction = min(max(fraction, 0.0), 1.0)
            if fraction > 0.001:
                cr.set_line_width(BAR_HEIGHT)
                cr.set_source_rgb(*palette.rgb(palette.accent))
                cr.move_to(bar_left, center)
                cr.line_to(bar_left + max(bar_width * fraction, 1.5), center)
                cr.stroke()

            value = self._layout(
                f"{row.watts:,.1f} W", 10.5, Pango.Weight.MEDIUM
            )
            value_width, value_height = value.get_pixel_size()
            cr.set_source_rgb(*palette.rgb(palette.ink))
            cr.move_to(width - value_width, center - value_height / 2.0)
            PangoCairo.show_layout(cr, value)

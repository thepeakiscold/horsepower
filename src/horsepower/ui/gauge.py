"""The circular power gauge, drawn with Cairo."""

from __future__ import annotations

import math

import cairo
import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Pango", "1.0")
gi.require_version("PangoCairo", "1.0")
from gi.repository import Gtk, Pango, PangoCairo  # noqa: E402

from .fonts import font_family  # noqa: E402
from .palette import Palette  # noqa: E402

_TWO_PI = math.pi * 2.0
_START = math.pi * 0.75
_SWEEP = math.pi * 1.5


class PowerGauge(Gtk.DrawingArea):
    """A quiet 270-degree gauge with a large tabular readout."""

    def __init__(self, palette: Palette):
        super().__init__()
        self.set_content_width(380)
        self.set_content_height(252)
        self.set_draw_func(self._draw)
        self._palette = palette
        self._family = font_family()
        self._maximum = 100.0
        self._target = 0.0
        self._displayed = 0.0
        self._available = False
        self._caption = "WAITING FOR SENSOR"
        self._animating = False
        self._last_frame: float | None = None

    def set_palette(self, palette: Palette) -> None:
        self._palette = palette
        self.queue_draw()

    def set_maximum(self, maximum: float) -> None:
        maximum = max(1.0, float(maximum))
        if abs(maximum - self._maximum) > 0.01:
            self._maximum = maximum
            self.queue_draw()

    def set_caption(self, caption: str) -> None:
        if caption != self._caption:
            self._caption = caption
            self.queue_draw()

    def set_available(self, available: bool) -> None:
        if available != self._available:
            self._available = available
            self.queue_draw()

    def set_value(self, watts: float) -> None:
        target = max(0.0, float(watts))
        if not self._available:
            self._target = 0.0
            self._displayed = 0.0
            return
        self._target = min(target, self._maximum * 1.001)
        if not self._animating:
            self._animating = True
            self._last_frame = None
            self.add_tick_callback(self._on_tick)

    def _on_tick(self, _widget: Gtk.Widget, clock, *_args) -> bool:
        now = clock.get_frame_time() / 1_000_000.0
        if self._last_frame is None:
            self._last_frame = now
        delta = min(max(now - self._last_frame, 0.0), 0.1)
        self._last_frame = now
        factor = 1.0 - math.exp(-delta / 0.14)
        difference = self._target - self._displayed
        if abs(difference) < 0.02:
            self._displayed = self._target
            self._animating = False
            self.queue_draw()
            return False
        self._displayed += difference * factor
        self.queue_draw()
        return True

    # -- drawing helpers ---------------------------------------------------

    def _layout(
        self,
        text: str,
        size: float,
        weight: Pango.Weight,
        color: tuple[float, float, float],
        spacing: float = 0.0,
        features: str | None = None,
    ):
        layout = PangoCairo.create_layout(self._cr)
        description = Pango.FontDescription()
        description.set_family(self._family)
        description.set_absolute_size(size * Pango.SCALE)
        description.set_weight(weight)
        layout.set_font_description(description)

        attributes = Pango.AttrList()
        if spacing:
            attributes.insert(Pango.attr_letter_spacing_new(int(spacing * Pango.SCALE)))
        if features:
            attributes.insert(Pango.attr_font_features_new(features))
        layout.set_attributes(attributes)
        layout.set_text(text, -1)
        return layout

    def _show(
        self,
        layout,
        x: float,
        y: float,
        color: tuple[float, float, float],
    ) -> None:
        self._cr.set_source_rgb(*color)
        self._cr.move_to(x, y)
        PangoCairo.show_layout(self._cr, layout)

    def _draw_centered(
        self,
        layout,
        x: float,
        center_y: float,
        color: tuple[float, float, float],
    ) -> tuple[int, int]:
        width, height = layout.get_pixel_size()
        self._show(layout, x - width / 2.0, center_y - height / 2.0, color)
        return width, height

    def _draw(self, _area: Gtk.DrawingArea, cr, width: int, height: int) -> None:
        self._cr = cr
        palette = self._palette
        cx = width / 2.0
        cy = height / 2.0 - 2.0
        radius = min(width, height) / 2.0 - 34.0
        if radius < 40:
            return

        # Tick ring ------------------------------------------------------
        for index in range(41):
            fraction = index / 40.0
            angle = _START + _SWEEP * fraction
            major = index % 5 == 0
            inner = radius + 13.0
            outer = radius + (21.0 if major else 17.0)
            cr.set_line_width(1.3 if major else 1.0)
            cr.set_source_rgba(
                *palette.rgba(palette.muted, 0.42 if major else 0.18)
            )
            cr.move_to(cx + math.cos(angle) * inner, cy + math.sin(angle) * inner)
            cr.line_to(cx + math.cos(angle) * outer, cy + math.sin(angle) * outer)
            cr.stroke()

        # Track ----------------------------------------------------------
        cr.set_line_cap(cairo.LINE_CAP_ROUND)
        cr.set_line_width(7.0)
        cr.set_source_rgba(*palette.rgba(palette.faint, 0.9))
        cr.arc(cx, cy, radius, _START, _START + _SWEEP)
        cr.stroke()

        fraction = 0.0
        if self._available:
            fraction = min(max(self._displayed / self._maximum, 0.0), 1.0)

        if fraction > 0.0005:
            end = _START + _SWEEP * fraction
            cr.set_line_width(16.0)
            cr.set_source_rgba(*palette.rgba(palette.accent, 0.10))
            cr.arc(cx, cy, radius, _START, end)
            cr.stroke()

            cr.set_line_width(7.0)
            cr.set_source_rgb(*palette.rgb(palette.accent))
            cr.arc(cx, cy, radius, _START, end)
            cr.stroke()

            tip_x = cx + math.cos(end) * radius
            tip_y = cy + math.sin(end) * radius
            cr.set_source_rgba(*palette.rgba(palette.accent, 0.16))
            cr.arc(tip_x, tip_y, 10.0, 0, _TWO_PI)
            cr.fill()
            cr.set_source_rgb(*palette.rgb(palette.accent))
            cr.arc(tip_x, tip_y, 4.0, 0, _TWO_PI)
            cr.fill()

        # Number ---------------------------------------------------------
        number_height = 0.0
        if self._available:
            shown = self._displayed
            number = f"{shown:.0f}" if shown >= 100 else f"{shown:.1f}"
            # Scale the readout with the ring so it never crowds the track.
            number_size = min(64.0, max(34.0, radius * 0.80))
            number_layout = self._layout(
                number,
                number_size,
                Pango.Weight.LIGHT,
                palette.rgb(palette.ink),
                features="tnum",
            )
            unit_layout = self._layout(
                "W", max(12.0, number_size * 0.24), Pango.Weight.MEDIUM,
                palette.rgb(palette.muted),
            )
            number_width, number_height = number_layout.get_pixel_size()
            unit_width, unit_height = unit_layout.get_pixel_size()
            gap = 7.0
            group_width = number_width + gap + unit_width
            left = cx - group_width / 2.0
            top = cy - number_height / 2.0
            self._show(number_layout, left, top, palette.rgb(palette.ink))
            self._show(
                unit_layout,
                left + number_width + gap,
                top + number_height - unit_height - 9.0,
                palette.rgb(palette.muted),
            )
        else:
            dash = self._layout(
                "—", 58.0, Pango.Weight.LIGHT, palette.rgb(palette.muted)
            )
            self._draw_centered(dash, cx, cy, palette.rgb(palette.muted))

        # Captions -------------------------------------------------------
        # Placed just under the number rather than at a fixed radius, so they
        # never collide with the readout when the window is short.
        caption_y = cy + (number_height / 2.0 + 13.0 if self._available else 44.0)
        caption_layout = self._layout(
            self._caption,
            9.5,
            Pango.Weight.SEMIBOLD,
            palette.rgb(palette.muted),
            spacing=1.7,
        )
        self._draw_centered(
            caption_layout, cx, caption_y, palette.rgb(palette.muted)
        )

        if self._available:
            range_layout = self._layout(
                f"0 – {self._maximum:.0f} W",
                9.5,
                Pango.Weight.MEDIUM,
                palette.rgb(palette.muted),
            )
            self._draw_centered(
                range_layout, cx, caption_y + 17.0, palette.rgb(palette.muted)
            )

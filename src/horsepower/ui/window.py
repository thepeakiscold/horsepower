"""Main window for HorsePower."""

from __future__ import annotations

import time

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Pango", "1.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gio, GLib, Gtk, Pango  # noqa: E402

from ..client import PowerClient  # noqa: E402
from ..constants import APP_ID, APP_NAME, COMPONENT_LABELS, VERSION  # noqa: E402
from ..settings import Settings  # noqa: E402
from .breakdown import Breakdown, Row  # noqa: E402
from .chart import HistoryChart  # noqa: E402
from .formatting import (  # noqa: E402
    format_date,
    format_energy,
    format_money,
    nice_maximum,
)
from .gauge import PowerGauge  # noqa: E402
from .palette import DARK, LIGHT, Palette  # noqa: E402
from .prefs import PreferencesDialog  # noqa: E402

#: Display order for the breakdown rows; anything unknown goes last.
ORDER = {
    key: index for index, (key, _label) in enumerate(COMPONENT_LABELS)
}


class PowerWindow(Adw.ApplicationWindow):
    def __init__(
        self,
        application: Adw.Application,
        client: PowerClient,
        settings: Settings,
    ):
        super().__init__(application=application, title=APP_NAME)
        self._client = client
        self._settings = settings
        self._status: dict = {}
        self._history: list[tuple[int, float, float]] = []
        self._palette: Palette = DARK
        self._scale_max = 50.0
        self._last_status_at = 0.0
        self._last_history_at = 0.0
        self._timer_id = 0
        self._prefs_dialog: PreferencesDialog | None = None

        self.set_default_size(560, 700)
        self.set_size_request(460, 620)
        self._palette = DARK if Adw.StyleManager.get_default().get_dark() else LIGHT

        self._build()
        self._apply_theme()

        style_manager = Adw.StyleManager.get_default()
        style_manager.connect("notify::dark", self._on_theme_changed)

        self._client.connect("status-updated", self._on_status)
        self._client.connect("history-updated", self._on_history)
        self._client.connect("availability-changed", self._on_availability)
        self._client.connect("history-invalidated", self._on_history_invalidated)

        self.connect("close-request", self._on_close)
        self._timer_id = GLib.timeout_add(1000, self._tick)

        if self._client.available:
            self._client.get_status()
            self._client.get_history(60)
        else:
            self._render_offline()

    # -- construction ------------------------------------------------------

    def _build(self) -> None:
        toolbar = Adw.ToolbarView()
        header = Adw.HeaderBar()
        header.set_show_title(False)

        brand = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=0)
        brand.add_css_class("hp-brand")
        mark = Gtk.Box()
        mark.add_css_class("hp-mark")
        mark.set_valign(Gtk.Align.CENTER)
        wordmark = Gtk.Label(label="HORSEPOWER")
        wordmark.add_css_class("hp-wordmark")
        wordmark.set_valign(Gtk.Align.CENTER)
        brand.append(mark)
        brand.append(wordmark)
        header.pack_start(brand)

        menu_button = Gtk.MenuButton()
        menu_button.add_css_class("hp-menu-btn")
        menu_button.set_icon_name("view-more-symbolic")
        menu_button.set_tooltip_text("Menu")
        menu = Gio.Menu()
        menu.append("Preferences", "app.preferences")
        menu.append("About HorsePower", "app.about")
        menu.append("Quit", "app.quit")
        menu_button.set_menu_model(menu)
        header.pack_end(menu_button)

        # A single dot carries service state. No word, no badge: the gauge
        # caption already says what is wrong when something is wrong.
        self._status_dot = Gtk.Box()
        self._status_dot.add_css_class("hp-status-dot")
        self._status_dot.add_css_class("hp-status-offline")
        self._status_dot.set_valign(Gtk.Align.CENTER)
        self._status_dot.set_tooltip_text("Connecting to the background service…")
        header.pack_end(self._status_dot)

        toolbar.add_top_bar(header)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=13)
        content.add_css_class("hp-content")

        self._gauge = PowerGauge(self._palette)
        self._gauge.set_halign(Gtk.Align.FILL)
        self._gauge.set_hexpand(True)
        content.append(self._gauge)

        stats = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=0)
        stats.set_halign(Gtk.Align.FILL)
        first = self._make_stat("TODAY", "—", "—")
        stats.append(first)
        stats.append(self._separator())
        stats.append(self._make_stat("TOTAL", "—", "since the first sample"))
        stats.append(self._separator())
        stats.append(self._make_stat("TODAY'S COST", "—", "—"))
        content.append(stats)

        breakdown_card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        breakdown_card.add_css_class("hp-card")

        breakdown_head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        breakdown_title = Gtk.Label(label="WHERE IT GOES")
        breakdown_title.add_css_class("hp-chart-title")
        breakdown_title.set_xalign(0.0)
        breakdown_title.set_hexpand(True)
        self._breakdown_caption = Gtk.Label(label="")
        self._breakdown_caption.add_css_class("hp-chart-peak")
        self._breakdown_caption.set_xalign(1.0)
        breakdown_head.append(breakdown_title)
        breakdown_head.append(self._breakdown_caption)
        breakdown_card.append(breakdown_head)

        self._breakdown = Breakdown(self._palette)
        self._breakdown.set_hexpand(True)
        breakdown_card.append(self._breakdown)
        self._breakdown_card = breakdown_card
        breakdown_card.set_visible(False)
        content.append(breakdown_card)

        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        card.add_css_class("hp-card")

        chart_head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        title = Gtk.Label(label="LAST HOUR")
        title.add_css_class("hp-chart-title")
        title.set_xalign(0.0)
        title.set_hexpand(True)
        self._peak_label = Gtk.Label(label="PEAK —")
        self._peak_label.add_css_class("hp-chart-peak")
        self._peak_label.set_xalign(1.0)
        chart_head.append(title)
        chart_head.append(self._peak_label)
        card.append(chart_head)

        self._chart = HistoryChart(self._palette)
        self._chart.set_hexpand(True)
        card.append(self._chart)

        axis = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        old = Gtk.Label(label="60 MIN AGO")
        old.add_css_class("hp-axis")
        old.set_xalign(0.0)
        old.set_hexpand(True)
        now = Gtk.Label(label="NOW")
        now.add_css_class("hp-axis")
        now.set_xalign(1.0)
        axis.append(old)
        axis.append(now)
        card.append(axis)
        content.append(card)

        footer = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        self._source_label = Gtk.Label(label="Connecting to the background service…")
        self._source_label.add_css_class("hp-footer")
        self._source_label.add_css_class("hp-footer-strong")
        self._source_label.set_xalign(0.0)
        self._source_label.set_hexpand(True)
        self._source_label.set_ellipsize(Pango.EllipsizeMode.END)
        self._log_label = Gtk.Label(label="LOCAL LOG")
        self._log_label.add_css_class("hp-footer")
        self._log_label.set_xalign(1.0)
        footer.append(self._source_label)
        footer.append(self._log_label)
        content.append(footer)

        # The dashboard has a natural height of roughly 770 px. On a 768 px
        # laptop work area that does not fit, so the content lives in a
        # scroller: tall screens never see it, short ones can still reach
        # every row instead of the window refusing to shrink.
        scroller = Gtk.ScrolledWindow()
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroller.set_has_frame(False)
        scroller.set_overlay_scrolling(True)
        scroller.set_propagate_natural_height(True)
        scroller.set_child(content)
        scroller.add_css_class("hp-scroller")

        toolbar.set_content(scroller)
        self.set_content(toolbar)

    def _make_stat(self, label: str, value: str, sub: str) -> Gtk.Box:
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        box.set_hexpand(True)
        box.set_margin_start(10)
        box.set_margin_end(10)

        heading = Gtk.Label(label=label)
        heading.add_css_class("hp-stat-label")
        heading.set_xalign(0.0)
        value_label = Gtk.Label(label=value)
        value_label.add_css_class("hp-stat-value")
        value_label.set_xalign(0.0)
        sub_label = Gtk.Label(label=sub)
        sub_label.add_css_class("hp-stat-sub")
        sub_label.set_xalign(0.0)
        sub_label.set_ellipsize(Pango.EllipsizeMode.END)

        box.append(heading)
        box.append(value_label)
        box.append(sub_label)

        if label == "TODAY":
            self._today_value, self._today_sub = value_label, sub_label
        elif label == "TOTAL":
            self._total_value, self._total_sub = value_label, sub_label
        else:
            self._cost_value, self._cost_sub = value_label, sub_label
        return box

    @staticmethod
    def _separator() -> Gtk.Separator:
        separator = Gtk.Separator(orientation=Gtk.Orientation.VERTICAL)
        separator.add_css_class("hp-separator")
        separator.set_margin_top(3)
        separator.set_margin_bottom(3)
        return separator

    # -- theme -------------------------------------------------------------

    def _on_theme_changed(self, *_args) -> None:
        self._apply_theme()

    def _apply_theme(self) -> None:
        dark = Adw.StyleManager.get_default().get_dark()
        self._palette = DARK if dark else LIGHT
        self.remove_css_class("hp-dark")
        self.remove_css_class("hp-light")
        self.add_css_class("hp-dark" if dark else "hp-light")
        if hasattr(self, "_gauge"):
            self._gauge.set_palette(self._palette)
            self._chart.set_palette(self._palette)

    # -- state rendering ---------------------------------------------------

    def _on_status(self, _client: PowerClient, status: dict) -> None:
        self._status = status
        self._last_status_at = time.monotonic()
        service = self._client.available
        sensor = bool(status.get("available")) and service
        message = str(status.get("message") or "")
        live = float(status.get("live_watts") or 0.0)

        self._gauge.set_available(sensor)
        if sensor:
            scope = str(status.get("scope") or "parts")
            caption = "SYSTEM POWER" if scope == "system" else "MEASURED PARTS"
            self._gauge.set_caption(caption)
            self._gauge.set_value(live)
            self._set_status("live")
            self._chart.set_placeholder("COLLECTING THE FIRST MINUTE")
        elif service:
            self._gauge.set_caption("SENSOR UNAVAILABLE")
            self._set_status("warn")
            self._chart.set_placeholder("NO READINGS YET")
        else:
            self._gauge.set_caption("SERVICE OFFLINE")
            self._set_status("offline")
            self._chart.set_placeholder("NO READINGS YET")

        tooltip = message or None
        self._gauge.set_tooltip_text(tooltip)
        self._status_dot.set_tooltip_text(
            message or ("Live power reading" if sensor else "Waiting for a sensor")
        )

        self._update_source(status, sensor, service)
        self._refresh_stats()
        self._update_breakdown(status, sensor)
        self._update_scale()
        self._chart.set_live(live if sensor else 0.0)

    def _on_history(self, _client: PowerClient, raw_points: list) -> None:
        points: list[tuple[int, float, float]] = []
        for item in raw_points:
            try:
                bucket, watts, wh = item
                points.append((int(bucket), float(watts), float(wh)))
            except (TypeError, ValueError):
                continue
        self._history = sorted(points, key=lambda point: point[0])
        self._last_history_at = time.monotonic()
        self._chart.set_data(self._history)
        self._update_scale()

    def _on_history_invalidated(self, _client: PowerClient) -> None:
        self._client.get_history(60)

    def _on_availability(self, _client: PowerClient, available: bool) -> None:
        if available:
            self._set_status("offline")
            self._source_label.set_text("Connecting to the background service…")
            self._client.get_status()
            self._client.get_history(60)
        else:
            self._render_offline()

    def _render_offline(self) -> None:
        self._gauge.set_available(False)
        self._gauge.set_caption("SERVICE OFFLINE")
        self._chart.set_live(0.0)
        self._chart.set_placeholder("NO READINGS YET")
        self._set_status("offline")
        self._breakdown.set_rows([], "")
        self._breakdown_caption.set_text("")
        self._source_label.set_text("Background service is not running")
        self._source_label.set_tooltip_text(
            "Start it with: sudo systemctl start horsepower"
        )
        self._log_label.set_text("LOCAL LOG")

    def _set_status(self, state: str) -> None:
        """Colour the header dot: live, warn (service up, no sensor) or off."""
        for css_class in ("hp-status-live", "hp-status-warn", "hp-status-offline"):
            self._status_dot.remove_css_class(css_class)
        self._status_dot.add_css_class(f"hp-status-{state}")

    def _update_breakdown(self, status: dict, sensor: bool) -> None:
        """Show every component the service could actually measure.

        Components the machine cannot read are omitted rather than drawn as
        zero, and the caption states whether the rows add up to the whole
        machine or only to the parts we can see.
        """
        if not sensor:
            self._breakdown.set_rows([], "")
            self._breakdown_caption.set_text("")
            self._breakdown_card.set_visible(False)
            return

        labels = dict(COMPONENT_LABELS)
        raw = status.get("components") or []
        rows: list[Row] = []
        for item in raw:
            try:
                key, label, watts = item
            except (TypeError, ValueError):
                continue
            rows.append(Row(str(key), str(label or labels.get(key, key)), float(watts)))
        rows.sort(key=lambda row: ORDER.get(row.key, len(ORDER)))

        scope = str(status.get("scope") or "parts")
        self._breakdown.set_rows(rows, "")
        self._breakdown_card.set_visible(bool(rows))
        if scope == "system":
            self._breakdown_caption.set_text("OF TOTAL")
        else:
            self._breakdown_caption.set_text("MEASURED PARTS ONLY")

    def _update_source(self, status: dict, sensor: bool, service: bool) -> None:
        if not service:
            self._source_label.set_text("Background service is not running")
            self._log_label.set_text("LOCAL LOG")
            return
        if sensor:
            self._source_label.set_text(str(status.get("source") or "Power sensor"))
            self._source_label.set_tooltip_text(str(status.get("detail") or ""))
            retention = int(status.get("retention_days") or 0)
            self._log_label.set_text(
                f"LOCAL LOG · {retention} D" if retention else "LOCAL LOG"
            )
        else:
            self._source_label.set_text("No measurable power source")
            self._source_label.set_tooltip_text(str(status.get("message") or ""))
            self._log_label.set_text("LOCAL LOG")

    def _refresh_stats(self) -> None:
        if not self._status:
            return
        today = float(self._status.get("today_wh") or 0.0)
        total = float(self._status.get("total_wh") or 0.0)
        first_seen = int(self._status.get("first_seen") or 0)
        currency = self._settings.currency
        cost = self._settings.price(today)

        self._today_value.set_text(format_energy(today))
        self._today_sub.set_text("since midnight")
        self._total_value.set_text(format_energy(total))
        self._total_sub.set_text(f"since {format_date(first_seen)}")
        self._cost_value.set_text(format_money(cost, currency))
        self._cost_sub.set_text(
            f"at {format_money(self._settings.tariff, currency)}/kWh"
        )

    def _update_scale(self) -> None:
        sensor = bool(self._status.get("available")) and self._client.available
        live = float(self._status.get("live_watts") or 0.0) if sensor else 0.0
        peak = max((point[1] for point in self._history), default=0.0)
        peak = max(peak, live)
        needed = nice_maximum(max(50.0, peak * 1.10))
        if needed > self._scale_max or needed < self._scale_max * 0.55:
            self._scale_max = needed
            self._gauge.set_maximum(needed)
            self._chart.set_maximum(needed)
        if peak > 0:
            self._peak_label.set_text(f"PEAK {peak:,.0f} W")
        else:
            self._peak_label.set_text("PEAK —")

    # -- housekeeping ------------------------------------------------------

    def _on_settings_changed(self, _settings: Settings) -> None:
        self._refresh_stats()

    def _tick(self) -> bool:
        now = time.monotonic()
        if self._client.available:
            if now - self._last_status_at > 3.0:
                self._client.get_status()
                self._last_status_at = now
            if now - self._last_history_at > 60.0:
                self._client.get_history(60)
                self._last_history_at = now
        return GLib.SOURCE_CONTINUE

    def _on_close(self, *_args) -> bool:
        if self._timer_id:
            GLib.source_remove(self._timer_id)
            self._timer_id = 0
        return False

    def show_preferences(self) -> None:
        retention = int(self._status.get("retention_days") or 90)
        dialog = PreferencesDialog(
            self._settings, retention, self._on_settings_changed
        )
        dialog.connect("closed", self._on_prefs_closed)
        self._prefs_dialog = dialog
        dialog.present(self)

    def _on_prefs_closed(self, *_args) -> None:
        self._prefs_dialog = None

    def show_about(self) -> None:
        dialog = Adw.AboutDialog(
            application_name=APP_NAME,
            application_icon=APP_ID,
            developer_name="HorsePower contributors",
            version=VERSION,
            comments=(
                "A small, honest power meter. It reads the energy counters "
                "built into Intel and AMD processors, keeps a local log, and "
                "never sends your data anywhere."
            ),
            license_type=Gtk.License.MIT_X11,
            copyright="© 2026 HorsePower contributors",
        )
        dialog.set_developers(["HorsePower contributors"])
        dialog.present(self)
